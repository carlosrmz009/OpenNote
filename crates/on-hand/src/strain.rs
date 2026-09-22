//! How uncomfortable a hand posture is.
//!
//! This is the objective that decides fingerings. It is expressed entirely as a sum
//! of squared residuals so that the same formulation drives both the comfort
//! ranking and the Levenberg-Marquardt solver in [`crate::ik`].
//!
//! The terms are meant to correspond to things that are actually true of hands:
//!
//! * joints resist being held away from their resting angle, and resist it more the
//!   closer they get to the end of their range
//! * adjacent fingers cannot flex independently, because the juncturae tendinum tie
//!   their extensor slips together and the flexor digitorum profundus shares a
//!   muscle belly. The ring finger is the worst affected, which is the anatomical
//!   reason pedagogy calls finger 4 weak
//! * spreading adjacent fingers apart costs more than the interossei like
//! * the wrist wants to stay near neutral, and the forearm wants to stay out of
//!   extreme pronation
//!
//! Nothing here knows anything about music. It is a description of a hand.

use crate::skeleton::{dof, HandPose, Posture, Skeleton, DOF, LIMITS};
use crate::Finger;

/// Relative importance of each source of discomfort.
///
/// Defaults are calibrated so that a hand of average size finds an octave
/// comfortable, a ninth a stretch, and a tenth close to its limit, which is the
/// standard way pianists describe reach.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrainWeights {
    /// Holding a finger joint away from its resting angle.
    pub joint_deviation: f32,
    /// Holding the wrist away from neutral.
    pub wrist_deviation: f32,
    /// Approaching or exceeding the end of a joint's range.
    pub limit_barrier: f32,
    /// Spreading adjacent fingers apart at the knuckle.
    pub spread: f32,
    /// Flexing adjacent fingers by different amounts, against their shared tendons.
    pub tendon_coupling: f32,
    /// Working the thumb away from its resting opposition.
    pub thumb_opposition: f32,
    /// Carrying the hand at an awkward height or depth over the keys.
    pub carriage: f32,
}

impl Default for StrainWeights {
    fn default() -> Self {
        Self {
            joint_deviation: 1.0,
            wrist_deviation: 1.4,
            limit_barrier: 24.0,
            spread: 1.8,
            tendon_coupling: 2.2,
            thumb_opposition: 0.9,
            carriage: 0.6,
        }
    }
}

/// How strongly adjacent fingers are mechanically yoked together.
///
/// Indexed by the gap between fingers: index-middle, middle-ring, ring-little. The
/// ring finger is the least independent digit in the hand; the index and little
/// fingers each have their own extensor and escape some of the coupling.
const COUPLING: [f32; 3] = [0.35, 1.0, 0.7];

/// Rest angle between adjacent fingers at the knuckle, in radians. Fingers sit
/// close together; pulling them apart is work.
const REST_GAP: f32 = 0.0;

/// Difference in total flexion between adjacent fingers when the hand is at rest.
///
/// A relaxed hand does not hold all four fingers at the same angle — the longer
/// middle and ring fingers curl further to bring their tips into the same plane.
/// The coupling term therefore measures departure from *this* differential rather
/// than from zero, so a hand doing nothing costs nothing.
fn rest_flexion_gap(pair: usize) -> f32 {
    let total = |slot: usize| {
        LIMITS[dof::finger(slot) + dof::MCP_FLEX].neutral
            + LIMITS[dof::finger(slot) + dof::PIP_FLEX].neutral
    };
    total(pair) - total(pair + 1)
}

/// Fraction of a joint's range within which the barrier term starts to bite.
const BARRIER_MARGIN: f32 = 0.12;

/// Comfortable height of the wrist above the white keys, in millimetres.
pub const COMFORTABLE_WRIST_Z: f32 = 45.0;
/// Comfortable depth of the wrist from the front edge of the keys, in millimetres.
pub const COMFORTABLE_WRIST_Y: f32 = -80.0;
/// Scale over which departures from a comfortable carriage are measured.
const CARRIAGE_SCALE: f32 = 45.0;

/// A breakdown of where a posture's discomfort comes from.
///
/// Kept separate from the scalar total because explaining *why* a fingering was
/// chosen is a first-class feature, not a debugging aid.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Strain {
    /// Finger joints held away from rest.
    pub joint_deviation: f32,
    /// Wrist held away from neutral.
    pub wrist_deviation: f32,
    /// Joints pushed to the end of their range.
    pub limit_barrier: f32,
    /// Fingers spread apart.
    pub spread: f32,
    /// Adjacent fingers fighting their shared tendons.
    pub tendon_coupling: f32,
    /// Thumb held out of its resting opposition.
    pub thumb_opposition: f32,
    /// Hand carried at an awkward height or depth.
    pub carriage: f32,
}

