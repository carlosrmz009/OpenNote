use glam::Vec3;

use crate::skeleton::{dof, HandPose, Posture, Skeleton, DOF};
use crate::strain::{
    carriage_residuals, strain_breakdown, strain_residuals, Strain, StrainResidual, StrainWeights,
    COMFORTABLE_WRIST_Y, COMFORTABLE_WRIST_Z,
};
use crate::Finger;

pub const CONTACT_TOLERANCE_MM: f32 = 1.5;

pub const IDLE_CLEARANCE_MM: f32 = 10.0;

#[derive(Debug, Clone)]
pub struct ReachRequest<'a> {
    pub targets: &'a [(Finger, Vec3)],
    pub seed: Option<HandPose>,
    pub weights: StrainWeights,
    pub position_weight: f32,
    pub max_iterations: usize,
    pub idle_clearance: f32,
    pub clearance_weight: f32,
}

impl<'a> ReachRequest<'a> {
    pub fn new(targets: &'a [(Finger, Vec3)]) -> Self {
        Self {
            targets,
            seed: None,
            weights: StrainWeights::default(),
            position_weight: 40.0,
            max_iterations: 60,
            idle_clearance: IDLE_CLEARANCE_MM,
            clearance_weight: 6.0,
        }
    }

    pub fn from_pose(mut self, seed: HandPose) -> Self {
        self.seed = Some(seed);
        self
    }

    pub fn with_weights(mut self, weights: StrainWeights) -> Self {
        self.weights = weights;
        self
    }

    fn is_playing(&self, finger: Finger) -> bool {
        self.targets.iter().any(|(f, _)| *f == finger)
    }
}

#[derive(Debug, Clone)]
pub struct ReachOutcome {
    pub pose: HandPose,
    pub posture: Posture,
    pub max_error_mm: f32,
    pub rms_error_mm: f32,
    pub strain: Strain,
    pub reached: bool,
}

impl ReachOutcome {
    pub fn strain_total(&self) -> f32 {
        self.strain.total()
    }
}

pub fn seed_pose(sk: &Skeleton, targets: &[(Finger, Vec3)]) -> HandPose {
    if targets.is_empty() {
        return sk.rest_pose(Vec3::new(0.0, COMFORTABLE_WRIST_Y, COMFORTABLE_WRIST_Z));
    }

    let mut sum = Vec3::ZERO;
    let mut lateral_bias = 0.0;
    for (finger, p) in targets {
        sum += *p;
        lateral_bias -= sk.hand().sign() * sk.profile().base_offset(*finger).0;
    }
    let n = targets.len() as f32;
    let centre = sum / n;
    let bias = lateral_bias / n;

    let mut pose = sk.rest_pose(Vec3::new(
        centre.x - bias,
        COMFORTABLE_WRIST_Y,
        COMFORTABLE_WRIST_Z,
    ));
    let mean_depth = sum.y / n;
    pose.q[dof::WRIST_Y] += (mean_depth - 30.0) * 0.6;
    pose
}

struct Row {
    residual: f32,
    grad: Vec<(usize, f32)>,
}

pub fn reach(sk: &Skeleton, req: &ReachRequest<'_>) -> ReachOutcome {
    let mut best: Option<(f32, ReachOutcome)> = None;
    for seed in seeds(sk, req) {
        let outcome = reach_from(sk, req, seed);
        let cost = objective(sk, &outcome.posture, req);
        let better = match &best {
            Some((best_cost, _)) => cost < *best_cost,
            None => true,
        };
        if better {
            best = Some((cost, outcome));
        }
    }
    best.expect("at least one seed is always tried").1
}

fn seeds(sk: &Skeleton, req: &ReachRequest<'_>) -> Vec<HandPose> {
    let base = req.seed.unwrap_or_else(|| seed_pose(sk, req.targets));
    let mut out = vec![base];

    for scale in [0.55f32, -0.5] {
        let mut variant = base;
        for slot in 0..4 {
            let b = dof::finger(slot);
            for offset in [crate::skeleton::dof::MCP_FLEX, crate::skeleton::dof::PIP_FLEX] {
                let i = b + offset;
                variant.q[i] += crate::skeleton::LIMITS[i].range() * scale * 0.5;
            }
        }
        variant.q[dof::THUMB_CMC_FLEX] +=
            crate::skeleton::LIMITS[dof::THUMB_CMC_FLEX].range() * scale * 0.3;
        sk.clamp(&mut variant);
        out.push(variant);
    }

    for deviation in [0.35f32, -0.3] {
        let mut variant = base;
        variant.q[dof::WRIST_DEVIATION] += deviation;
        variant.q[dof::WRIST_PRONATION] += deviation * 0.5;
        sk.clamp(&mut variant);
        out.push(variant);
    }
    out
}

