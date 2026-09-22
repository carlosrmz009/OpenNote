//! Matching a downloaded hand model's skeleton to ours.
//!
//! The hands are not modelled here — they are a premade, openly licensed, rigged
//! glTF that the user supplies (see `assets/hands/README.md`). Which means the rig
//! could be named almost anything, so this module identifies bones by name against
//! the conventions the common sources actually use, and then drives them by
//! direction rather than by copying joint angles.
//!
//! Driving by direction is what makes the model asset-agnostic. Our skeleton and the
//! downloaded one will not share a rest pose, an axis convention, or bone lengths,
//! so copying angles across would produce nonsense. But both have a bone running
//! from one joint to the next, and pointing that bone the way ours points is
//! well-defined whatever the rig's own conventions are.

use on_hand::{Finger, Hand};
use glam::{Quat, Vec3};

/// Which joint of which digit a bone drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoneRole {
    /// The hand root, which the whole rig hangs from.
    Wrist,
    /// The bone from the knuckle to the next joint. For the thumb this is the
    /// metacarpal, which is why the thumb has one more segment in the palm than the
    /// fingers do.
    Proximal(Finger),
    /// The middle segment.
    Intermediate(Finger),
    /// The last segment, ending at the fingertip.
    Distal(Finger),
}

/// A bone found in a rig.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RigBone {
    /// Which hand it belongs to.
    pub hand: Hand,
    /// What it drives.
    pub role: BoneRole,
}