impl Strain {
    /// The scalar cost.
    pub fn total(&self) -> f32 {
        self.joint_deviation
            + self.wrist_deviation
            + self.limit_barrier
            + self.spread
            + self.tendon_coupling
            + self.thumb_opposition
            + self.carriage
    }

    /// The single largest contributor, for a one-line explanation.
    pub fn dominant(&self) -> (&'static str, f32) {
        let items = [
            ("joint deviation", self.joint_deviation),
            ("wrist deviation", self.wrist_deviation),
            ("joint limits", self.limit_barrier),
            ("finger spread", self.spread),
            ("tendon coupling", self.tendon_coupling),
            ("thumb opposition", self.thumb_opposition),
            ("hand carriage", self.carriage),
        ];
        items
            .into_iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or(("none", 0.0))
    }
}

/// Normalised distance of a joint from its resting angle.
#[inline]
fn deviation(i: usize, q: f32, wrist_neutral: f32) -> f32 {
    (settled(i, q, wrist_neutral) - LIMITS[i].neutral) / LIMITS[i].range()
}

/// A joint angle measured against the neutral that actually applies to it.
///
/// Every joint but one is measured against a fixed neutral written into [`LIMITS`].
/// Wrist deviation is the exception, because radial and ulnar deviation are angles
/// between the hand and the *forearm*: where the forearm is pointing moves the whole
/// window, and a hand held square to the keys out at the end of the keyboard is at the
/// end of its range rather than at the middle of it.
#[inline]
fn settled(i: usize, q: f32, wrist_neutral: f32) -> f32 {
    if i == dof::WRIST_DEVIATION {
        q - wrist_neutral
    } else {
        q
    }
}

/// How far into the forbidden margin near a joint limit a value has strayed,
/// normalised to the joint's range. Zero in the comfortable interior.
#[inline]
fn barrier(i: usize, q: f32, wrist_neutral: f32) -> f32 {
    let l = &LIMITS[i];
    let range = l.range();
    let q = settled(i, q, wrist_neutral);
    let head = (l.max - q) / range;
    let tail = (q - l.min) / range;
    let slack = head.min(tail);
    if slack >= BARRIER_MARGIN {
        0.0
    } else {
        (BARRIER_MARGIN - slack) / BARRIER_MARGIN
    }
}

/// Total angular flexion of a finger, used for the tendon coupling term. The
/// metacarpophalangeal and proximal interphalangeal joints both matter because the
/// long flexors cross both.
#[inline]
fn finger_flexion(pose: &HandPose, slot: usize) -> f32 {
    let b = dof::finger(slot);
    pose.q[b + dof::MCP_FLEX] + pose.q[b + dof::PIP_FLEX]
}

/// Knuckle angle of a finger relative to the palm, including its resting fan.
#[inline]
fn finger_direction(pose: &HandPose, slot: usize) -> f32 {
    pose.q[dof::finger(slot) + dof::MCP_SPREAD]
}

/// Evaluate the discomfort of a posture, broken down by cause.
pub fn strain_breakdown(sk: &Skeleton, posture: &Posture, w: &StrainWeights) -> Strain {
    let pose = &posture.pose;
    let mut s = Strain::default();
    let wrist_neutral = sk.wrist_neutral(pose);

    for i in dof::WRIST_DEVIATION..DOF {
        let d = deviation(i, pose.q[i], wrist_neutral);
        let term = d * d;
        if i <= dof::WRIST_PRONATION {
            s.wrist_deviation += w.wrist_deviation * term;
        } else if (dof::THUMB_CMC_FLEX..=dof::THUMB_IP_FLEX).contains(&i) {
            s.thumb_opposition += w.thumb_opposition * term;
        } else {
            s.joint_deviation += w.joint_deviation * term;
        }
        let b = barrier(i, pose.q[i], wrist_neutral);
        s.limit_barrier += w.limit_barrier * b * b;
    }

    // Fingers pulling away from each other, and fighting each other's tendons.
    for pair in 0..3 {
        let gap = finger_direction(pose, pair) - finger_direction(pose, pair + 1) - REST_GAP;
        s.spread += w.spread * gap * gap;

        let diff =
            finger_flexion(pose, pair) - finger_flexion(pose, pair + 1) - rest_flexion_gap(pair);
        s.tendon_coupling += w.tendon_coupling * COUPLING[pair] * diff * diff;
    }

    // Where the hand is being carried. Height is what a teacher corrects first;
    // depth matters because reaching far in shortens every finger's usable travel.
    let wrist = posture.wrist;
    let dz = (wrist.z - COMFORTABLE_WRIST_Z) / CARRIAGE_SCALE;
    let dy = (wrist.y - COMFORTABLE_WRIST_Y) / CARRIAGE_SCALE;
    s.carriage = w.carriage * (dz * dz + dy * dy);

    s
}

