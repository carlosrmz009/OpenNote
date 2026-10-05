use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, LazyLock, RwLock};

use on_hand::ik::{reach, ReachRequest};
use on_hand::keyboard::{is_black, Keyboard, StrikeStyle};
use on_hand::skeleton::{dof, HandPose, Skeleton, DOF};
use on_hand::strain::StrainWeights;
use on_hand::{Finger, Hand, HandProfile};

pub const UNREACHABLE_PENALTY: f32 = 60.0;

const WRIST_TRAVEL_PER_RADIAN: f32 = 90.0;

const MIN_TRANSITION_SECONDS: f64 = 0.03;

const OCTAVE_MM: f32 = 164.0;

const IDLE_JOINT_WEIGHT: f32 = 0.15;

const MAX_RECONFIGURATION_RATE: f32 = 28.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BiomechWeights {
    pub posture: f32,
    pub motion: f32,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Grip {
    pub keys: Vec<(u8, Finger)>,
}

impl Grip {
    pub fn new(keys: Vec<(u8, Finger)>) -> Self {
        Self { keys }
    }

    fn base(&self) -> Option<u8> {
        self.keys.iter().map(|(m, _)| *m).min()
    }

    fn top(&self) -> Option<u8> {
        self.keys.iter().map(|(m, _)| *m).max()
    }

    fn octave_offset(&self) -> i32 {
        let (Some(base), Some(top)) = (self.base(), self.top()) else {
            return 0;
        };
        let (lowest, highest) = (
            on_hand::keyboard::MIDI_LOWEST as i32,
            on_hand::keyboard::MIDI_HIGHEST as i32,
        );
        let most = (base as i32 - lowest).div_euclid(12);
        let least = (top as i32 - highest + 11).div_euclid(12);
        ((base as i32 - 60).div_euclid(12)).clamp(least, most)
    }

    fn cache_key(&self) -> GripKey {
        let shift = self.octave_offset() * 12;
        GripKey(
            self.keys
                .iter()
                .map(|(m, f)| ((*m as i32 - shift) as u8, *f))
                .collect(),
        )
    }

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

#[derive(Debug, Clone, Copy)]
pub struct GripOutcome {
    pub strain: f32,
    pub reachable: bool,
    pub shortfall_mm: f32,
}

impl GripOutcome {
    pub fn cost(&self, weights: &BiomechWeights) -> f32 {
        let mut cost = weights.posture * self.strain;
        if !self.reachable {
            cost += UNREACHABLE_PENALTY + self.shortfall_mm;
        }
        cost
    }
}

#[derive(Default)]
struct Geometry {
    postures: RwLock<HashMap<GripKey, (GripOutcome, HandPose)>>,
    moved: RwLock<HashMap<GripKey, HandPose>>,
    work: RwLock<HashMap<(GripKey, GripKey, i16), f32>>,
}

const CACHE_CAP: usize = 200_000;

fn remember<K: Eq + Hash, V>(map: &RwLock<HashMap<K, V>>, key: K, value: V) {
    let mut map = map.write().unwrap();
    if map.len() >= CACHE_CAP {
        map.clear();
    }
    map.insert(key, value);
}

fn profile_key(profile: &HandProfile) -> Vec<u32> {
    let mut key = vec![profile.hand_length.to_bits()];
    for d in &profile.digits {
        key.extend([d.metacarpal, d.proximal, d.medial, d.distal, d.pulp].map(f32::to_bits));
    }
    for (x, y, z) in &profile.base_offsets {
        key.extend([x, y, z].map(|v| v.to_bits()));
    }
    key
}

static GEOMETRY: LazyLock<RwLock<HashMap<(Hand, Vec<u32>), Arc<Geometry>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

fn geometry_for(hand: Hand, profile: &HandProfile) -> Arc<Geometry> {
    let key = (hand, profile_key(profile));
    if let Some(hit) = GEOMETRY.read().unwrap().get(&key) {
        return Arc::clone(hit);
    }
    Arc::clone(GEOMETRY.write().unwrap().entry(key).or_default())
}

pub struct BiomechModel {
    skeleton: Skeleton,
    keyboard: Keyboard,
    weights: BiomechWeights,
    cache: Arc<Geometry>,
}

impl BiomechModel {
    pub fn new(profile: HandProfile, hand: Hand, weights: BiomechWeights) -> Self {
        let cache = geometry_for(hand, &profile);
        Self {
            skeleton: Skeleton::new(profile, hand),
            keyboard: Keyboard::new(),
            weights,
            cache,
        }
    }

    pub fn hand(&self) -> Hand {
        self.skeleton.hand()
    }

    pub fn skeleton(&self) -> &Skeleton {
        &self.skeleton
    }

    pub fn keyboard(&self) -> &Keyboard {
        &self.keyboard
    }

    pub fn weights(&self) -> &BiomechWeights {
        &self.weights
    }

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

    fn solve(&self, grip: &Grip) -> (GripOutcome, HandPose) {
        let key = grip.cache_key();
        if let Some(hit) = self.cache.postures.read().unwrap().get(&key) {
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
        remember(&self.cache.postures, key, result);
        result
    }

    fn wrist_is_legal(&self, pose: &HandPose) -> bool {
        let relative = pose.q[dof::WRIST_DEVIATION] - self.skeleton.wrist_neutral(pose);
        on_hand::skeleton::LIMITS[dof::WRIST_DEVIATION].violation(relative) <= 0.0
    }

    fn solved_where_it_is(&self, grip: &Grip) -> HandPose {
        let key = GripKey(grip.keys.clone());
        if let Some(hit) = self.cache.moved.read().unwrap().get(&key) {
            return *hit;
        }
        let targets = self.targets(grip);
        let pose = reach(&self.skeleton, &ReachRequest::new(&targets)).pose;
        remember(&self.cache.moved, key, pose);
        pose
    }

    pub fn grip_outcome(&self, grip: &Grip) -> GripOutcome {
        self.solve(grip).0
    }

    pub fn grip_pose(&self, grip: &Grip) -> HandPose {
        let (_, cached) = self.solve(grip);
        let mut pose = cached;
        pose.q[dof::WRIST_X] += grip.octave_offset() as f32 * OCTAVE_MM;
        if !self.wrist_is_legal(&pose) {
            return self.solved_where_it_is(grip);
        }
        pose
    }

    pub fn grip_pose_near(&self, grip: &Grip, near: &HandPose, weights: [f32; on_hand::skeleton::DOF]) -> (HandPose, f32) {
        let targets = self.targets(grip);
        let cold = self.grip_pose(grip);
        let outcome = reach(&self.skeleton, &ReachRequest::new(&targets).from_pose(cold).near(*near, weights));
        (outcome.pose, outcome.max_error_mm)
    }

    pub fn grip_cost(&self, grip: &Grip) -> f32 {
        self.grip_outcome(grip).cost(&self.weights)
    }

    fn transition_work(&self, from: &Grip, to: &Grip) -> f32 {
        let (from_key, to_key) = (from.cache_key(), to.cache_key());
        let offset = (to.octave_offset() - from.octave_offset()) as i16;
        let cache_key = (from_key, to_key, offset);
        if let Some(hit) = self.cache.work.read().unwrap().get(&cache_key) {
            return *hit;
        }

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
        remember(&self.cache.work, cache_key, work);
        work
    }

    pub fn transition_cost(&self, from: &Grip, to: &Grip, seconds: f64) -> f32 {
        if from.keys.is_empty() || to.keys.is_empty() {
            return 0.0;
        }
        let work = self.transition_work(from, to);
        let dt = seconds.max(MIN_TRANSITION_SECONDS) as f32;
        let demand = work / dt / MAX_RECONFIGURATION_RATE;
        self.weights.motion * demand * demand
    }

    pub fn cached_postures(&self) -> usize {
        self.cache.postures.read().unwrap().len()
    }
}

fn owning_finger(index: usize) -> Option<Finger> {
    if (dof::THUMB_CMC_FLEX..=dof::THUMB_IP_FLEX).contains(&index) {
        return Some(Finger::Thumb);
    }
    if index >= dof::FINGER_BASE {
        return Some(Finger::from_index((index - dof::FINGER_BASE) / 3 + 1));
    }
    None
}

fn joint_weight(from: &Grip, to: &Grip, index: usize) -> f32 {
    let Some(finger) = owning_finger(index) else {
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
        let m = BiomechModel::new(
            HandProfile::from_hand_length(183.5),
            Hand::Right,
            BiomechWeights::default(),
        );
        let low = m.grip_cost(&grip(&[(48, 1), (52, 3), (55, 5)]));
        let high = m.grip_cost(&grip(&[(60, 1), (64, 3), (67, 5)]));
        assert!((low - high).abs() < 1e-4, "{low} vs {high}");
        assert_eq!(m.cached_postures(), 1);
    }

    #[test]
    fn a_pose_moved_to_another_octave_is_still_one_the_arm_can_hold() {
        let model = model(Hand::Right);
        let skeleton = model.skeleton();
        for base in [60u8, 24] {
            let pose = model.grip_pose(&grip(&[(base, 1), (base + 4, 3)]));
            let mut clamped = pose;
            skeleton.clamp(&mut clamped);
            let (before, after) = (skeleton.forward(&pose), skeleton.forward(&clamped));
            for finger in 0..5 {
                let moved = before.chain[finger][3].distance(after.chain[finger][3]);
                assert!(
                    moved < 1.0,
                    "clamping the pose for {base} moved finger {finger} by {moved:.1} mm,                      so what is drawn is not what was solved"
                );
            }
        }
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
