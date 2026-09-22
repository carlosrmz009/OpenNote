//! Forward kinematics of the hand.
//!
//! # Frames and chirality
//!
//! Each hand has its own local frame anchored at the wrist pivot:
//!
//! * `x` — radial, i.e. toward the thumb
//! * `y` — distal, i.e. toward the fingertips
//! * `z` — `x × y`
//!
//! Because both frames are right-handed and hands are mirror images, `z` comes out
//! *palmar* for the right hand and *dorsal* for the left. So the two hands are
//! related by a flip of `z` alone, and that single fact is the whole of the
//! chirality handling:
//!
//! * quantities measured along `z` — the thumb's palmar offset — scale by
//!   [`Hand::sign`]
//! * rotations about `x` (flexion) and `y` (pronation) change sign with it, since
//!   a `z` flip reverses them
//! * rotations about `z` (spread, wrist deviation) keep the same sign for both
//!   hands, because a `z` flip leaves them alone
//!
//! The local frame then maps into world space with a proper rotation for either
//! hand, so rotation axes stay honest vectors and the analytic Jacobian is exact.
//!
//! # Degrees of freedom
//!
//! 22 per hand: 6 for wrist placement, 4 for the thumb, 3 for each finger. The
//! distal interphalangeal joints are not free — they are driven from the PIP by
//! the Landsmeer coupling, which is what real fingers do and what stops the solver
//! from producing poses no tendon could hold.

use std::f32::consts::PI;

use glam::{Quat, Vec3};

use crate::profile::HandProfile;
use crate::torso::Torso;
use crate::{Finger, Hand};

/// Ratio by which the DIP joint follows the PIP joint.
///
/// The flexor digitorum profundus crosses both joints and the oblique retinacular
/// ligament links them, so the distal joint is not independently controllable in
/// normal use. Landsmeer's classic figure is about two thirds.
pub const DIP_PIP_COUPLING: f32 = 0.66;

/// Number of free joint variables in a [`HandPose`].
pub const DOF: usize = 22;

/// Index of each degree of freedom in the flat parameter vector.
pub mod dof {
    /// Wrist position along the keyboard.
    pub const WRIST_X: usize = 0;
    /// Wrist position into the keyboard.
    pub const WRIST_Y: usize = 1;
    /// Wrist height above the keys.
    pub const WRIST_Z: usize = 2;
    /// Radial (+) / ulnar (-) deviation.
    pub const WRIST_DEVIATION: usize = 3;
    /// Flexion (+) / extension (-).
    pub const WRIST_FLEXION: usize = 4;
    /// Pronation (+) / supination (-) of the forearm.
    pub const WRIST_PRONATION: usize = 5;
    /// Thumb carpometacarpal flexion.
    pub const THUMB_CMC_FLEX: usize = 6;
    /// Thumb carpometacarpal palmar abduction.
    pub const THUMB_CMC_ABD: usize = 7;
    /// Thumb metacarpophalangeal flexion.
    pub const THUMB_MCP_FLEX: usize = 8;
    /// Thumb interphalangeal flexion.
    pub const THUMB_IP_FLEX: usize = 9;
    /// First degree of freedom of the index finger; fingers are three apart.
    pub const FINGER_BASE: usize = 10;
    /// Offset of metacarpophalangeal flexion within a finger's block.
    pub const MCP_FLEX: usize = 0;
    /// Offset of metacarpophalangeal spread within a finger's block.
    pub const MCP_SPREAD: usize = 1;
    /// Offset of proximal interphalangeal flexion within a finger's block.
    pub const PIP_FLEX: usize = 2;

    /// First parameter index for one of the four fingers (index..little).
    pub const fn finger(slot: usize) -> usize {
        FINGER_BASE + slot * 3
    }
}

/// A joint configuration: 22 angles and positions, in radians and millimetres.
///
/// Element 0..3 are the wrist translation in world millimetres; everything else is
/// an anatomical angle in radians, always signed so that positive means flexion,
/// radial deviation, or abduction away from the palm, for *either* hand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandPose {
    /// The flat parameter vector. See [`dof`] for the layout.
    pub q: [f32; DOF],
}

impl HandPose {
    /// Wrist position in world millimetres.
    pub fn wrist_position(&self) -> Vec3 {
        Vec3::new(self.q[dof::WRIST_X], self.q[dof::WRIST_Y], self.q[dof::WRIST_Z])
    }

    /// Set the wrist position in world millimetres.
    pub fn set_wrist_position(&mut self, p: Vec3) {
        self.q[dof::WRIST_X] = p.x;
        self.q[dof::WRIST_Y] = p.y;
        self.q[dof::WRIST_Z] = p.z;
    }