/// Evaluate the discomfort of a posture as a single number.
pub fn strain(sk: &Skeleton, posture: &Posture, w: &StrainWeights) -> f32 {
    strain_breakdown(sk, posture, w).total()
}

/// A single squared term of the strain objective, expressed so the solver can use it.
///
/// Each entry says: the residual value, and how it changes with up to two degrees of
/// freedom. Every strain term is linear in the joint variables, which keeps the
/// Jacobian exact and the solve well behaved.
#[derive(Debug, Clone, Copy)]
pub struct StrainResidual {
    /// Current value of the residual.
    pub value: f32,
    /// `(degree of freedom, derivative)` pairs; unused slots have a zero derivative.
    /// Four slots because the tendon coupling term spans two joints on each of two
    /// fingers.
    pub grad: [(usize, f32); 4],
}

/// A gradient entry that contributes nothing.
const NO_GRAD: (usize, f32) = (0, 0.0);

/// Build the strain residual vector for the solver.
///
/// The squared sum of these residuals equals [`strain`] up to the carriage term,
/// which is supplied separately because it depends on wrist translation rather than
/// joint angles.
pub fn strain_residuals(
    w: &StrainWeights,
    pose: &HandPose,
    wrist_neutral: f32,
    out: &mut Vec<StrainResidual>,
) {
    out.clear();

    for i in dof::WRIST_DEVIATION..DOF {
        let weight = if i <= dof::WRIST_PRONATION {
            w.wrist_deviation
        } else if (dof::THUMB_CMC_FLEX..=dof::THUMB_IP_FLEX).contains(&i) {
            w.thumb_opposition
        } else {
            w.joint_deviation
        };
        let k = weight.sqrt() / LIMITS[i].range();
        out.push(StrainResidual {
            value: k * (settled(i, pose.q[i], wrist_neutral) - LIMITS[i].neutral),
            grad: [(i, k), NO_GRAD, NO_GRAD, NO_GRAD],
        });

        let b = barrier(i, pose.q[i], wrist_neutral);
        if b > 0.0 {
            // Sub-gradient of the barrier: it pushes back toward whichever limit is
            // being crowded.
            let range = LIMITS[i].range();
            let settled = settled(i, pose.q[i], wrist_neutral);
            let toward_max = (LIMITS[i].max - settled) < (settled - LIMITS[i].min);
            let slope = if toward_max { 1.0 } else { -1.0 } / (BARRIER_MARGIN * range);
            let k = w.limit_barrier.sqrt();
            out.push(StrainResidual {
                value: k * b,
                grad: [(i, k * slope), NO_GRAD, NO_GRAD, NO_GRAD],
            });
        }
    }

    for pair in 0..3 {
        let a = dof::finger(pair) + dof::MCP_SPREAD;
        let b = dof::finger(pair + 1) + dof::MCP_SPREAD;
        let k = w.spread.sqrt();
        out.push(StrainResidual {
            value: k * (pose.q[a] - pose.q[b] - REST_GAP),
            grad: [(a, k), (b, -k), NO_GRAD, NO_GRAD],
        });

        // Differential flexion works against the shared long flexors. Both joints
        // of both fingers enter the same residual, because it is the total flexion
        // of each finger that the tendons compare.
        let k = (w.tendon_coupling * COUPLING[pair]).sqrt();
        let (a_mcp, a_pip) = (
            dof::finger(pair) + dof::MCP_FLEX,
            dof::finger(pair) + dof::PIP_FLEX,
        );
        let (b_mcp, b_pip) = (
            dof::finger(pair + 1) + dof::MCP_FLEX,
            dof::finger(pair + 1) + dof::PIP_FLEX,
        );
        let diff = finger_flexion(pose, pair) - finger_flexion(pose, pair + 1)
            - rest_flexion_gap(pair);
        out.push(StrainResidual {
            value: k * diff,
            grad: [(a_mcp, k), (a_pip, k), (b_mcp, -k), (b_pip, -k)],
        });
    }
}