fn reach_from(sk: &Skeleton, req: &ReachRequest<'_>, seed: HandPose) -> ReachOutcome {
    let mut pose = seed;
    sk.clamp(&mut pose);

    let mut lambda = 1e-2f32;
    let mut posture = sk.forward(&pose);
    let mut cost = objective(sk, &posture, req);

    let mut rows: Vec<Row> = Vec::with_capacity(3 * req.targets.len() + 40);
    let mut scratch: Vec<StrainResidual> = Vec::new();

    for _ in 0..req.max_iterations {
        build_rows(sk, &posture, req, &mut rows, &mut scratch);

        let mut ata = [[0.0f32; DOF]; DOF];
        let mut atb = [0.0f32; DOF];
        for row in &rows {
            for &(i, gi) in &row.grad {
                if gi == 0.0 {
                    continue;
                }
                atb[i] -= gi * row.residual;
                for &(j, gj) in &row.grad {
                    if gj != 0.0 {
                        ata[i][j] += gi * gj;
                    }
                }
            }
        }

        let mut improved = false;
        for _ in 0..8 {
            let mut damped = ata;
            for i in 0..DOF {
                damped[i][i] += lambda * ata[i][i].max(1e-6);
            }
            let Some(delta) = solve_symmetric(&damped, &atb) else {
                lambda *= 4.0;
                continue;
            };

            let mut candidate = pose;
            for i in 0..DOF {
                candidate.q[i] += delta[i];
            }
            sk.clamp(&mut candidate);
            let candidate_posture = sk.forward(&candidate);
            let candidate_cost = objective(sk, &candidate_posture, req);

            if candidate_cost < cost {
                let gain = cost - candidate_cost;
                pose = candidate;
                posture = candidate_posture;
                cost = candidate_cost;
                lambda = (lambda * 0.4).max(1e-8);
                improved = true;
                if gain < 1e-7 * cost.max(1.0) {
                    return finish(sk, pose, posture, req);
                }
                break;
            }
            lambda *= 4.0;
            if lambda > 1e8 {
                return finish(sk, pose, posture, req);
            }
        }

        if !improved {
            break;
        }
    }

    finish(sk, pose, posture, req)
}

fn build_rows(
    sk: &Skeleton,
    posture: &Posture,
    req: &ReachRequest<'_>,
    rows: &mut Vec<Row>,
    scratch: &mut Vec<StrainResidual>,
) {
    rows.clear();
    let k = req.position_weight.sqrt();

    for (finger, target) in req.targets {
        let tip = posture.tip(*finger);
        let error = tip - *target;
        for axis in 0..3 {
            let mut grad = Vec::with_capacity(10);
            for j in 0..DOF {
                let d = sk.tip_jacobian(posture, *finger, j);
                let g = k * d[axis];
                if g != 0.0 {
                    grad.push((j, g));
                }
            }
            rows.push(Row { residual: k * error[axis], grad });
        }
    }

    let k = req.clearance_weight.sqrt();
    for finger in Finger::ALL {
        if req.is_playing(finger) {
            continue;
        }
        let height = posture.tip(finger).z;
        if height >= req.idle_clearance {
            continue;
        }
        let mut grad = Vec::with_capacity(10);
        for j in 0..DOF {
            let d = sk.tip_jacobian(posture, finger, j);
            if d.z != 0.0 {
                grad.push((j, -k * d.z));
            }
        }
        rows.push(Row { residual: k * (req.idle_clearance - height), grad });
    }

    let wrist_neutral = sk.wrist_neutral(&posture.pose);
    strain_residuals(&req.weights, &posture.pose, wrist_neutral, scratch);
    for r in scratch.iter() {
        rows.push(Row {
            residual: r.value,
            grad: r.grad.iter().copied().filter(|(_, g)| *g != 0.0).collect(),
        });
    }
    for r in carriage_residuals(&req.weights, &posture.pose) {
        rows.push(Row {
            residual: r.value,
            grad: r.grad.iter().copied().filter(|(_, g)| *g != 0.0).collect(),
        });
    }
}

