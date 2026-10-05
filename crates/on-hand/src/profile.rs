use serde::{Deserialize, Serialize};

use crate::Finger;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DigitLengths {
    pub metacarpal: f32,
    pub proximal: f32,
    pub medial: f32,
    pub distal: f32,
    pub pulp: f32,
}

impl DigitLengths {
    pub fn finger_length(&self) -> f32 {
        self.proximal + self.medial + self.distal + self.pulp
    }

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

pub const REFERENCE_DIGITS: [DigitLengths; 5] = [
    DigitLengths { metacarpal: 46.22, proximal: 31.57, medial: 0.0, distal: 21.67, pulp: 5.67 },
    DigitLengths { metacarpal: 68.12, proximal: 39.78, medial: 22.38, distal: 15.82, pulp: 3.84 },
    DigitLengths { metacarpal: 64.60, proximal: 44.63, medial: 26.33, distal: 17.40, pulp: 3.95 },
    DigitLengths { metacarpal: 58.00, proximal: 41.37, medial: 25.65, distal: 17.30, pulp: 3.95 },
    DigitLengths { metacarpal: 53.69, proximal: 32.74, medial: 18.11, distal: 15.96, pulp: 3.73 },
];

pub const REFERENCE_CARPAL_LENGTH: f32 = 25.0;

pub const REFERENCE_HAND_LENGTH: f32 = REFERENCE_CARPAL_LENGTH + 156.91;

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
    pub fn hand_length(self) -> f32 {
        match self {
            HandSize::ExtraSmall => 155.0,
            HandSize::Small => 170.0,
            HandSize::Medium => 182.0,
            HandSize::Large => 196.0,
            HandSize::ExtraLarge => 210.0,
        }
    }

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

const REFERENCE_MCP_XY: [(f32, f32); 5] = [
    (30.0, 12.0),
    (28.65, 87.28),
    (8.25, 84.56),
    (-13.07, 77.57),
    (-33.70, 71.35),
];

const REFERENCE_MCP_Z: [f32; 5] = [10.0, 0.0, 0.0, 0.0, 0.0];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandProfile {
    pub hand_length: f32,
    pub digits: [DigitLengths; 5],
    pub base_offsets: [(f32, f32, f32); 5],
}

impl Default for HandProfile {
    fn default() -> Self {
        Self::from_hand_length(REFERENCE_HAND_LENGTH)
    }
}

impl HandProfile {
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

    pub fn from_size(size: HandSize) -> Self {
        Self::from_hand_length(size.hand_length())
    }

    pub fn size(&self) -> HandSize {
        HandSize::from_hand_length(self.hand_length)
    }

    pub fn scale(&self) -> f32 {
        self.hand_length / REFERENCE_HAND_LENGTH
    }

    pub fn digit(&self, finger: Finger) -> &DigitLengths {
        &self.digits[finger.index()]
    }

    pub fn base_offset(&self, finger: Finger) -> (f32, f32, f32) {
        self.base_offsets[finger.index()]
    }

    pub fn comfortable_span_mm(&self) -> f32 {
        self.hand_length * 1.16
    }

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
        let white = crate::keyboard::WHITE_KEY_WIDTH;
        let (octave, twelfth) = (7.0 * white, 11.0 * white);
        for size in [HandSize::Small, HandSize::Medium, HandSize::Large] {
            let span = HandProfile::from_size(size).comfortable_span_mm();
            assert!(span >= octave, "{size:?} cannot reach an octave: {span} mm");
            assert!(span < twelfth, "{size:?} comfortably takes a twelfth: {span} mm");
        }
        let small = HandProfile::from_size(HandSize::Small).comfortable_span_mm();
        let large = HandProfile::from_size(HandSize::Large).comfortable_span_mm();
        assert!(small < large);
    }
}