    /// Linear blend between two poses, used for animation and for warm starts.
    pub fn lerp(&self, other: &HandPose, t: f32) -> HandPose {
        let mut q = self.q;
        for i in 0..DOF {
            q[i] += (other.q[i] - self.q[i]) * t;
        }
        HandPose { q }
    }
}

/// Inclusive bounds on one degree of freedom.
#[derive(Debug, Clone, Copy)]
pub struct Limit {
    /// Lower bound.
    pub min: f32,
    /// Upper bound.
    pub max: f32,
    /// The value the joint relaxes to when nothing is asking anything of it.
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

    /// Free range of the joint, used to normalise deviations across joints of very
    /// different mobility.
    pub fn range(&self) -> f32 {
        (self.max - self.min).max(1e-6)
    }

    /// Clamp a value into the joint's range.
    pub fn clamp(&self, v: f32) -> f32 {
        v.clamp(self.min, self.max)
    }

    /// How far outside its range a value sits, in the same units.
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

/// Wrist translation limits are in millimetres, so they are stored separately from
/// the angular ones.
const WRIST_TRANSLATION_LIMIT: Limit = Limit { min: -1e4, max: 1e4, neutral: 0.0 };

/// Range of motion and resting angle for every degree of freedom.
///
/// Ranges are the functional ranges used at the keyboard rather than the maximum a
/// joint can be forced to; the neutral values describe a hand hovering in playing
/// position, fingers gently curved, wrist level and forearm pronated.
pub const LIMITS: [Limit; DOF] = [
    WRIST_TRANSLATION_LIMIT,
    WRIST_TRANSLATION_LIMIT,
    WRIST_TRANSLATION_LIMIT,
    Limit::new(-30.0, 20.0, 0.0),  // deviation: ulnar reaches further than radial
    Limit::new(-40.0, 40.0, -5.0), // flexion: slight extension is the playing neutral
    Limit::new(-25.0, 25.0, 0.0),  // pronation about the forearm axis
    // The trapeziometacarpal joint extends a long way; the tight ranges quoted for
    // it usually describe flexion from a neutral the hand rarely sits at.
    Limit::new(-35.0, 55.0, 8.0),  // thumb CMC flexion
    // Adduction is a large motion: the thumb can be brought right across the palm,
    // and at the keyboard it lives much closer to the hand than its resting fan.
    Limit::new(-30.0, 55.0, 16.0), // thumb CMC palmar abduction
    Limit::new(-25.0, 60.0, 8.0),  // thumb MCP flexion
    Limit::new(-15.0, 80.0, 10.0), // thumb IP flexion
    Limit::new(-25.0, 90.0, 22.0), // index MCP flexion
    Limit::new(-25.0, 25.0, 0.0),  // index spread
    Limit::new(0.0, 110.0, 32.0),  // index PIP flexion
    Limit::new(-25.0, 90.0, 24.0), // middle MCP flexion
    Limit::new(-18.0, 18.0, 0.0),  // middle spread
    Limit::new(0.0, 110.0, 34.0),  // middle PIP flexion
    Limit::new(-25.0, 90.0, 24.0), // ring MCP flexion
    Limit::new(-18.0, 18.0, 0.0),  // ring spread
    Limit::new(0.0, 110.0, 34.0),  // ring PIP flexion
    Limit::new(-25.0, 90.0, 22.0), // little MCP flexion
    Limit::new(-30.0, 30.0, 0.0),  // little spread: the most mobile of the four
    Limit::new(0.0, 110.0, 32.0),  // little PIP flexion
];

/// Resting fan of the fingers, in degrees, positive toward the thumb. The fingers
/// converge slightly rather than running parallel.
const FINGER_SPLAY_DEG: [f32; 4] = [4.0, 0.0, -4.0, -9.0];

/// Orientation of the thumb's plane of motion relative to the palm, in degrees:
/// the metacarpal fans radially, tilts palmar, and is rolled about its own axis.
/// That roll is opposition — it is why the thumb pad can face the fingertips.
const THUMB_SPLAY_DEG: f32 = 38.0;
const THUMB_TILT_DEG: f32 = 28.0;
const THUMB_ROLL_DEG: f32 = 70.0;

/// A named joint in the kinematic chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joint {
    /// Wrist pivot.
    Wrist,
    /// Knuckle: metacarpophalangeal for fingers, carpometacarpal for the thumb.
    Base(Finger),
    /// Proximal interphalangeal; for the thumb, metacarpophalangeal.
    Middle(Finger),
    /// Distal interphalangeal; for the thumb, interphalangeal.
    Distal(Finger),
    /// The fleshy fingertip that touches the key.
    Tip(Finger),
}

