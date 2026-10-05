use std::f32::consts::PI;

use glam::{Quat, Vec3};

use crate::profile::HandProfile;
use crate::torso::Torso;
use crate::{Finger, Hand};

pub const DIP_PIP_COUPLING: f32 = 0.66;

pub const DOF: usize = 22;

pub mod dof {
    pub const WRIST_X: usize = 0;
    pub const WRIST_Y: usize = 1;
    pub const WRIST_Z: usize = 2;
    pub const WRIST_DEVIATION: usize = 3;
    pub const WRIST_FLEXION: usize = 4;
    pub const WRIST_PRONATION: usize = 5;
    pub const THUMB_CMC_FLEX: usize = 6;
    pub const THUMB_CMC_ABD: usize = 7;
    pub const THUMB_MCP_FLEX: usize = 8;
    pub const THUMB_IP_FLEX: usize = 9;
    pub const FINGER_BASE: usize = 10;
    pub const MCP_FLEX: usize = 0;
    pub const MCP_SPREAD: usize = 1;
    pub const PIP_FLEX: usize = 2;

    pub const fn finger(slot: usize) -> usize {
        FINGER_BASE + slot * 3
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandPose {
    pub q: [f32; DOF],
}

impl HandPose {
    pub fn wrist_position(&self) -> Vec3 {
        Vec3::new(self.q[dof::WRIST_X], self.q[dof::WRIST_Y], self.q[dof::WRIST_Z])
    }

    pub fn set_wrist_position(&mut self, p: Vec3) {
        self.q[dof::WRIST_X] = p.x;
        self.q[dof::WRIST_Y] = p.y;
        self.q[dof::WRIST_Z] = p.z;
    }

    pub fn lerp(&self, other: &HandPose, t: f32) -> HandPose {
        let mut q = self.q;
        for i in 0..DOF {
            q[i] += (other.q[i] - self.q[i]) * t;
        }
        HandPose { q }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limit {
    pub min: f32,
    pub max: f32,
    pub neutral: f32,
}

impl Limit {
    const fn new(min_deg: f32, max_deg: f32, neutral_deg: f32) -> Self {
        Self {
            min: min_deg * PI / 180.0,
            max: max_deg * PI / 180.0,
            neutral: neutral_deg * PI / 180.0,
        }
    }

    pub fn range(&self) -> f32 {
        (self.max - self.min).max(1e-6)
    }

    pub fn clamp(&self, v: f32) -> f32 {
        v.clamp(self.min, self.max)
    }

    pub fn violation(&self, v: f32) -> f32 {
        if v < self.min {
            self.min - v
        } else if v > self.max {
            v - self.max
        } else {
            0.0
        }
    }
}

const WRIST_TRANSLATION_LIMIT: Limit = Limit { min: -1e4, max: 1e4, neutral: 0.0 };

pub const LIMITS: [Limit; DOF] = [
    WRIST_TRANSLATION_LIMIT,
    WRIST_TRANSLATION_LIMIT,
    WRIST_TRANSLATION_LIMIT,
    Limit::new(-30.0, 20.0, 0.0),
    Limit::new(-40.0, 40.0, -5.0),
    Limit::new(-25.0, 25.0, 0.0),
    Limit::new(-35.0, 55.0, 8.0),
    Limit::new(-30.0, 55.0, 16.0),
    Limit::new(-25.0, 60.0, 8.0),
    Limit::new(-15.0, 80.0, 10.0),
    Limit::new(-25.0, 90.0, 22.0),
    Limit::new(-25.0, 25.0, 0.0),
    Limit::new(0.0, 110.0, 32.0),
    Limit::new(-25.0, 90.0, 24.0),
    Limit::new(-18.0, 18.0, 0.0),
    Limit::new(0.0, 110.0, 34.0),
    Limit::new(-25.0, 90.0, 24.0),
    Limit::new(-18.0, 18.0, 0.0),
    Limit::new(0.0, 110.0, 34.0),
    Limit::new(-25.0, 90.0, 22.0),
    Limit::new(-30.0, 30.0, 0.0),
    Limit::new(0.0, 110.0, 32.0),
];

const FINGER_SPLAY_DEG: [f32; 4] = [4.0, 0.0, -4.0, -9.0];

const THUMB_SPLAY_DEG: f32 = 38.0;
const THUMB_TILT_DEG: f32 = 28.0;
const THUMB_ROLL_DEG: f32 = 70.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joint {
    Wrist,
    Base(Finger),
    Middle(Finger),
    Distal(Finger),
    Tip(Finger),
}

#[derive(Debug, Clone)]
pub struct Posture {
    pub pose: HandPose,
    pub wrist: Vec3,
    pub chain: [[Vec3; 4]; 5],
    axes: [(Vec3, Vec3); DOF],
}

impl Posture {
    pub fn joint(&self, j: Joint) -> Vec3 {
        match j {
            Joint::Wrist => self.wrist,
            Joint::Base(f) => self.chain[f.index()][0],
            Joint::Middle(f) => self.chain[f.index()][1],
            Joint::Distal(f) => self.chain[f.index()][2],
            Joint::Tip(f) => self.chain[f.index()][3],
        }
    }

    #[inline]
    pub fn tip(&self, f: Finger) -> Vec3 {
        self.chain[f.index()][3]
    }

    pub fn tip_jacobian(&self, f: Finger, dof_index: usize) -> Vec3 {
        match dof_index {
            dof::WRIST_X => Vec3::X,
            dof::WRIST_Y => Vec3::Y,
            dof::WRIST_Z => Vec3::Z,
            _ => {
                if !self.dof_affects(f, dof_index) {
                    return Vec3::ZERO;
                }
                let (axis, origin) = self.axes[dof_index];
                axis.cross(self.tip(f) - origin)
            }
        }
    }

    fn dof_affects(&self, f: Finger, dof_index: usize) -> bool {
        if dof_index <= dof::WRIST_PRONATION {
            return true;
        }
        match f {
            Finger::Thumb => (dof::THUMB_CMC_FLEX..=dof::THUMB_IP_FLEX).contains(&dof_index),
            other => {
                let base = dof::finger(other.index() - 1);
                (base..base + 3).contains(&dof_index)
            }
        }
    }
}

fn keyboard_centre_x() -> f32 {
    let keyboard = crate::keyboard::Keyboard::new();
    (keyboard.centre_x(crate::keyboard::MIDI_LOWEST)
        + keyboard.centre_x(crate::keyboard::MIDI_HIGHEST))
        / 2.0
}

#[derive(Debug, Clone)]
pub struct Skeleton {
    profile: HandProfile,
    hand: Hand,
    sign: f32,
    torso: Torso,
}

impl Skeleton {
    pub fn new(profile: HandProfile, hand: Hand) -> Self {
        let sign = hand.sign();
        let torso = Torso::new(&profile, keyboard_centre_x());
        Self { profile, hand, sign, torso }
    }

    pub fn torso(&self) -> &Torso {
        &self.torso
    }

    pub fn wrist_neutral(&self, pose: &HandPose) -> f32 {
        let wrist = pose.wrist_position();
        let lean = self.torso.lean_for(self.hand, wrist);
        let shoulder = self.torso.shoulder(self.hand, lean);
        let along = wrist - shoulder;
        let bearing = along.x.atan2(along.y.max(1.0));
        -self.sign * bearing
    }

    pub fn profile(&self) -> &HandProfile {
        &self.profile
    }

    pub fn hand(&self) -> Hand {
        self.hand
    }

    fn neutral_rotation(&self) -> Quat {
        match self.hand {
            Hand::Right => Quat::from_rotation_y(PI),
            Hand::Left => Quat::IDENTITY,
        }
    }

    pub fn rest_pose(&self, wrist: Vec3) -> HandPose {
        let mut q = [0.0f32; DOF];
        for i in 0..DOF {
            q[i] = LIMITS[i].neutral;
        }
        let mut pose = HandPose { q };
        pose.set_wrist_position(wrist);
        pose
    }

    pub fn clamp(&self, pose: &mut HandPose) {
        let wrist_neutral = self.wrist_neutral(pose);
        for i in dof::WRIST_DEVIATION..DOF {
            pose.q[i] = if i == dof::WRIST_DEVIATION {
                LIMITS[i].clamp(pose.q[i] - wrist_neutral) + wrist_neutral
            } else {
                LIMITS[i].clamp(pose.q[i])
            };
        }
    }

    fn base_offset(&self, finger: Finger) -> Vec3 {
        let (x, y, z) = self.profile.base_offset(finger);
        Vec3::new(x, y, z * self.sign)
    }

    pub fn wrist_rotation(&self, pose: &HandPose) -> Quat {
        let s = self.sign;
        self.neutral_rotation()
            * Quat::from_rotation_z(-pose.q[dof::WRIST_DEVIATION])
            * Quat::from_rotation_x(s * pose.q[dof::WRIST_FLEXION])
            * Quat::from_rotation_y(s * pose.q[dof::WRIST_PRONATION])
    }

    pub fn forward(&self, pose: &HandPose) -> Posture {
        let s = self.sign;
        let wrist = pose.wrist_position();
        let wrist_rot = self.wrist_rotation(pose);

        let mut axes = [(Vec3::ZERO, Vec3::ZERO); DOF];
        {
            let r0 = self.neutral_rotation();
            let r1 = r0 * Quat::from_rotation_z(-pose.q[dof::WRIST_DEVIATION]);
            let r2 = r1 * Quat::from_rotation_x(s * pose.q[dof::WRIST_FLEXION]);
            axes[dof::WRIST_DEVIATION] = (r0 * -Vec3::Z, wrist);
            axes[dof::WRIST_FLEXION] = (r1 * (Vec3::X * s), wrist);
            axes[dof::WRIST_PRONATION] = (r2 * (Vec3::Y * s), wrist);
        }

        let to_world = |local: Vec3| wrist + wrist_rot * local;
        let mut chain = [[Vec3::ZERO; 4]; 5];

        {
            let d = *self.profile.digit(Finger::Thumb);
            let base_local = self.base_offset(Finger::Thumb);
            let frame = Quat::from_rotation_z(-THUMB_SPLAY_DEG.to_radians())
                * Quat::from_rotation_x(s * THUMB_TILT_DEG.to_radians())
                * Quat::from_rotation_y(s * THUMB_ROLL_DEG.to_radians());

            let r_cmc = frame
                * Quat::from_rotation_z(-pose.q[dof::THUMB_CMC_ABD])
                * Quat::from_rotation_x(s * pose.q[dof::THUMB_CMC_FLEX]);
            let r_mcp = r_cmc * Quat::from_rotation_x(s * pose.q[dof::THUMB_MCP_FLEX]);
            let r_ip = r_mcp * Quat::from_rotation_x(s * pose.q[dof::THUMB_IP_FLEX]);

            let p_cmc = base_local;
            let p_mcp = p_cmc + r_cmc * (Vec3::Y * d.metacarpal);
            let p_ip = p_mcp + r_mcp * (Vec3::Y * d.proximal);
            let p_tip = p_ip + r_ip * (Vec3::Y * (d.distal + d.pulp));

            chain[Finger::Thumb.index()] = [
                to_world(p_cmc),
                to_world(p_mcp),
                to_world(p_ip),
                to_world(p_tip),
            ];

            let w = |r: Quat, v: Vec3| wrist_rot * (r * v);
            axes[dof::THUMB_CMC_ABD] = (w(frame, -Vec3::Z), to_world(p_cmc));
            axes[dof::THUMB_CMC_FLEX] = (
                w(frame * Quat::from_rotation_z(-pose.q[dof::THUMB_CMC_ABD]), Vec3::X * s),
                to_world(p_cmc),
            );
            axes[dof::THUMB_MCP_FLEX] = (w(r_cmc, Vec3::X * s), to_world(p_mcp));
            axes[dof::THUMB_IP_FLEX] = (w(r_mcp, Vec3::X * s), to_world(p_ip));
        }

        for slot in 0..4 {
            let finger = Finger::from_index(slot + 1);
            let d = *self.profile.digit(finger);
            let base_local = self.base_offset(finger);
            let b = dof::finger(slot);

            let splay = FINGER_SPLAY_DEG[slot].to_radians();
            let spread = pose.q[b + dof::MCP_SPREAD];
            let mcp_flex = pose.q[b + dof::MCP_FLEX];
            let pip_flex = pose.q[b + dof::PIP_FLEX];
            let dip_flex = pip_flex * DIP_PIP_COUPLING;

            let r_spread = Quat::from_rotation_z(-(splay + spread));
            let r_mcp = r_spread * Quat::from_rotation_x(s * mcp_flex);
            let r_pip = r_mcp * Quat::from_rotation_x(s * pip_flex);
            let r_dip = r_pip * Quat::from_rotation_x(s * dip_flex);

            let p_mcp = base_local;
            let p_pip = p_mcp + r_mcp * (Vec3::Y * d.proximal);
            let p_dip = p_pip + r_pip * (Vec3::Y * d.medial);
            let p_tip = p_dip + r_dip * (Vec3::Y * (d.distal + d.pulp));

            chain[finger.index()] = [
                to_world(p_mcp),
                to_world(p_pip),
                to_world(p_dip),
                to_world(p_tip),
            ];

            let w = |r: Quat, v: Vec3| wrist_rot * (r * v);
            axes[b + dof::MCP_SPREAD] = (w(Quat::IDENTITY, -Vec3::Z), to_world(p_mcp));
            axes[b + dof::MCP_FLEX] = (w(r_spread, Vec3::X * s), to_world(p_mcp));
            axes[b + dof::PIP_FLEX] = (w(r_mcp, Vec3::X * s), to_world(p_pip));
        }

        Posture { pose: *pose, wrist, chain, axes }
    }

    pub fn tip_jacobian(&self, posture: &Posture, finger: Finger, dof_index: usize) -> Vec3 {
        let base = posture.tip_jacobian(finger, dof_index);
        if finger == Finger::Thumb {
            return base;
        }
        let slot = finger.index() - 1;
        if dof_index != dof::finger(slot) + dof::PIP_FLEX {
            return base;
        }
        let (axis, _) = posture.axes[dof_index];
        let dip_origin = posture.chain[finger.index()][2];
        base + DIP_PIP_COUPLING * axis.cross(posture.tip(finger) - dip_origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::HandProfile;

    fn right() -> Skeleton {
        Skeleton::new(HandProfile::default(), Hand::Right)
    }

    fn left() -> Skeleton {
        Skeleton::new(HandProfile::default(), Hand::Left)
    }

    #[test]
    fn rest_pose_respects_every_limit() {
        let sk = right();
        let pose = sk.rest_pose(Vec3::ZERO);
        for i in dof::WRIST_DEVIATION..DOF {
            assert_eq!(LIMITS[i].violation(pose.q[i]), 0.0, "dof {i} starts out of range");
        }
    }

    #[test]
    fn fingers_point_into_the_keyboard_and_curl_downward() {
        for sk in [right(), left()] {
            let posture = sk.forward(&sk.rest_pose(Vec3::new(0.0, 0.0, 100.0)));
            for f in [Finger::Index, Finger::Middle, Finger::Ring, Finger::Little] {
                let tip = posture.tip(f);
                let knuckle = posture.joint(Joint::Base(f));
                assert!(tip.y > knuckle.y, "{:?} {f:?} tip should be further in", sk.hand());
                assert!(tip.z < knuckle.z, "{:?} {f:?} tip should curl below the knuckle", sk.hand());
            }
        }
    }

    #[test]
    fn the_thumb_sits_toward_the_low_notes_for_the_right_hand() {
        let sk = right();
        let posture = sk.forward(&sk.rest_pose(Vec3::new(500.0, 0.0, 100.0)));
        let thumb = posture.tip(Finger::Thumb);
        let little = posture.tip(Finger::Little);
        assert!(thumb.x < little.x, "right thumb should be left of the little finger");

        let sk = left();
        let posture = sk.forward(&sk.rest_pose(Vec3::new(500.0, 0.0, 100.0)));
        let thumb = posture.tip(Finger::Thumb);
        let little = posture.tip(Finger::Little);
        assert!(thumb.x > little.x, "left thumb should be right of the little finger");
    }

    #[test]
    fn the_two_hands_are_mirror_images() {
        let (r, l) = (right(), left());
        let pose = r.rest_pose(Vec3::ZERO);
        let (pr, pl) = (r.forward(&pose), l.forward(&pose));
        for f in Finger::ALL {
            let a = pr.tip(f);
            let b = pl.tip(f);
            assert!(
                (a.x + b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3 && (a.z - b.z).abs() < 1e-3,
                "{f:?} right {a:?} is not the mirror of left {b:?}"
            );
        }
    }

    #[test]
    fn bone_lengths_survive_the_kinematic_chain() {
        let sk = right();
        let mut pose = sk.rest_pose(Vec3::new(300.0, 40.0, 90.0));
        for i in dof::WRIST_DEVIATION..DOF {
            pose.q[i] = LIMITS[i].neutral + LIMITS[i].range() * 0.2;
        }
        sk.clamp(&mut pose);
        let p = sk.forward(&pose);

        for f in Finger::ALL {
            let d = sk.profile().digit(f);
            let c = &p.chain[f.index()];
            let expected = if f == Finger::Thumb {
                [d.metacarpal, d.proximal, d.distal + d.pulp]
            } else {
                [d.proximal, d.medial, d.distal + d.pulp]
            };
            for (i, want) in expected.iter().enumerate() {
                let got = (c[i + 1] - c[i]).length();
                assert!((got - want).abs() < 1e-2, "{f:?} bone {i}: {got} vs {want}");
            }
        }
    }

    #[test]
    fn analytic_jacobian_matches_finite_differences() {
        let sk = right();
        let mut pose = sk.rest_pose(Vec3::new(400.0, 30.0, 95.0));
        for (i, item) in pose.q.iter_mut().enumerate().skip(dof::WRIST_DEVIATION) {
            *item += LIMITS[i].range() * 0.13;
        }
        let posture = sk.forward(&pose);

        for f in Finger::ALL {
            for i in 0..DOF {
                let h = if i <= dof::WRIST_Z { 0.05 } else { 1e-3 };
                let mut plus = pose;
                plus.q[i] += h;
                let mut minus = pose;
                minus.q[i] -= h;
                let numeric = (sk.forward(&plus).tip(f) - sk.forward(&minus).tip(f)) / (2.0 * h);
                let analytic = sk.tip_jacobian(&posture, f, i);
                let err = (numeric - analytic).length();
                assert!(
                    err < 1e-2 * numeric.length().max(1.0),
                    "{f:?} dof {i}: analytic {analytic:?} vs numeric {numeric:?}"
                );
            }
        }
    }

    #[test]
    fn the_wrist_rotation_puts_the_palm_down_and_the_fingers_forward() {
        for sk in [right(), left()] {
            let rotation = sk.wrist_rotation(&sk.rest_pose(Vec3::ZERO));
            let distal = rotation * Vec3::Y;
            assert!(distal.y > 0.9, "{:?} fingers point {distal:?}", sk.hand());
            let palmar = rotation * (Vec3::Z * sk.hand().sign());
            assert!(palmar.z < -0.9, "{:?} palm faces {palmar:?}", sk.hand());
        }
    }

    #[test]
    fn wrist_translation_moves_the_whole_hand_rigidly() {
        let sk = right();
        let a = sk.forward(&sk.rest_pose(Vec3::new(100.0, 0.0, 90.0)));
        let b = sk.forward(&sk.rest_pose(Vec3::new(264.0, 0.0, 90.0)));
        for f in Finger::ALL {
            let d = b.tip(f) - a.tip(f);
            assert!((d - Vec3::new(164.0, 0.0, 0.0)).length() < 1e-3, "{f:?} moved {d:?}");
        }
    }
}