fn objective(sk: &Skeleton, posture: &Posture, req: &ReachRequest<'_>) -> f32 {
    let mut total = 0.0;
    for (finger, target) in req.targets {
        total += req.position_weight * (posture.tip(*finger) - *target).length_squared();
    }
    for finger in Finger::ALL {
        if req.is_playing(finger) {
            continue;
        }
        let shortfall = (req.idle_clearance - posture.tip(finger).z).max(0.0);
        total += req.clearance_weight * shortfall * shortfall;
    }
    total + strain_breakdown(sk, posture, &req.weights).total()
}

fn finish(sk: &Skeleton, pose: HandPose, posture: Posture, req: &ReachRequest<'_>) -> ReachOutcome {
    let mut max_error: f32 = 0.0;
    let mut sum_sq = 0.0;
    for (finger, target) in req.targets {
        let e = (posture.tip(*finger) - *target).length();
        max_error = max_error.max(e);
        sum_sq += e * e;
    }
    let rms = if req.targets.is_empty() {
        0.0
    } else {
        (sum_sq / req.targets.len() as f32).sqrt()
    };
    let strain = strain_breakdown(sk, &posture, &req.weights);
    ReachOutcome {
        pose,
        posture,
        max_error_mm: max_error,
        rms_error_mm: rms,
        strain,
        reached: max_error <= CONTACT_TOLERANCE_MM,
    }
}