/// A solved hand: every joint placed in world space.
#[derive(Debug, Clone)]
pub struct Posture {
    /// The pose that produced it.
    pub pose: HandPose,
    /// Wrist pivot in world millimetres.
    pub wrist: Vec3,
    /// World positions of each digit's four joints plus its tip, in chain order:
    /// base, middle, distal, tip.
    pub chain: [[Vec3; 4]; 5],
    /// World rotation axis of every free angular degree of freedom, and the world
    /// point it rotates about. Translation degrees of freedom store a zero axis.
    axes: [(Vec3, Vec3); DOF],
}

impl Posture {
    /// World position of a named joint.
    pub fn joint(&self, j: Joint) -> Vec3 {
        match j {
            Joint::Wrist => self.wrist,
            Joint::Base(f) => self.chain[f.index()][0],
            Joint::Middle(f) => self.chain[f.index()][1],
            Joint::Distal(f) => self.chain[f.index()][2],
            Joint::Tip(f) => self.chain[f.index()][3],
        }
    }

    /// World position of a fingertip.
    #[inline]
    pub fn tip(&self, f: Finger) -> Vec3 {
        self.chain[f.index()][3]
    }

    /// Partial derivative of a fingertip's world position with respect to one
    /// degree of freedom.
    ///
    /// For a revolute joint this is the textbook `axis × (tip - joint)`; for the
    /// three wrist translations it is a basis vector. Exact, and one forward pass
    /// of kinematics supplies everything it needs.
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

    /// Whether a degree of freedom can move a given fingertip at all. Wrist degrees
    /// of freedom move everything; a finger's own joints move only that finger.
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


/// The middle of an eighty-eight key keyboard, in millimetres from the left edge.
///
/// Where the player sits. Derived from the key geometry rather than written down, so
/// the two cannot drift apart.
fn keyboard_centre_x() -> f32 {
    let keyboard = crate::keyboard::Keyboard::new();
    (keyboard.centre_x(crate::keyboard::MIDI_LOWEST)
        + keyboard.centre_x(crate::keyboard::MIDI_HIGHEST))
        / 2.0
}

/// A hand's kinematics: a [`HandProfile`] bound to a left or right chirality.
#[derive(Debug, Clone)]
pub struct Skeleton {
    profile: HandProfile,
    hand: Hand,
    /// `+1` for the right hand, `-1` for the left. Multiplies every quantity whose
    /// sign is defined relative to the palm.
    sign: f32,
    /// The player this hand belongs to, which is what decides which way the forearm
    /// points and therefore where the wrist's neutral is. See [`Self::wrist_neutral`].
    torso: Torso,
}

impl Skeleton {
    /// Bind a hand profile to a chirality.
    pub fn new(profile: HandProfile, hand: Hand) -> Self {
        let sign = hand.sign();
        let torso = Torso::new(&profile, keyboard_centre_x());
        Self { profile, hand, sign, torso }
    }

    /// The player this hand hangs from.
    pub fn torso(&self) -> &Torso {
        &self.torso
    }

    /// Where the wrist's deviation sits when the hand is doing nothing to it, given
    /// where the arm is coming from.
    ///
    /// Radial and ulnar deviation are angles between the hand and the *forearm*, not
    /// between the hand and the keyboard. That distinction does nothing at all when the
    /// wrist is directly in front of its own shoulder, which is why it went unnoticed:
    /// there the forearm already points along the keys and a hand square to them is a
    /// straight wrist. Away from that spot the forearm comes in at an angle — a right
    /// hand down at the bottom of the keyboard is reaching right across the player —
    /// and holding the hand square to the keys is then a real ulnar deviation of thirty
    /// degrees or so, which is the whole of the joint's range.
    ///
    /// Charging deviation from a fixed zero says a hand is equally comfortable
    /// everywhere along the keyboard, which is the one thing every pianist knows to be
    /// false. Charging it from here is what finally makes [`Torso`] worth having: until
    /// now the player it describes was drawn but never consulted.
    ///
    /// Returned in the same units and sign as `q[dof::WRIST_DEVIATION]`, so it can be
    /// subtracted from it directly.
    pub fn wrist_neutral(&self, pose: &HandPose) -> f32 {
        let wrist = pose.wrist_position();
        let lean = self.torso.lean_for(self.hand, wrist);
        let shoulder = self.torso.shoulder(self.hand, lean);
        let along = wrist - shoulder;
        // The forearm's bearing in the plane of the keyboard, measured from straight
        // away from the player. `y` runs towards the player, so the forearm reaches out
        // along +y and this is well conditioned.
        let bearing = along.x.atan2(along.y.max(1.0));
        // Measured, not derived: turning the hand by one radian of deviation swings its
        // long axis by one radian the other way, and the other way again for the other
        // hand. See the `yawprobe` example.
        -self.sign * bearing
    }

    /// The anthropometry this skeleton was built from.
    pub fn profile(&self) -> &HandProfile {
        &self.profile
    }

