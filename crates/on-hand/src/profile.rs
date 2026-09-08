//! Anthropometry: the measured dimensions of a pianist's hand.
//!
//! The reference skeleton comes from Buryanov & Kotiuk, *Proportions of Hand
//! Segments*, Int. J. Morphol. 28(3):755-758 (2010) — anteroposterior X-rays of
//! 66 adults, measuring every metacarpal and phalanx of all five digits plus the
//! soft tissue over each fingertip. Using measured bone lengths rather than the
//! usual Fibonacci folklore matters here, because the ring and little fingers are
//! shorter than the idealised ratios predict, and that is precisely what makes
//! 4-5 fingerings awkward.
//!
//! A specific pianist is described by scaling this reference isotropically to
//! their hand length, which is the standard normalized-anthropometry assumption.

use serde::{Deserialize, Serialize};

use crate::Finger;

/// One digit's bone lengths, in millimetres.
///
/// The thumb has no medial phalanx, so it stores `medial: 0.0` and its chain is
/// one joint shorter.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DigitLengths {
    /// Metacarpal: wrist-side base to the knuckle (MCP head).
    pub metacarpal: f32,
    /// Proximal phalanx: MCP to PIP (thumb: MCP to IP).
    pub proximal: f32,
    /// Medial phalanx: PIP to DIP. Zero for the thumb.
    pub medial: f32,
    /// Distal phalanx: DIP to the bone tip (thumb: IP to bone tip).
    pub distal: f32,
    /// Soft tissue over the end of the distal phalanx — this is the part that
    /// actually touches the key, so it belongs in the kinematic chain.
    pub pulp: f32,
}

impl DigitLengths {
    /// Distance from the knuckle to the fleshy fingertip.
    pub fn finger_length(&self) -> f32 {
        self.proximal + self.medial + self.distal + self.pulp
    }

    /// Distance from the metacarpal base to the fleshy fingertip.
    pub fn ray_length(&self) -> f32 {
        self.metacarpal + self.finger_length()
    }

    fn scaled(&self, k: f32) -> Self {
        Self {
            metacarpal: self.metacarpal * k,
            proximal: self.proximal * k,
            medial: self.medial * k,
            distal: self.distal * k,
            pulp: self.pulp * k,
        }
    }
}

/// Measured means from Buryanov & Kotiuk (2010), Table I, in millimetres.
/// Indexed by [`Finger`] order: thumb, index, middle, ring, little.
pub const REFERENCE_DIGITS: [DigitLengths; 5] = [
    // Digit I. The thumb's "proximal" is its proximal phalanx (MCP to IP).
    DigitLengths { metacarpal: 46.22, proximal: 31.57, medial: 0.0, distal: 21.67, pulp: 5.67 },
    // Digit II
    DigitLengths { metacarpal: 68.12, proximal: 39.78, medial: 22.38, distal: 15.82, pulp: 3.84 },
    // Digit III
    DigitLengths { metacarpal: 64.60, proximal: 44.63, medial: 26.33, distal: 17.40, pulp: 3.95 },
    // Digit IV
    DigitLengths { metacarpal: 58.00, proximal: 41.37, medial: 25.65, distal: 17.30, pulp: 3.95 },
    // Digit V
    DigitLengths { metacarpal: 53.69, proximal: 32.74, medial: 18.11, distal: 15.96, pulp: 3.73 },
];

/// Distance from the wrist crease — where the hand pivots on the forearm — to the
/// carpometacarpal joints, which is where [`REFERENCE_DIGITS`] starts measuring.
/// The X-ray study measures from the CMC joints, so this closes the gap between
/// its numbers and the hand length a pianist can measure with a ruler.
pub const REFERENCE_CARPAL_LENGTH: f32 = 25.0;

/// Hand length of the reference skeleton: wrist crease to the tip of the middle
/// finger. Equal to `REFERENCE_CARPAL_LENGTH + digit III ray length`.
pub const REFERENCE_HAND_LENGTH: f32 = REFERENCE_CARPAL_LENGTH + 156.91;

/// Coarse hand-size buckets, used to pick a finger-span table and as a shorthand
/// for people who have not measured their hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum HandSize {
    ExtraSmall,
    Small,
    #[default]
    Medium,
    Large,
    ExtraLarge,
}

impl HandSize {
    /// Representative hand length in millimetres for this bucket.
    pub fn hand_length(self) -> f32 {
        match self {
            HandSize::ExtraSmall => 155.0,
            HandSize::Small => 170.0,
            HandSize::Medium => 182.0,
            HandSize::Large => 196.0,
            HandSize::ExtraLarge => 210.0,
        }
    }

