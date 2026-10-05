use crate::skeleton::{dof, HandPose, Posture, Skeleton, DOF, LIMITS};
use crate::Finger;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrainWeights {
    pub joint_deviation: f32,
    pub wrist_deviation: f32,
    pub limit_barrier: f32,
    pub spread: f32,
    pub tendon_coupling: f32,
    pub thumb_opposition: f32,
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

const COUPLING: [f32; 3] = [0.35, 1.0, 0.7];

const REST_GAP: f32 = 0.0;

fn rest_flexion_gap(pair: usize) -> f32 {
    let total = |slot: usize| {
        LIMITS[dof::finger(slot) + dof::MCP_FLEX].neutral
            + LIMITS[dof::finger(slot) + dof::PIP_FLEX].neutral
    };
    total(pair) - total(pair + 1)
}

const BARRIER_MARGIN: f32 = 0.12;

pub const COMFORTABLE_WRIST_Z: f32 = 45.0;
pub const COMFORTABLE_WRIST_Y: f32 = -80.0;
const CARRIAGE_SCALE: f32 = 45.0;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Strain {
    pub joint_deviation: f32,
    pub wrist_deviation: f32,
    pub limit_barrier: f32,
    pub spread: f32,
    pub tendon_coupling: f32,
    pub thumb_opposition: f32,
    pub carriage: f32,
}

impl Strain {
    pub fn total(&self) -> f32 {
        self.joint_deviation
            + self.wrist_deviation
            + self.limit_barrier
            + self.spread
            + self.tendon_coupling
            + self.thumb_opposition
            + self.carriage
    }

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

#[inline]
fn deviation(i: usize, q: f32, wrist_neutral: f32) -> f32 {
    (settled(i, q, wrist_neutral) - LIMITS[i].neutral) / LIMITS[i].range()
}

#[inline]
fn settled(i: usize, q: f32, wrist_neutral: f32) -> f32 {
    if i == dof::WRIST_DEVIATION {
        q - wrist_neutral
    } else {
        q
    }
}

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

#[inline]
fn finger_flexion(pose: &HandPose, slot: usize) -> f32 {
    let b = dof::finger(slot);
    pose.q[b + dof::MCP_FLEX] + pose.q[b + dof::PIP_FLEX]
}

#[inline]
fn finger_direction(pose: &HandPose, slot: usize) -> f32 {
    pose.q[dof::finger(slot) + dof::MCP_SPREAD]
}

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

    for pair in 0..3 {
        let gap = finger_direction(pose, pair) - finger_direction(pose, pair + 1) - REST_GAP;
        s.spread += w.spread * gap * gap;

        let diff =
            finger_flexion(pose, pair) - finger_flexion(pose, pair + 1) - rest_flexion_gap(pair);
        s.tendon_coupling += w.tendon_coupling * COUPLING[pair] * diff * diff;
    }

    let wrist = posture.wrist;
    let dz = (wrist.z - COMFORTABLE_WRIST_Z) / CARRIAGE_SCALE;
    let dy = (wrist.y - COMFORTABLE_WRIST_Y) / CARRIAGE_SCALE;
    s.carriage = w.carriage * (dz * dz + dy * dy);

    s
}

pub fn strain(sk: &Skeleton, posture: &Posture, w: &StrainWeights) -> f32 {
    strain_breakdown(sk, posture, w).total()
}

#[derive(Debug, Clone, Copy)]
pub struct StrainResidual {
    pub value: f32,
    pub grad: [(usize, f32); 4],
}

const NO_GRAD: (usize, f32) = (0, 0.0);

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