    /// Which hand this is.
    pub fn hand(&self) -> Hand {
        self.hand
    }

    /// Rotation taking the hand's local frame into world space with the wrist at
    /// its neutral orientation: fingers pointing into the keyboard, palm down.
    fn neutral_rotation(&self) -> Quat {
        match self.hand {
            // Mapping local (radial, distal, palmar) onto world (-x, +y, -z).
            Hand::Right => Quat::from_rotation_y(PI),
            // Mapping local (radial, distal, dorsal) onto world (+x, +y, +z).
            Hand::Left => Quat::IDENTITY,
        }
    }

    /// The pose a hand relaxes into, with the wrist at the given world position.
    pub fn rest_pose(&self, wrist: Vec3) -> HandPose {
        let mut q = [0.0f32; DOF];
        for i in 0..DOF {
            q[i] = LIMITS[i].neutral;
        }
        let mut pose = HandPose { q };
        pose.set_wrist_position(wrist);
        pose
    }

    /// Clamp every angular degree of freedom into its anatomical range.
    pub fn clamp(&self, pose: &mut HandPose) {
        // The wrist's window travels with the forearm: what the joint can do is
        // measured from wherever the arm is pointing, not from the keyboard.
        let wrist_neutral = self.wrist_neutral(pose);
        for i in dof::WRIST_DEVIATION..DOF {
            pose.q[i] = if i == dof::WRIST_DEVIATION {
                LIMITS[i].clamp(pose.q[i] - wrist_neutral) + wrist_neutral
            } else {
                LIMITS[i].clamp(pose.q[i])
            };
        }
    }

    /// Local offset of a digit's base joint, with palmar depth signed for this hand.
    fn base_offset(&self, finger: Finger) -> Vec3 {
        let (x, y, z) = self.profile.base_offset(finger);
        Vec3::new(x, y, z * self.sign)
    }

    /// Rotation of the wrist relative to the world.
    ///
    /// Public because the visualizer needs it: the hand model it loads is a rigid
    /// mesh hanging off this frame, so the whole hand is oriented by this one
    /// rotation and only the fingers are driven bone by bone.
    pub fn wrist_rotation(&self, pose: &HandPose) -> Quat {
        let s = self.sign;
        self.neutral_rotation()
            * Quat::from_rotation_z(-pose.q[dof::WRIST_DEVIATION])
            * Quat::from_rotation_x(s * pose.q[dof::WRIST_FLEXION])
            * Quat::from_rotation_y(s * pose.q[dof::WRIST_PRONATION])
    }

    /// Place every joint of the hand in world space.
    ///
    /// One pass produces both the geometry and the rotation axes the Jacobian needs.
    pub fn forward(&self, pose: &HandPose) -> Posture {
        let s = self.sign;
        let wrist = pose.wrist_position();
        let wrist_rot = self.wrist_rotation(pose);

        let mut axes = [(Vec3::ZERO, Vec3::ZERO); DOF];
        // The three wrist rotations all pivot about the wrist itself. Their axes are
        // the partially-composed local frames, which is what makes them independent.
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

        // Thumb: carpometacarpal, metacarpophalangeal, interphalangeal, tip.
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

        // Fingers: metacarpophalangeal, proximal interphalangeal, distal
        // interphalangeal, tip. The DIP angle is driven from the PIP.
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
            // The PIP degree of freedom also drags the coupled DIP, so its effect on
            // the tip is the sum of both rotations. Both share an axis direction.
            axes[b + dof::PIP_FLEX] = (w(r_mcp, Vec3::X * s), to_world(p_pip));
        }

        Posture { pose: *pose, wrist, chain, axes }
    }

    /// Tip Jacobian corrected for the DIP coupling.
    ///
    /// Moving the PIP joint also moves the DIP by [`DIP_PIP_COUPLING`], so the true
    /// derivative is the sum of the two contributions. Ignoring the second term is
    /// the usual reason a coupled-finger IK creeps rather than converges.
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
        // Bend everything to a non-trivial configuration first.
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
        // Perturb off the neutral pose so no derivative is accidentally zero.
        for (i, item) in pose.q.iter_mut().enumerate().skip(dof::WRIST_DEVIATION) {
            *item += LIMITS[i].range() * 0.13;
        }
        let posture = sk.forward(&pose);

        for f in Finger::ALL {
            for i in 0..DOF {
                // The wrist translations are stored in millimetres and sit hundreds
                // of millimetres from the origin, where an f32 step of 1e-4 is only
                // a few units in the last place. Step those coarsely.
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
            // Local +y is distal, so it should point into the keyboard.
            let distal = rotation * Vec3::Y;
            assert!(distal.y > 0.9, "{:?} fingers point {distal:?}", sk.hand());
            // Local +z is palmar for the right hand and dorsal for the left, so the
            // palm faces down either way.
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