fn solve_symmetric(a: &[[f32; DOF]; DOF], b: &[f32; DOF]) -> Option<[f32; DOF]> {
    let mut l = [[0.0f32; DOF]; DOF];
    for i in 0..DOF {
        for j in 0..=i {
            let mut sum = a[i][j];
            for k in 0..j {
                sum -= l[i][k] * l[j][k];
            }
            if i == j {
                if sum <= 1e-12 {
                    return None;
                }
                l[i][j] = sum.sqrt();
            } else {
                l[i][j] = sum / l[j][j];
            }
        }
    }

    let mut y = [0.0f32; DOF];
    for i in 0..DOF {
        let mut sum = b[i];
        for k in 0..i {
            sum -= l[i][k] * y[k];
        }
        y[i] = sum / l[i][i];
    }
    let mut x = [0.0f32; DOF];
    for i in (0..DOF).rev() {
        let mut sum = y[i];
        for k in i + 1..DOF {
            sum -= l[k][i] * x[k];
        }
        x[i] = sum / l[i][i];
    }
    if x.iter().any(|v| !v.is_finite()) {
        return None;
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyboard::Keyboard;
    use crate::profile::{HandProfile, HandSize};
    use crate::Hand;

    fn setup(hand: Hand, size: HandSize) -> (Skeleton, Keyboard) {
        (
            Skeleton::new(HandProfile::from_size(size), hand),
            Keyboard::new(),
        )
    }

    fn five_finger(kb: &Keyboard, hand: Hand, lowest: u8) -> Vec<(Finger, Vec3)> {
        let whites: Vec<u8> = (lowest..lowest + 12)
            .filter(|m| crate::keyboard::is_white(*m))
            .take(5)
            .collect();
        let fingers = match hand {
            Hand::Right => Finger::ALL,
            Hand::Left => [
                Finger::Little,
                Finger::Ring,
                Finger::Middle,
                Finger::Index,
                Finger::Thumb,
            ],
        };
        whites
            .iter()
            .zip(fingers)
            .map(|(m, f)| (f, kb.nominal_strike(*m)))
            .collect()
    }

    #[test]
    fn a_five_finger_position_is_reachable_and_relaxed() {
        for hand in Hand::ALL {
            let (sk, kb) = setup(hand, HandSize::Medium);
            let targets = five_finger(&kb, hand, 60);
            let out = reach(&sk, &ReachRequest::new(&targets));
            assert!(
                out.reached,
                "{hand:?} could not reach a five-finger position: max error {} mm",
                out.max_error_mm
            );
            assert!(
                out.strain_total() < 6.0,
                "{hand:?} five-finger position should be relaxed, cost {}",
                out.strain_total()
            );
        }
    }

    #[test]
    fn an_octave_is_comfortable_a_ninth_is_a_stretch_a_twelfth_is_not_happening() {
        let (sk, kb) = setup(Hand::Right, HandSize::Medium);
        let span = |top: u8| {
            let targets = [
                (Finger::Thumb, kb.nominal_strike(60)),
                (Finger::Little, kb.nominal_strike(top)),
            ];
            reach(&sk, &ReachRequest::new(&targets))
        };

        let octave = span(72);
        let ninth = span(74);
        let tenth = span(76);
        let twelfth = span(79);

        assert!(octave.reached, "an octave must be reachable");
        assert!(ninth.reached, "a ninth must be reachable at a stretch");
        assert!(
            !twelfth.reached,
            "a twelfth should be out of reach for an average hand, error was {} mm",
            twelfth.max_error_mm
        );
        assert!(
            octave.strain_total() < ninth.strain_total(),
            "a ninth should cost more than an octave"
        );
        assert!(
            ninth.strain_total() < tenth.strain_total(),
            "a tenth should cost more than a ninth"
        );
    }

    #[test]
    fn bigger_hands_reach_further() {
        let kb = Keyboard::new();
        let reach_of = |size: HandSize, top: u8| {
            let sk = Skeleton::new(HandProfile::from_size(size), Hand::Right);
            let targets = [
                (Finger::Thumb, kb.nominal_strike(60)),
                (Finger::Little, kb.nominal_strike(top)),
            ];
            reach(&sk, &ReachRequest::new(&targets)).strain_total()
        };
        assert!(reach_of(HandSize::Small, 76) > reach_of(HandSize::ExtraLarge, 76));
    }

    #[test]
    fn the_solver_lands_on_black_keys_too() {
        let (sk, kb) = setup(Hand::Right, HandSize::Medium);
        let notes = [66u8, 68, 70, 71, 73];
        let targets: Vec<_> = notes
            .iter()
            .zip(Finger::ALL)
            .map(|(m, f)| (f, kb.nominal_strike(*m)))
            .collect();
        let out = reach(&sk, &ReachRequest::new(&targets));
        assert!(
            out.reached,
            "black-key shape unreached, max error {} mm",
            out.max_error_mm
        );
    }

    #[test]
    fn a_warm_start_from_the_previous_chord_still_converges() {
        let (sk, kb) = setup(Hand::Right, HandSize::Medium);
        let first: Vec<_> = [60u8, 62, 64, 65, 67]
            .iter()
            .zip(Finger::ALL)
            .map(|(m, f)| (f, kb.nominal_strike(*m)))
            .collect();
        let a = reach(&sk, &ReachRequest::new(&first));

        let second: Vec<_> = [67u8, 69, 71, 72, 74]
            .iter()
            .zip(Finger::ALL)
            .map(|(m, f)| (f, kb.nominal_strike(*m)))
            .collect();
        let b = reach(&sk, &ReachRequest::new(&second).from_pose(a.pose));
        assert!(b.reached, "warm start failed: {} mm", b.max_error_mm);
    }

    #[test]
    fn single_finger_targets_are_always_reachable_across_the_keyboard() {
        let (sk, kb) = setup(Hand::Right, HandSize::Medium);
        for midi in [21u8, 40, 60, 61, 80, 108] {
            for f in Finger::ALL {
                let targets = [(f, kb.nominal_strike(midi))];
                let out = reach(&sk, &ReachRequest::new(&targets));
                assert!(
                    out.reached,
                    "{f:?} could not reach midi {midi}: {} mm off",
                    out.max_error_mm
                );
            }
        }
    }

    #[test]
    fn solved_postures_never_break_a_joint_limit() {
        let (sk, kb) = setup(Hand::Right, HandSize::Small);
        let targets = [
            (Finger::Thumb, kb.nominal_strike(60)),
            (Finger::Little, kb.nominal_strike(77)),
        ];
        let out = reach(&sk, &ReachRequest::new(&targets));
        for i in dof::WRIST_DEVIATION..DOF {
            let v = crate::skeleton::LIMITS[i].violation(out.pose.q[i]);
            assert!(v < 1e-5, "dof {i} out of range by {v} rad");
        }
    }
}