    /// The bucket a measured hand length falls into.
    pub fn from_hand_length(mm: f32) -> Self {
        match mm {
            x if x < 162.5 => HandSize::ExtraSmall,
            x if x < 176.0 => HandSize::Small,
            x if x < 189.0 => HandSize::Medium,
            x if x < 203.0 => HandSize::Large,
            _ => HandSize::ExtraLarge,
        }
    }
}

/// Where each knuckle sits in the palm, as `(radial, distal)` millimetres from the
/// wrist pivot in the reference hand.
///
/// Derived from the reference metacarpal lengths splayed across the carpus so that
/// adjacent knuckles end up ~20 mm apart and the index-to-little knuckle breadth
/// is ~62 mm, matching measured hand breadth once soft tissue is added. The arch
/// is why the ring and little fingers start further back than the index: they have
/// less reach to spend before they run out.
const REFERENCE_MCP_XY: [(f32, f32); 5] = [
    (30.0, 12.0),   // thumb CMC — proximal and well radial of the finger knuckles
    (28.65, 87.28), // index
    (8.25, 84.56),  // middle
    (-13.07, 77.57),// ring
    (-33.70, 71.35),// little
];

/// How far palmar (below the palm plane) each digit's base sits. Only the thumb is
/// meaningfully offset: its carpometacarpal joint sits in front of the palm, which
/// is what lets it oppose the fingers at all.
const REFERENCE_MCP_Z: [f32; 5] = [10.0, 0.0, 0.0, 0.0, 0.0];

/// A specific pianist's hand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandProfile {
    /// Wrist crease to the tip of the middle finger, in millimetres.
    pub hand_length: f32,
    /// Bone lengths, scaled from the reference to this hand.
    pub digits: [DigitLengths; 5],
    /// Wrist pivot to each digit's base joint, `(radial, distal, palmar)` mm.
    pub base_offsets: [(f32, f32, f32); 5],
}

impl Default for HandProfile {
    fn default() -> Self {
        Self::from_hand_length(REFERENCE_HAND_LENGTH)
    }
}

impl HandProfile {
    /// Build a profile by scaling the reference skeleton to a measured hand length.
    pub fn from_hand_length(hand_length: f32) -> Self {
        let k = hand_length / REFERENCE_HAND_LENGTH;
        let mut digits = REFERENCE_DIGITS;
        for d in &mut digits {
            *d = d.scaled(k);
        }
        let mut base_offsets = [(0.0, 0.0, 0.0); 5];
        for i in 0..5 {
            let (x, y) = REFERENCE_MCP_XY[i];
            base_offsets[i] = (x * k, y * k, REFERENCE_MCP_Z[i] * k);
        }
        Self { hand_length, digits, base_offsets }
    }

    /// Build a profile from a coarse size bucket.
    pub fn from_size(size: HandSize) -> Self {
        Self::from_hand_length(size.hand_length())
    }

    /// The size bucket this hand falls into.
    pub fn size(&self) -> HandSize {
        HandSize::from_hand_length(self.hand_length)
    }

    /// Scale relative to the reference skeleton.
    pub fn scale(&self) -> f32 {
        self.hand_length / REFERENCE_HAND_LENGTH
    }

    /// Bone lengths for one digit.
    pub fn digit(&self, finger: Finger) -> &DigitLengths {
        &self.digits[finger.index()]
    }

    /// Base joint offset for one digit, `(radial, distal, palmar)` mm from the wrist.
    pub fn base_offset(&self, finger: Finger) -> (f32, f32, f32) {
        self.base_offsets[finger.index()]
    }

    /// A quick estimate of how far apart this hand can put its thumb and little
    /// finger while still playing, in millimetres.
    ///
    /// Only an estimate: the authority is solving for the posture and looking at
    /// what it costs, which is what [`crate::ik::reach`] does. This exists for the
    /// places that need to reject obviously impossible spans cheaply, such as
    /// separating the hands of an unlabelled MIDI file, where running the full
    /// solver for every candidate would be wasteful.
    ///
    /// The constant is calibrated against the solver: an average 182 mm hand comes
    /// out at about 211 mm, which is a tenth.
    pub fn comfortable_span_mm(&self) -> f32 {
        self.hand_length * 1.16
    }

