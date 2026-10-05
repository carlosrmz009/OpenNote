use on_hand::keyboard::Keyboard;
use serde::{Deserialize, Serialize};

pub const SEMITONE_MM: f32 = 7.0 * on_hand::keyboard::WHITE_KEY_WIDTH / 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Ruler {
    Chromatic,
    #[default]
    Physical,
}

impl Ruler {
    pub fn distance(self, from: u8, to: u8) -> f32 {
        match self {
            Ruler::Chromatic => to as f32 - from as f32,
            Ruler::Physical => {
                keyboard().interval_mm(from, to) / SEMITONE_MM
            }
        }
    }
}

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
        let e_g = Ruler::Physical.distance(64, 67);
        let fs_a = Ruler::Physical.distance(66, 69);
        assert!(e_g > 3.0, "E to G measured {e_g}");
        assert!(fs_a < 3.0, "F# to A measured {fs_a}");
        assert!(e_g - fs_a > 0.5, "the two thirds differ by only {}", e_g - fs_a);
    }

    #[test]
    fn the_physical_ruler_stays_close_to_the_chromatic_one_on_average() {
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
