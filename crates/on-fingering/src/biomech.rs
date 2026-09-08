//! What a fingering costs the hand that has to play it.
//!
//! The rule sets in [`crate::rules`] describe awkwardness in terms of semitones and
//! key colours. This module asks the hand directly: solve for the posture that
//! reaches these keys most comfortably, and report what holding it costs and what
//! getting to it from the previous posture costs.
//!
//! Two things follow from doing it this way rather than with more tables.
//!
//! First, the model becomes tempo-aware. Reconfiguring the hand takes work, and the
//! same reconfiguration is trivial at crotchet 60 and impossible at 168. Rules that
//! count semitones cannot see that; joint angles divided by seconds can.
//!
//! Second, the fingering and the animation agree by construction. The visualizer
//! poses its 3D hands with the same solver, from the same postures, so what you
//! watch is the reasoning rather than a separate performance of it.

use std::cell::RefCell;
use std::collections::HashMap;

use on_hand::ik::{reach, ReachRequest};
use on_hand::keyboard::{is_black, Keyboard, StrikeStyle};
use on_hand::skeleton::{dof, HandPose, Skeleton, DOF};
use on_hand::strain::StrainWeights;
use on_hand::{Finger, Hand, HandProfile};

/// Cost charged when a chord simply cannot be reached with the given fingers.
///
/// Large enough that the solver will take any reachable alternative, finite so that
/// a passage with no reachable fingering still produces an answer rather than no
/// answer.
pub const UNREACHABLE_PENALTY: f32 = 60.0;

/// How many millimetres of wrist travel count as one radian of joint rotation when
/// measuring the work of a transition. The wrist is carried by the whole arm, so
/// moving it is cheaper per unit than bending a finger.
const WRIST_TRAVEL_PER_RADIAN: f32 = 90.0;

/// Below this many seconds, two notes are treated as simultaneous rather than as a
/// transition to be hurried through.
const MIN_TRANSITION_SECONDS: f64 = 0.03;

/// Width of one octave on the keyboard, in millimetres.
const OCTAVE_MM: f32 = 164.0;

/// How much a joint counts toward the work of a transition when its finger is not
/// playing at either end of it.
///
/// A finger with nothing to do drifts between one comfortable resting angle and
/// another. That is not effort, and counting it as effort makes every hand
/// reconfiguration look expensive — enough to swamp the reasons a fingering is
/// chosen in the first place. What costs something is carrying the hand and placing
/// the fingers that have to play.
const IDLE_JOINT_WEIGHT: f32 = 0.15;

/// How fast a hand can reconfigure, in the units [`BiomechModel::transition_cost`]
/// measures work in, per second.
///
/// Calibrated against something checkable: a pianist can cover two octaves — about
/// 3.6 units on this scale — in roughly an eighth of a second, so about 30 units per
/// second is the practical ceiling. Expressing effort relative to that ceiling is
/// what makes ordinary movement free and impossible movement expensive; measuring
/// raw speed instead makes every leap costly and drowns out the reasons a fingering
/// is chosen.
const MAX_RECONFIGURATION_RATE: f32 = 28.0;

/// Relative importance of the biomechanical terms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BiomechWeights {
    /// Weight on the discomfort of holding a chord's posture.
    pub posture: f32,
    /// Weight on the work of moving between postures, per unit of squared speed.
    pub motion: f32,
    /// Comfort weights handed to the hand model itself.
    pub strain: StrainWeights,
}

impl Default for BiomechWeights {
    fn default() -> Self {
        Self {
            posture: 1.0,
            motion: 2.5,
            strain: StrainWeights::default(),
        }
    }
}

/// A chord reduced to what the hand model needs: which finger goes on which key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Grip {
    /// Notes low to high, paired with the fingers that play them.
    pub keys: Vec<(u8, Finger)>,
}

impl Grip {
    /// Build a grip from notes and their fingers.
    pub fn new(keys: Vec<(u8, Finger)>) -> Self {
        Self { keys }
    }

    /// The lowest pitch in the grip, or `None` if it is empty.
    fn base(&self) -> Option<u8> {
        self.keys.first().map(|(m, _)| *m)
    }