    /// Straight-line distance between two knuckles, ignoring the thumb's mobility.
    pub fn knuckle_distance(&self, a: Finger, b: Finger) -> f32 {
        let (ax, ay, az) = self.base_offset(a);
        let (bx, by, bz) = self.base_offset(b);
        ((ax - bx).powi(2) + (ay - by).powi(2) + (az - bz).powi(2)).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_hand_is_a_normal_adult_hand() {
        // Adult hand length runs about 165-200 mm; the reference should sit mid-range.
        assert!((175.0..190.0).contains(&REFERENCE_HAND_LENGTH));
        assert_eq!(HandProfile::default().size(), HandSize::Medium);
    }

    #[test]
    fn middle_finger_is_the_longest_and_little_the_shortest() {
        let p = HandProfile::default();
        let len = |f: Finger| p.digit(f).finger_length();
        assert!(len(Finger::Middle) > len(Finger::Ring));
        assert!(len(Finger::Ring) > len(Finger::Index));
        assert!(len(Finger::Index) > len(Finger::Little));
    }

    #[test]
    fn knuckles_are_about_twenty_millimetres_apart() {
        let p = HandProfile::default();
        for (a, b) in [
            (Finger::Index, Finger::Middle),
            (Finger::Middle, Finger::Ring),
            (Finger::Ring, Finger::Little),
        ] {
            let d = p.knuckle_distance(a, b);
            assert!((18.0..24.0).contains(&d), "{a:?}-{b:?} knuckles {d} mm apart");
        }
        let breadth = p.knuckle_distance(Finger::Index, Finger::Little);
        assert!((58.0..68.0).contains(&breadth), "knuckle breadth {breadth} mm");
    }

    #[test]
    fn scaling_is_isotropic_and_round_trips() {
        let big = HandProfile::from_hand_length(200.0);
        assert!((big.hand_length - 200.0).abs() < 1e-4);
        let k = big.scale();
        let ref_len = REFERENCE_DIGITS[Finger::Middle.index()].finger_length();
        let got = big.digit(Finger::Middle).finger_length();
        assert!((got - ref_len * k).abs() < 1e-3);
    }

    #[test]
    fn hand_length_is_carpus_plus_middle_ray() {
        let p = HandProfile::default();
        let modelled = REFERENCE_CARPAL_LENGTH + p.digit(Finger::Middle).ray_length();
        assert!(
            (modelled - p.hand_length).abs() < 0.05,
            "modelled {modelled} vs stated {}",
            p.hand_length
        );
    }

    #[test]
    fn the_span_estimate_agrees_with_what_pianists_report() {
        // An average hand should manage a tenth (211 mm) but not an eleventh
        // (234.5 mm); a small hand should stop around the ninth.
        let medium = HandProfile::from_size(HandSize::Medium).comfortable_span_mm();
        assert!((205.0..225.0).contains(&medium), "medium span {medium} mm");
        let small = HandProfile::from_size(HandSize::Small).comfortable_span_mm();
        assert!(small < 211.0, "a small hand should not be given a tenth: {small} mm");
        let large = HandProfile::from_size(HandSize::ExtraLarge).comfortable_span_mm();
        assert!(large > 234.0, "an extra large hand should manage an eleventh: {large} mm");
    }

    #[test]
    fn size_buckets_are_ordered_and_stable() {
        let sizes = [
            HandSize::ExtraSmall,
            HandSize::Small,
            HandSize::Medium,
            HandSize::Large,
            HandSize::ExtraLarge,
        ];
        for w in sizes.windows(2) {
            assert!(w[0].hand_length() < w[1].hand_length());
        }
        for s in sizes {
            assert_eq!(HandSize::from_hand_length(s.hand_length()), s);
        }
    }

    #[test]
    fn every_hand_takes_an_octave_and_none_takes_a_twelfth() {
        // The claim the whole picture rests on is that the hands and the keys are drawn
        // to the same scale, and hand reach is in millimetres while keys are in white
        // key widths. The two are pinned together at the notes pianists talk about: an
        // adult hand takes an octave, and nobody comfortably takes a twelfth — that is
        // the stretch the repertoire is famous for not being written for.
        let white = crate::keyboard::WHITE_KEY_WIDTH;
        let (octave, twelfth) = (7.0 * white, 11.0 * white);
        for size in [HandSize::Small, HandSize::Medium, HandSize::Large] {
            let span = HandProfile::from_size(size).comfortable_span_mm();
            assert!(span >= octave, "{size:?} cannot reach an octave: {span} mm");
            assert!(span < twelfth, "{size:?} comfortably takes a twelfth: {span} mm");
        }
        // And the buckets are ordered, so a bigger hand really does reach further.
        let small = HandProfile::from_size(HandSize::Small).comfortable_span_mm();
        let large = HandProfile::from_size(HandSize::Large).comfortable_span_mm();
        assert!(small < large);
    }
}