/// The carriage residuals, which depend on where the wrist is rather than how the
/// joints are bent.
pub fn carriage_residuals(w: &StrainWeights, pose: &HandPose) -> [StrainResidual; 2] {
    let k = w.carriage.sqrt() / CARRIAGE_SCALE;
    [
        StrainResidual {
            value: k * (pose.q[dof::WRIST_Z] - COMFORTABLE_WRIST_Z),
            grad: [(dof::WRIST_Z, k), NO_GRAD, NO_GRAD, NO_GRAD],
        },
        StrainResidual {
            value: k * (pose.q[dof::WRIST_Y] - COMFORTABLE_WRIST_Y),
            grad: [(dof::WRIST_Y, k), NO_GRAD, NO_GRAD, NO_GRAD],
        },
    ]
}

/// Which fingers a posture is asking the most of.
pub fn worst_finger(sk: &Skeleton, posture: &Posture, w: &StrainWeights) -> Option<Finger> {
    let _ = (sk, w);
    let pose = &posture.pose;
    (0..4)
        .map(|slot| {
            let f = Finger::from_index(slot + 1);
            let b = dof::finger(slot);
            let cost: f32 = [dof::MCP_FLEX, dof::MCP_SPREAD, dof::PIP_FLEX]
                .iter()
                .map(|o| {
                    let i = b + o;
                    // Finger joints only, never the wrist, so the wrist's
                    // neutral does not arise.
                    let d = deviation(i, pose.q[i], 0.0);
                    d * d + barrier(i, pose.q[i], 0.0)
                })
                .sum();
            (f, cost)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(f, _)| f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::HandProfile;
    use crate::skeleton::LIMITS as L;
    use crate::Hand;
    use glam::Vec3;

    fn sk() -> Skeleton {
        Skeleton::new(HandProfile::default(), Hand::Right)
    }

    /// Where this hand's own shoulder is, along the keyboard. In front of it the
    /// forearm points straight down the keys, which is the one place a hand square to
    /// them is also a straight wrist.
    fn home_x(sk: &Skeleton) -> f32 {
        sk.torso().shoulder(sk.hand(), 0.0).x
    }

    #[test]
    fn the_resting_hand_is_the_most_comfortable_one() {
        let sk = sk();
        let w = StrainWeights::default();
        let rest = sk.rest_pose(Vec3::new(home_x(&sk), COMFORTABLE_WRIST_Y, COMFORTABLE_WRIST_Z));
        let base = strain(&sk, &sk.forward(&rest), &w);
        assert!(base < 1e-3, "rest posture should cost nothing, got {base}");

        for i in dof::WRIST_DEVIATION..DOF {
            let mut bent = rest;
            bent.q[i] += L[i].range() * 0.25;
            let cost = strain(&sk, &sk.forward(&bent), &w);
            assert!(cost > base, "bending dof {i} should cost more than rest");
        }
    }

    #[test]
    fn the_same_posture_costs_more_the_further_it_is_from_its_own_shoulder() {
        // The point of giving the hand an arm. Held square to the keys, a hand is at a
        // straight wrist only in front of its own shoulder; anywhere else the forearm
        // arrives at an angle and the wrist has to make up the difference. A model that
        // charges deviation from a fixed zero says the whole keyboard is alike, which
        // is the one thing every pianist knows to be false.
        let sk = sk();
        let w = StrainWeights::default();
        let at = |x: f32| {
            let pose = sk.rest_pose(Vec3::new(x, COMFORTABLE_WRIST_Y, COMFORTABLE_WRIST_Z));
            strain(&sk, &sk.forward(&pose), &w)
        };
        let home = home_x(&sk);
        let here = at(home);
        let across = at(home - 500.0);
        assert!(
            across > here + 0.05,
            "reaching across the body should cost more: {across} against {here}"
        );

        // And the left hand is the mirror of it, which is what makes a note at the
        // bottom of the keyboard the left hand's to play.
        let left = Skeleton::new(HandProfile::default(), Hand::Left);
        let left_at = |x: f32| {
            let pose = left.rest_pose(Vec3::new(x, COMFORTABLE_WRIST_Y, COMFORTABLE_WRIST_Z));
            strain(&left, &left.forward(&pose), &w)
        };
        let low = home_x(&left) - 300.0;
        assert!(
            left_at(low) < at(low),
            "the left hand should be the comfortable one down there: {} against {}",
            left_at(low),
            at(low)
        );
    }

    #[test]
    fn the_ring_finger_is_the_expensive_one_to_move_alone() {
        let sk = sk();
        let w = StrainWeights::default();
        let rest = sk.rest_pose(Vec3::new(400.0, COMFORTABLE_WRIST_Y, COMFORTABLE_WRIST_Z));

        // Lift one finger while its neighbours stay put, and see what it costs.
        let cost_of_moving = |slot: usize| {
            let mut p = rest;
            p.q[dof::finger(slot) + dof::MCP_FLEX] += 0.35;
            strain(&sk, &sk.forward(&p), &w)
        };
        let index = cost_of_moving(0);
        let ring = cost_of_moving(2);
        assert!(
            ring > index,
            "moving the ring finger alone ({ring}) should cost more than the index ({index})"
        );
    }

    #[test]
    fn crowding_a_joint_limit_is_penalised_sharply() {
        let sk = sk();
        let w = StrainWeights::default();
        let rest = sk.rest_pose(Vec3::new(400.0, COMFORTABLE_WRIST_Y, COMFORTABLE_WRIST_Z));
        let i = dof::finger(1) + dof::PIP_FLEX;

        let mut mid = rest;
        mid.q[i] = L[i].min + L[i].range() * 0.5;
        let mut edge = rest;
        edge.q[i] = L[i].max - L[i].range() * 0.01;

        let mid_cost = strain(&sk, &sk.forward(&mid), &w);
        let edge_cost = strain(&sk, &sk.forward(&edge), &w);
        assert!(edge_cost > 4.0 * mid_cost, "{edge_cost} vs {mid_cost}");
    }

    #[test]
    fn breakdown_sums_to_the_total_and_names_a_cause() {
        let sk = sk();
        let w = StrainWeights::default();
        let mut p = sk.rest_pose(Vec3::new(400.0, 10.0, 90.0));
        p.q[dof::finger(3) + dof::MCP_SPREAD] = -0.4;
        let posture = sk.forward(&p);
        let b = strain_breakdown(&sk, &posture, &w);
        assert!((b.total() - strain(&sk, &posture, &w)).abs() < 1e-6);
        let (name, value) = b.dominant();
        assert!(value > 0.0);
        assert!(!name.is_empty());
    }

    #[test]
    fn residuals_reproduce_the_scalar_strain() {
        let sk = sk();
        let w = StrainWeights::default();
        let mut p = sk.rest_pose(Vec3::new(400.0, 5.0, 80.0));
        for (i, q) in p.q.iter_mut().enumerate().skip(dof::WRIST_DEVIATION) {
            *q += L[i].range() * 0.11;
        }
        sk.clamp(&mut p);

        // The same neutral `strain` will use internally: the two are only equal if
        // they measure the wrist against the same place.
        let mut res = Vec::new();
        strain_residuals(&w, &p, sk.wrist_neutral(&p), &mut res);
        let mut sum: f32 = res.iter().map(|r| r.value * r.value).sum();
        sum += carriage_residuals(&w, &p).iter().map(|r| r.value * r.value).sum::<f32>();

        let direct = strain(&sk, &sk.forward(&p), &w);
        assert!(
            (sum - direct).abs() < 5e-3 * direct.max(1.0),
            "residual sum {sum} vs direct {direct}"
        );
    }

    #[test]
    fn residual_gradients_match_finite_differences() {
        let w = StrainWeights::default();
        let sk = sk();
        let mut p = sk.rest_pose(Vec3::new(400.0, 5.0, 80.0));
        for (i, q) in p.q.iter_mut().enumerate().skip(dof::WRIST_DEVIATION) {
            *q += L[i].range() * 0.07;
        }

        // A non-zero wrist neutral, so the shifted path is the one under test, held
        // fixed across the perturbations exactly as the solver holds it.
        let wrist_neutral = 0.15;
        let mut res = Vec::new();
        strain_residuals(&w, &p, wrist_neutral, &mut res);
        let h = 1e-5;
        for (n, r) in res.iter().enumerate() {
            for (dof_index, analytic) in r.grad {
                if analytic == 0.0 {
                    continue;
                }
                let mut plus = p;
                plus.q[dof_index] += h;
                let mut minus = p;
                minus.q[dof_index] -= h;
                let (mut rp, mut rm) = (Vec::new(), Vec::new());
                strain_residuals(&w, &plus, wrist_neutral, &mut rp);
                strain_residuals(&w, &minus, wrist_neutral, &mut rm);
                // Skip if the barrier turned on or off across the step and changed
                // the residual count.
                if rp.len() != res.len() || rm.len() != res.len() {
                    continue;
                }
                let numeric = (rp[n].value - rm[n].value) / (2.0 * h);
                assert!(
                    (numeric - analytic).abs() < 1e-2 * analytic.abs().max(1.0),
                    "residual {n} dof {dof_index}: {analytic} vs {numeric}"
                );
            }
        }
    }
}