    /// How many octaves this grip sits above the reference octave the cache uses.
    ///
    /// Normalising to a middle octave rather than to zero keeps every shifted note
    /// on the keyboard: a hand spans well under two octaves, so a grip anchored in
    /// the octave from middle C never runs off either end.
    fn octave_offset(&self) -> i32 {
        match self.base() {
            Some(base) => (base as i32 - 60).div_euclid(12),
            None => 0,
        }
    }

    /// A cache key that collapses grips differing only by whole octaves.
    ///
    /// Octaves are exactly 164 mm on a real keyboard, so shifting a shape up or down
    /// by twelve semitones gives a geometrically identical problem. Shifting it by
    /// anything else does not, because the black keys move relative to the white
    /// ones. Folding octaves together typically cuts the number of distinct postures
    /// the solver has to compute by an order of magnitude on real music.
    fn cache_key(&self) -> GripKey {
        let shift = self.octave_offset() * 12;
        GripKey(
            self.keys
                .iter()
                .map(|(m, f)| ((*m as i32 - shift) as u8, *f))
                .collect(),
        )
    }

    /// The same shape, moved into the reference octave.
    fn normalised(&self) -> Grip {
        let shift = self.octave_offset() * 12;
        Grip::new(
            self.keys
                .iter()
                .map(|(m, f)| ((*m as i32 - shift) as u8, *f))
                .collect(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GripKey(Vec<(u8, Finger)>);

/// What the hand model concluded about one grip.
#[derive(Debug, Clone, Copy)]
pub struct GripOutcome {
    /// Discomfort of holding the posture.
    pub strain: f32,
    /// Whether every finger actually landed on its key.
    pub reachable: bool,
    /// How far the worst finger fell short, in millimetres.
    pub shortfall_mm: f32,
}

impl GripOutcome {
    /// The cost this grip contributes.
    pub fn cost(&self, weights: &BiomechWeights) -> f32 {
        let mut cost = weights.posture * self.strain;
        if !self.reachable {
            cost += UNREACHABLE_PENALTY + self.shortfall_mm;
        }
        cost
    }
}

/// Solves and caches hand postures for chord grips.
///
/// Not `Sync`: it owns a cache behind a [`RefCell`]. The solver is single-threaded
/// per hand, and the two hands get one of these each.
pub struct BiomechModel {
    skeleton: Skeleton,
    keyboard: Keyboard,
    weights: BiomechWeights,
    postures: RefCell<HashMap<GripKey, (GripOutcome, HandPose)>>,
    work: RefCell<HashMap<(GripKey, GripKey, i16), f32>>,
}

impl BiomechModel {
    /// Build a model for one hand.
    pub fn new(profile: HandProfile, hand: Hand, weights: BiomechWeights) -> Self {
        Self {
            skeleton: Skeleton::new(profile, hand),
            keyboard: Keyboard::new(),
            weights,
            postures: RefCell::new(HashMap::new()),
            work: RefCell::new(HashMap::new()),
        }
    }

    /// The hand this model describes.
    pub fn hand(&self) -> Hand {
        self.skeleton.hand()
    }

    /// The underlying kinematics, for the visualizer.
    pub fn skeleton(&self) -> &Skeleton {
        &self.skeleton
    }

    /// Keyboard geometry.
    pub fn keyboard(&self) -> &Keyboard {
        &self.keyboard
    }

    /// The weights in force.
    pub fn weights(&self) -> &BiomechWeights {
        &self.weights
    }

    /// Where each finger of a grip has to put its tip.
    ///
    /// White keys are played at the front by default, but back in the neck between
    /// the black keys when the same hand also has to reach a black key — which is
    /// what a pianist does, and what keeps the hand from having to rock in and out
    /// within a single chord.
    fn targets(&self, grip: &Grip) -> Vec<(Finger, glam::Vec3)> {
        let any_black = grip.keys.iter().any(|(m, _)| is_black(*m));
        grip.keys
            .iter()
            .map(|(midi, finger)| {
                let style = if is_black(*midi) {
                    StrikeStyle::Black
                } else if any_black {
                    StrikeStyle::WhiteNeck
                } else {
                    StrikeStyle::WhiteFront
                };
                (*finger, self.keyboard.strike_point(*midi, style))
            })
            .collect()
    }

    /// Solve for the posture that plays a grip, caching by octave-equivalent shape.
    ///
    /// The pose that comes back is for the *normalised* grip, in the reference
    /// octave; [`Self::grip_pose`] shifts it back to where the notes really are.
    fn solve(&self, grip: &Grip) -> (GripOutcome, HandPose) {
        let key = grip.cache_key();
        if let Some(hit) = self.postures.borrow().get(&key) {
            return *hit;
        }

        let normalised = grip.normalised();
        let targets = self.targets(&normalised);
        let outcome = reach(&self.skeleton, &ReachRequest::new(&targets));
        let result = (
            GripOutcome {
                strain: outcome.strain_total(),
                reachable: outcome.reached,
                shortfall_mm: outcome.max_error_mm,
            },
            outcome.pose,
        );
        self.postures.borrow_mut().insert(key, result);
        result
    }

    /// What it costs the hand to hold this grip.
    pub fn grip_outcome(&self, grip: &Grip) -> GripOutcome {
        self.solve(grip).0
    }

    /// The posture that plays this grip, for the visualizer to pose a hand with.
    ///
    /// The cache holds the posture for the shape moved into the reference octave;
    /// since an octave is exactly 164 mm, putting it back is a translation.
    pub fn grip_pose(&self, grip: &Grip) -> HandPose {
        let (_, cached) = self.solve(grip);
        let mut pose = cached;
        pose.q[dof::WRIST_X] += grip.octave_offset() as f32 * OCTAVE_MM;
        pose
    }

    /// Cost of holding a grip.
    pub fn grip_cost(&self, grip: &Grip) -> f32 {
        self.grip_outcome(grip).cost(&self.weights)
    }

    /// Joint-space work of reconfiguring from one grip to another, independent of
    /// how long there is to do it.
    fn transition_work(&self, from: &Grip, to: &Grip) -> f32 {
        let (from_key, to_key) = (from.cache_key(), to.cache_key());
        // Octave-equivalence holds for the pair only if their *relative* offset is
        // preserved, so the offset between the two anchors is part of the key.
        let offset = (to.octave_offset() - from.octave_offset()) as i16;
        let cache_key = (from_key, to_key, offset);
        if let Some(hit) = self.work.borrow().get(&cache_key) {
            return *hit;
        }

        // Both poses in real keyboard coordinates, so the octave difference between
        // the two grips counts as the arm travel it actually is.
        let a = self.grip_pose(from);
        let b = self.grip_pose(to);

        let mut sum = 0.0;
        for i in dof::WRIST_DEVIATION..DOF {
            let d = b.q[i] - a.q[i];
            sum += joint_weight(from, to, i) * d * d;
        }
        for i in [dof::WRIST_X, dof::WRIST_Y, dof::WRIST_Z] {
            let d = (b.q[i] - a.q[i]) / WRIST_TRAVEL_PER_RADIAN;
            sum += d * d;
        }

        let work = sum.sqrt();
        self.work.borrow_mut().insert(cache_key, work);
        work
    }

    /// Cost of moving between two grips in the time the music allows.
    ///
    /// The required rate is measured against [`MAX_RECONFIGURATION_RATE`] and
    /// squared, which is the usual way effort scales: at half the hand's top speed a
    /// movement is nearly free, at its limit it costs the full weight, and beyond it
    /// the cost climbs fast. So the same fingering is accepted between two slow notes
    /// and rejected between two quick ones, without ordinary leaps being penalised
    /// merely for being leaps.
    pub fn transition_cost(&self, from: &Grip, to: &Grip, seconds: f64) -> f32 {
        if from.keys.is_empty() || to.keys.is_empty() {
            return 0.0;
        }
        let work = self.transition_work(from, to);
        let dt = seconds.max(MIN_TRANSITION_SECONDS) as f32;
        let demand = work / dt / MAX_RECONFIGURATION_RATE;
        self.weights.motion * demand * demand
    }

    /// How many distinct postures have been solved so far.
    pub fn cached_postures(&self) -> usize {
        self.postures.borrow().len()
    }
}

/// Which finger, if any, a degree of freedom belongs to.
fn owning_finger(index: usize) -> Option<Finger> {
    if (dof::THUMB_CMC_FLEX..=dof::THUMB_IP_FLEX).contains(&index) {
        return Some(Finger::Thumb);
    }
    if index >= dof::FINGER_BASE {
        return Some(Finger::from_index((index - dof::FINGER_BASE) / 3 + 1));
    }
    None
}

/// How much a joint's movement counts, given which fingers are playing.
fn joint_weight(from: &Grip, to: &Grip, index: usize) -> f32 {
    let Some(finger) = owning_finger(index) else {
        // Wrist joints always count: the whole hand rides on them.
        return 1.0;
    };
    let plays = |g: &Grip| g.keys.iter().any(|(_, f)| *f == finger);
    if plays(from) || plays(to) {
        1.0
    } else {
        IDLE_JOINT_WEIGHT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(hand: Hand) -> BiomechModel {
        BiomechModel::new(HandProfile::default(), hand, BiomechWeights::default())
    }

    fn grip(notes: &[(u8, u8)]) -> Grip {
        Grip::new(
            notes
                .iter()
                .map(|(m, f)| (*m, Finger::from_number(*f).unwrap()))
                .collect(),
        )
    }

    #[test]
    fn a_natural_five_finger_shape_is_cheap() {
        let m = model(Hand::Right);
        let g = grip(&[(60, 1), (62, 2), (64, 3), (65, 4), (67, 5)]);
        let out = m.grip_outcome(&g);
        assert!(out.reachable, "five-finger position unreachable");
        assert!(out.strain < 6.0, "five-finger position cost {}", out.strain);
    }

    #[test]
    fn an_impossible_grip_is_reported_as_unreachable() {
        let m = model(Hand::Right);
        // Fingers 4 and 5 asked to span a twelfth.
        let g = grip(&[(60, 4), (79, 5)]);
        let out = m.grip_outcome(&g);
        assert!(!out.reachable);
        assert!(out.cost(m.weights()) > UNREACHABLE_PENALTY);
    }

    #[test]
    fn an_octave_costs_less_with_one_and_five_than_with_four_and_five() {
        let m = model(Hand::Right);
        let sensible = m.grip_cost(&grip(&[(60, 1), (72, 5)]));
        let absurd = m.grip_cost(&grip(&[(60, 4), (72, 5)]));
        assert!(sensible < absurd, "{sensible} should beat {absurd}");
    }

    #[test]
    fn the_same_shape_an_octave_apart_costs_the_same() {
        let m = model(Hand::Right);
        let low = m.grip_cost(&grip(&[(48, 1), (52, 3), (55, 5)]));
        let high = m.grip_cost(&grip(&[(60, 1), (64, 3), (67, 5)]));
        assert!((low - high).abs() < 1e-4, "{low} vs {high}");
        // And it should have been solved only once.
        assert_eq!(m.cached_postures(), 1);
    }

    #[test]
    fn moving_faster_costs_more() {
        let m = model(Hand::Right);
        let a = grip(&[(60, 1)]);
        let b = grip(&[(72, 1)]);
        let slow = m.transition_cost(&a, &b, 2.0);
        let quick = m.transition_cost(&a, &b, 0.2);
        assert!(quick > slow * 10.0, "slow {slow} quick {quick}");
    }

    #[test]
    fn staying_put_costs_nothing_to_move() {
        let m = model(Hand::Right);
        let a = grip(&[(60, 1), (64, 3)]);
        assert!(m.transition_cost(&a, &a, 0.25) < 1e-6);
    }

    #[test]
    fn a_bigger_leap_costs_more_than_a_smaller_one() {
        let m = model(Hand::Right);
        let home = grip(&[(60, 1)]);
        let near = m.transition_cost(&home, &grip(&[(62, 1)]), 0.25);
        let far = m.transition_cost(&home, &grip(&[(84, 1)]), 0.25);
        assert!(far > near, "near {near} far {far}");
    }

    #[test]
    fn both_hands_find_their_own_five_finger_positions() {
        for hand in Hand::ALL {
            let m = model(hand);
            let fingers: Vec<u8> = match hand {
                Hand::Right => vec![1, 2, 3, 4, 5],
                Hand::Left => vec![5, 4, 3, 2, 1],
            };
            let notes: Vec<(u8, u8)> = [60u8, 62, 64, 65, 67]
                .iter()
                .zip(fingers)
                .map(|(m, f)| (*m, f))
                .collect();
            let out = m.grip_outcome(&grip(&notes));
            assert!(out.reachable, "{hand:?} could not reach its own home position");
        }
    }
}