/// Strip the decorations rig exporters add, leaving something comparable.
fn normalise(name: &str) -> String {
    let name = name.rsplit(':').next().unwrap_or(name); // "mixamorig:LeftHand"
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Which hand a bone name says it belongs to.
fn hand_of(normalised: &str) -> Option<Hand> {
    // Suffix conventions first: "hand.L" normalises to "handl", and a trailing l or r
    // is only meaningful when something precedes it.
    if normalised.starts_with("left") {
        return Some(Hand::Left);
    }
    if normalised.starts_with("right") {
        return Some(Hand::Right);
    }
    match normalised.chars().last()? {
        'l' if normalised.len() > 1 => Some(Hand::Left),
        'r' if normalised.len() > 1 => Some(Hand::Right),
        _ => None,
    }
}

/// The digit a name mentions, if any.
fn finger_of(normalised: &str) -> Option<Finger> {
    const NAMES: [(&str, Finger); 10] = [
        ("thumb", Finger::Thumb),
        ("index", Finger::Index),
        ("middle", Finger::Middle),
        ("ring", Finger::Ring),
        ("little", Finger::Little),
        ("pinky", Finger::Little),
        ("pinkie", Finger::Little),
        ("finger1", Finger::Thumb),
        ("finger2", Finger::Index),
        ("finger3", Finger::Middle),
    ];
    for (needle, finger) in NAMES {
        if normalised.contains(needle) {
            return Some(finger);
        }
    }
    // MakeHuman numbers its digits, and 4 and 5 would otherwise be missed.
    if normalised.contains("finger4") {
        return Some(Finger::Ring);
    }
    if normalised.contains("finger5") {
        return Some(Finger::Little);
    }
    None
}

/// The segment number a name carries, counting from the palm outward.
///
/// Rigs disagree about whether to start at 1 or 0, and about whether to say
/// "proximal" or "01", so both forms are read.
fn segment_of(normalised: &str) -> Option<u8> {
    if normalised.contains("proximal") || normalised.contains("metacarpal") {
        return Some(1);
    }
    if normalised.contains("intermediate") || normalised.contains("middlephalanx") {
        return Some(2);
    }
    if normalised.contains("distal") {
        return Some(3);
    }
    // Trailing digits: "f_index.02.L" becomes "findex02l", "LeftHandThumb1" becomes
    // "lefthandthumb1". Take the last run of digits that is not part of a finger
    // number like "finger3".
    let bytes: Vec<char> = normalised.chars().collect();
    let mut index = bytes.len();
    while index > 0 && !bytes[index - 1].is_ascii_digit() {
        index -= 1;
    }
    if index == 0 {
        return None;
    }
    let end = index;
    let mut start = end;
    while start > 0 && bytes[start - 1].is_ascii_digit() {
        start -= 1;
    }
    // "finger1-2" normalises to "finger12"; the digit we want is the last one.
    let digits: String = bytes[start..end].iter().collect();
    let value: u32 = digits.parse().ok()?;
    let value = if digits.len() > 1 { value % 10 } else { value };
    (1..=3).contains(&value).then_some(value as u8)
}

/// Identify what a bone in a rig drives, from its name.
///
/// Returns `None` for bones that are not part of a hand — the rest of a body, helper
/// bones, and anything else the file happens to contain.
pub fn classify(name: &str) -> Option<RigBone> {
    let normalised = normalise(name);
    let hand = hand_of(&normalised)?;

    let is_hand_root = matches!(
        normalised.as_str(),
        "handl" | "handr" | "lefthand" | "righthand" | "wristl" | "wristr"
    ) || normalised == "hand"
        || normalised.ends_with("hand");

    let Some(finger) = finger_of(&normalised) else {
        return is_hand_root.then_some(RigBone { hand, role: BoneRole::Wrist });
    };

    let role = match segment_of(&normalised)? {
        1 => BoneRole::Proximal(finger),
        2 => BoneRole::Intermediate(finger),
        3 => BoneRole::Distal(finger),
        _ => return None,
    };
    Some(RigBone { hand, role })
}

/// The rotation that turns one direction into another by the shortest path.
///
/// Used to point a rig's bone along the direction our own skeleton computed for it.
/// Both vectors are expected to be non-degenerate; a zero-length input leaves the
/// bone alone rather than producing a rotation of nothing in particular.
pub fn align(from: Vec3, to: Vec3) -> Quat {
    let (Some(from), Some(to)) = (from.try_normalize(), to.try_normalize()) else {
        return Quat::IDENTITY;
    };
    let dot = from.dot(to).clamp(-1.0, 1.0);
    if dot > 0.999_999 {
        return Quat::IDENTITY;
    }
    if dot < -0.999_999 {
        // Opposite directions: any perpendicular axis will do, so pick a stable one.
        let axis = from.any_orthonormal_vector();
        return Quat::from_axis_angle(axis, std::f32::consts::PI);
    }
    Quat::from_rotation_arc(from, to)
}

/// The rotation that turns a loaded model's rest pose into our own convention.
///
/// Rather than requiring the model to be exported facing a particular way — which
/// depends on the tool, the export options and whoever built the rig — the loader
/// measures where the hand is actually pointing and works out the correction. Three
/// rest-pose landmarks are enough to pin a frame down: the wrist, the middle
/// fingertip, and the line across the knuckles.
///
/// The frame this produces is [`on_hand::skeleton`]'s *local* hand frame, not world
/// space: `x` radial — toward the thumb — and `y` distal, for **either** hand. That is
/// the whole reason this does not take chirality into account. The two hands differ by
/// a flip of the palm plane, so `x` and `y` are the same for both and the third axis
/// falls out of the model's own geometry; the skeleton's neutral rotation is what puts
/// the right hand's thumb toward the low notes afterwards. Targeting world axes here
/// instead rolls the right hand palm-up, which is invisible on the fingers — they are
/// nearly round — and unmistakable on the back of the hand.
///
/// Returns `None` if the landmarks are degenerate, which means the rig is not a hand.
pub fn rest_correction(
    hand: Hand,
    wrist: Vec3,
    middle_tip: Vec3,
    index_knuckle: Vec3,
    little_knuckle: Vec3,
) -> Option<Quat> {
    let distal = (middle_tip - wrist).try_normalize()?;
    let across = (index_knuckle - little_knuckle).try_normalize()?;
    // Take the component of `across` perpendicular to `distal`, so the two axes make
    // a proper frame however the hand happens to be splayed.
    let across = (across - distal * across.dot(distal)).try_normalize()?;
    let normal = distal.cross(across);

    // Where those axes belong in the skeleton's local frame. The index knuckle is
    // nearer the thumb than the little knuckle is, on both hands, so the line across
    // the knuckles runs radially — the same way for both.
    let _ = hand;
    let target_distal = Vec3::Y;
    let target_across = Vec3::X;
    let target_normal = target_distal.cross(target_across);

    let from = glam::Mat3::from_cols(across, distal, normal);
    let to = glam::Mat3::from_cols(target_across, target_distal, target_normal);
    Some(Quat::from_mat3(&(to * from.transpose())))
}

/// The two joints a bone runs between, in our own skeleton.
pub fn segment_ends(role: BoneRole) -> Option<(on_hand::skeleton::Joint, on_hand::skeleton::Joint)> {
    use on_hand::skeleton::Joint;
    Some(match role {
        BoneRole::Wrist => return None,
        BoneRole::Proximal(f) => (Joint::Base(f), Joint::Middle(f)),
        BoneRole::Intermediate(f) => (Joint::Middle(f), Joint::Distal(f)),
        BoneRole::Distal(f) => (Joint::Distal(f), Joint::Tip(f)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(name: &str) -> Option<RigBone> {
        classify(name)
    }

    /// A synthetic hand, posed however the caller likes, with landmarks in the
    /// positions a real rig would have them.
    ///
    /// Built in the skeleton's own local frame and then turned by `pose`, so the
    /// correction has something non-trivial to undo.
    fn landmarks(hand: Hand, pose: Quat) -> (Vec3, Vec3, Vec3, Vec3) {
        // Radial is +x and distal is +y for both hands; the palm plane is what flips,
        // so the knuckles sit at +z for one hand and -z for the other. See
        // `on_hand::skeleton` for why.
        let z = hand.sign();
        let wrist = Vec3::ZERO;
        let index_knuckle = Vec3::new(30.0, 95.0, 8.0 * z);
        let little_knuckle = Vec3::new(-32.0, 84.0, 8.0 * z);
        let middle_tip = Vec3::new(2.0, 170.0, 4.0 * z);
        let turn = |v: Vec3| pose * v;
        (
            turn(wrist),
            turn(middle_tip),
            turn(index_knuckle),
            turn(little_knuckle),
        )
    }

    /// Whatever orientation a model was exported in, the correction has to bring the
    /// fingers onto +y and the thumb side onto +x — the skeleton's own local frame,
    /// for either hand.
    #[test]
    fn it_puts_any_rest_pose_into_the_skeletons_frame() {
        let poses = [
            Quat::IDENTITY,
            Quat::from_rotation_y(std::f32::consts::PI),
            Quat::from_rotation_x(1.1) * Quat::from_rotation_z(-0.7),
            Quat::from_euler(glam::EulerRot::XYZ, 0.3, -2.2, 1.4),
        ];
        for hand in Hand::ALL {
            for pose in poses {
                let (wrist, tip, index, little) = landmarks(hand, pose);
                let correction = rest_correction(hand, wrist, tip, index, little)
                    .expect("the landmarks are not degenerate");

                let distal = (correction * (tip - wrist)).normalize();
                assert!(distal.y > 0.99, "{hand:?}: fingers point {distal:?}");

                let across = (correction * (index - little)).normalize();
                assert!(across.x > 0.9, "{hand:?}: knuckles run {across:?}");
            }
        }
    }

    /// The thumb has to end up on the radial side — +x — for both hands.
    ///
    /// This is the assertion that matters, and the one that was missing. The
    /// correction is fed into [`on_hand::skeleton::Skeleton::wrist_rotation`], which
    /// expects the skeleton's *local* frame and turns it into world space itself,
    /// putting the right hand's thumb toward the low notes on the way. Aiming the
    /// correction at world axes instead lands the right hand's thumb on -x, and the
    /// whole hand comes out rolled palm-up — which is invisible on the fingers,
    /// because they are nearly round, and unmistakable on the back of the hand.
    #[test]
    fn the_thumb_ends_up_on_the_radial_side() {
        for hand in Hand::ALL {
            for pose in [
                Quat::IDENTITY,
                Quat::from_rotation_y(std::f32::consts::PI),
                Quat::from_euler(glam::EulerRot::XYZ, 0.3, -2.2, 1.4),
            ] {
                let z = hand.sign();
                let turn = |v: Vec3| pose * v;
                let wrist = turn(Vec3::ZERO);
                let tip = turn(Vec3::new(2.0, 170.0, -4.0 * z));
                let index = turn(Vec3::new(30.0, 95.0, -8.0 * z));
                let little = turn(Vec3::new(-32.0, 84.0, -8.0 * z));
                // The thumb sits radial of the index knuckle and much closer in.
                let thumb = turn(Vec3::new(62.0, 55.0, 18.0 * z));

                let correction = rest_correction(hand, wrist, tip, index, little).unwrap();
                let placed = correction * (thumb - wrist);
                assert!(
                    placed.x > 0.0,
                    "{hand:?}: the thumb came out at {placed:?}"
                );
            }
        }
    }

    #[test]
    fn it_reads_the_unity_and_vrm_humanoid_convention() {
        assert_eq!(
            role("LeftIndexProximal"),
            Some(RigBone { hand: Hand::Left, role: BoneRole::Proximal(Finger::Index) })
        );
        assert_eq!(
            role("RightLittleDistal"),
            Some(RigBone { hand: Hand::Right, role: BoneRole::Distal(Finger::Little) })
        );
        assert_eq!(
            role("RightThumbIntermediate"),
            Some(RigBone { hand: Hand::Right, role: BoneRole::Intermediate(Finger::Thumb) })
        );
        assert_eq!(
            role("LeftHand"),
            Some(RigBone { hand: Hand::Left, role: BoneRole::Wrist })
        );
    }

    #[test]
    fn it_reads_the_mixamo_convention() {
        assert_eq!(
            role("mixamorig:LeftHandIndex1"),
            Some(RigBone { hand: Hand::Left, role: BoneRole::Proximal(Finger::Index) })
        );
        assert_eq!(
            role("mixamorig:RightHandPinky3"),
            Some(RigBone { hand: Hand::Right, role: BoneRole::Distal(Finger::Little) })
        );
        assert_eq!(
            role("mixamorig:RightHand"),
            Some(RigBone { hand: Hand::Right, role: BoneRole::Wrist })
        );
    }

    #[test]
    fn it_reads_the_blender_rigify_convention() {
        assert_eq!(
            role("f_index.01.L"),
            Some(RigBone { hand: Hand::Left, role: BoneRole::Proximal(Finger::Index) })
        );
        assert_eq!(
            role("f_ring.03.R"),
            Some(RigBone { hand: Hand::Right, role: BoneRole::Distal(Finger::Ring) })
        );
        assert_eq!(
            role("thumb.02.L"),
            Some(RigBone { hand: Hand::Left, role: BoneRole::Intermediate(Finger::Thumb) })
        );
        assert_eq!(
            role("hand.R"),
            Some(RigBone { hand: Hand::Right, role: BoneRole::Wrist })
        );
    }

    #[test]
    fn it_reads_the_makehuman_convention() {
        assert_eq!(
            role("finger2-1.L"),
            Some(RigBone { hand: Hand::Left, role: BoneRole::Proximal(Finger::Index) })
        );
        assert_eq!(
            role("finger5-3.R"),
            Some(RigBone { hand: Hand::Right, role: BoneRole::Distal(Finger::Little) })
        );
        assert_eq!(
            role("finger1-2.R"),
            Some(RigBone { hand: Hand::Right, role: BoneRole::Intermediate(Finger::Thumb) })
        );
    }

    #[test]
    fn every_digit_and_segment_is_reachable() {
        // A complete Unity-convention rig should classify into all fifteen bones per
        // hand plus the wrist, with nothing colliding.
        let mut seen = std::collections::HashSet::new();
        for hand in ["Left", "Right"] {
            for finger in ["Thumb", "Index", "Middle", "Ring", "Little"] {
                for segment in ["Proximal", "Intermediate", "Distal"] {
                    let name = format!("{hand}{finger}{segment}");
                    let bone = role(&name).unwrap_or_else(|| panic!("{name} not classified"));
                    assert!(seen.insert(bone), "{name} collided with another bone");
                }
            }
            assert!(seen.insert(role(&format!("{hand}Hand")).unwrap()));
        }
        assert_eq!(seen.len(), 32);
    }

    #[test]
    fn bones_that_are_not_hands_are_ignored() {
        for name in ["Spine", "LeftUpLeg", "Hips", "Armature", "Root", "Head"] {
            let classified = role(name);
            assert!(
                classified.map(|b| b.role) != Some(BoneRole::Proximal(Finger::Index)),
                "{name} was mistaken for a finger bone: {classified:?}"
            );
        }
    }

    #[test]
    fn aligning_a_direction_to_itself_does_nothing() {
        let v = Vec3::new(1.0, 2.0, 3.0);
        assert!(align(v, v).is_near_identity());
    }

    #[test]
    fn aligning_actually_points_the_bone_the_right_way() {
        let cases = [
            (Vec3::Y, Vec3::X),
            (Vec3::Y, Vec3::new(1.0, -1.0, 0.5)),
            (Vec3::new(0.3, 0.9, -0.2), Vec3::NEG_Z),
        ];
        for (from, to) in cases {
            let rotated = align(from, to) * from.normalize();
            assert!(
                (rotated - to.normalize()).length() < 1e-5,
                "{from:?} did not end up along {to:?}, got {rotated:?}"
            );
        }
    }

    #[test]
    fn aligning_a_direction_to_its_opposite_turns_it_all_the_way_round() {
        let v = Vec3::Y;
        let rotated = align(v, -v) * v;
        assert!((rotated + v).length() < 1e-5, "got {rotated:?}");
    }

    #[test]
    fn a_zero_direction_leaves_the_bone_alone() {
        assert!(align(Vec3::ZERO, Vec3::Y).is_near_identity());
        assert!(align(Vec3::Y, Vec3::ZERO).is_near_identity());
    }

    /// A model already built in the skeleton's own frame needs no correction at all.
    #[test]
    fn a_hand_already_in_our_convention_needs_no_correction() {
        // Radial is +x and distal is +y, which is the frame for either hand.
        let correction = rest_correction(
            Hand::Right,
            Vec3::ZERO,
            Vec3::new(0.0, 180.0, 0.0),
            Vec3::new(30.0, 85.0, 0.0),
            Vec3::new(-30.0, 70.0, 0.0),
        )
        .unwrap();
        assert!(correction.is_near_identity(), "{correction:?}");
    }

    #[test]
    fn a_hand_exported_upright_is_turned_flat() {
        // A model built standing up, fingers pointing along +z, as a tool with a
        // different up-axis would export it.
        let correction = rest_correction(
            Hand::Right,
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, 180.0),
            Vec3::new(-30.0, 0.0, 85.0),
            Vec3::new(30.0, 0.0, 70.0),
        )
        .unwrap();
        let fingers = correction * Vec3::Z;
        assert!(
            (fingers - Vec3::Y).length() < 1e-4,
            "the fingers should end up pointing into the keyboard, got {fingers:?}"
        );
    }

    /// The same measurements read as either hand give the same correction.
    ///
    /// They should: the correction's job is to reach the skeleton's local frame, and
    /// that frame is the same for both hands. What tells the two apart afterwards is
    /// the skeleton's neutral rotation, not this. Reading chirality into the
    /// correction is what rolled the right hand palm-up.
    #[test]
    fn the_correction_does_not_depend_on_which_hand_it_is_told() {
        let landmarks = (
            Vec3::ZERO,
            Vec3::new(0.0, 180.0, 6.0),
            Vec3::new(30.0, 85.0, 4.0),
            Vec3::new(-30.0, 70.0, 4.0),
        );
        let right =
            rest_correction(Hand::Right, landmarks.0, landmarks.1, landmarks.2, landmarks.3)
                .unwrap();
        let left =
            rest_correction(Hand::Left, landmarks.0, landmarks.1, landmarks.2, landmarks.3)
                .unwrap();
        assert!((right * Vec3::X - left * Vec3::X).length() < 1e-4);
        assert!((right * Vec3::Y - left * Vec3::Y).length() < 1e-4);
    }

    #[test]
    fn a_degenerate_rig_is_rejected_rather_than_producing_nonsense() {
        assert!(rest_correction(Hand::Right, Vec3::ZERO, Vec3::ZERO, Vec3::X, -Vec3::X).is_none());
    }

    #[test]
    fn each_bone_knows_which_two_joints_it_runs_between() {
        use on_hand::skeleton::Joint;
        assert_eq!(segment_ends(BoneRole::Wrist), None);
        assert_eq!(
            segment_ends(BoneRole::Proximal(Finger::Index)),
            Some((Joint::Base(Finger::Index), Joint::Middle(Finger::Index)))
        );
        assert_eq!(
            segment_ends(BoneRole::Distal(Finger::Thumb)),
            Some((Joint::Distal(Finger::Thumb), Joint::Tip(Finger::Thumb)))
        );
    }
}
