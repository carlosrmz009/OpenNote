//! How the rules measure the distance between two keys.
//!
//! The published rule sets count semitones, because that is what you can write down
//! in a table. But semitones are not distances: E to G is 47 mm while F# to A is
//! 38.5 mm, and both are minor thirds. A hand feels the millimetres.
//!
//! [`Ruler::Physical`] keeps the rules and their tables exactly as published, and
//! only changes what gets fed into them — a real millimetre distance, re-expressed
//! in semitone units so it lands in the same scale the tables are calibrated on.
//! The tables were themselves derived from measurements on a real keyboard, so this
//! sharpens them rather than distorting them.

use on_hand::keyboard::Keyboard;
use serde::{Deserialize, Serialize};

/// Width of one semitone on average, in millimetres.
///
/// Taken from the keyboard rather than written down again: an octave is seven white
/// keys, and the two have to agree or an octave stops measuring twelve.
pub const SEMITONE_MM: f32 = 7.0 * on_hand::keyboard::WHITE_KEY_WIDTH / 12.0;

/// How to measure the interval between two keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Ruler {
    /// Count semitones, as the original papers do.
    Chromatic,
    /// Measure the real distance along the keyboard, expressed in semitone units.
    #[default]
    Physical,
}

impl Ruler {
    /// Signed distance from one key to another, in semitone units.
    pub fn distance(self, from: u8, to: u8) -> f32 {
        match self {
            Ruler::Chromatic => to as f32 - from as f32,
            Ruler::Physical => {
                // A thread-local keyboard would be over-engineering: the table is 88
                // floats and building it is a handful of additions, but it is worth
                // not doing per call, so it lives in a lazily built static.
                keyboard().interval_mm(from, to) / SEMITONE_MM
            }
        }
    }
}

/// The shared keyboard geometry.
fn keyboard() -> &'static Keyboard {
    use std::sync::OnceLock;
    static KEYBOARD: OnceLock<Keyboard> = OnceLock::new();
    KEYBOARD.get_or_init(Keyboard::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chromatic_ruler_counts_semitones() {
        assert_eq!(Ruler::Chromatic.distance(60, 72), 12.0);
        assert_eq!(Ruler::Chromatic.distance(72, 60), -12.0);
    }

    #[test]
    fn an_octave_measures_twelve_either_way() {
        for midi in 21..97u8 {
            let d = Ruler::Physical.distance(midi, midi + 12);
            assert!((d - 12.0).abs() < 1e-3, "octave at {midi} measured {d}");
        }
    }

    #[test]
    fn equal_intervals_are_not_equal_distances() {
        // Both minor thirds, but E to G is a full white key wider than F# to A.
        let e_g = Ruler::Physical.distance(64, 67);
        let fs_a = Ruler::Physical.distance(66, 69);
        assert!(e_g > 3.0, "E to G measured {e_g}");
        assert!(fs_a < 3.0, "F# to A measured {fs_a}");
        assert!(e_g - fs_a > 0.5, "the two thirds differ by only {}", e_g - fs_a);
    }

    #[test]
    fn the_physical_ruler_stays_close_to_the_chromatic_one_on_average() {
        // It has to: the tables are calibrated in semitones, so the physical ruler
        // must be a refinement rather than a rescaling.
        let mut worst: f32 = 0.0;
        for from in 21..=93u8 {
            for step in 1..=15u8 {
                let chromatic = Ruler::Chromatic.distance(from, from + step);
                let physical = Ruler::Physical.distance(from, from + step);
                worst = worst.max((chromatic - physical).abs());
            }
        }
        assert!(worst < 1.6, "physical ruler drifts up to {worst} semitones");
    }

    #[test]
    fn distance_is_antisymmetric() {
        for ruler in [Ruler::Chromatic, Ruler::Physical] {
            for (a, b) in [(60u8, 67u8), (40, 88), (21, 22)] {
                assert!((ruler.distance(a, b) + ruler.distance(b, a)).abs() < 1e-4);
            }
        }
    }
}
