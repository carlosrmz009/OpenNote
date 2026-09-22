//! Finger span tables: how far apart a pair of fingers can be.
//!
//! These are the empirical core of the published ergonomic models. For every
//! ordered pair of fingers they give six thresholds, in semitones, describing the
//! interval that pair can span:
//!
//! | Threshold | Meaning |
//! |---|---|
//! | `min_prac` / `max_prac` | the widest the pair can be forced, at all |
//! | `min_comf` / `max_comf` | as far as it can go without discomfort |
//! | `min_rel` / `max_rel` | where the pair sits when the hand is relaxed |
//!
//! The values are signed and read in the direction of travel: the right hand's
//! thumb-to-index pair relaxes at `+1..+5` semitones because the thumb sits below
//! the index, while index-to-thumb relaxes at `-5..-1`.
//!
//! # Provenance
//!
//! Transcribed from the reference implementations in `pydactyl` (David A. Randolph,
//! MIT licensed), which follow the published papers: [`PARNCUTT`] is Table 1 of
//! Parncutt, Sloboda, Clarke, Raekallio & Desain (1997); the three `BALLIAUW_*`
//! tables are the small, medium and large hands of Balliauw et al.; [`BADGEROW`] is
//! Justin Badgerow's pianist-specific revision, which loosens the thumb pairs and
//! tightens finger 4.
//!
//! # Chirality
//!
//! Only right-hand values are stored. Every table satisfies
//! `span(a, b) = -reverse(span(b, a))`, which is exactly the statement that the left
//! hand is the right hand mirrored, so the left hand is served by swapping the two
//! finger indices. That identity is asserted in the tests rather than assumed.

use on_hand::{Finger, Hand};

/// The six span thresholds for one ordered finger pair, in semitones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Narrowest the pair can be forced.
    pub min_prac: i32,
    /// Narrowest the pair is comfortable.
    pub min_comf: i32,
    /// Narrowest the pair is fully relaxed.
    pub min_rel: i32,
    /// Widest the pair is fully relaxed.
    pub max_rel: i32,
    /// Widest the pair is comfortable.
    pub max_comf: i32,
    /// Widest the pair can be forced.
    pub max_prac: i32,
}

impl Span {
    /// Build a span from its six thresholds, narrowest first.
    pub const fn new(
        min_prac: i32,
        min_comf: i32,
        min_rel: i32,
        max_rel: i32,
        max_comf: i32,
        max_prac: i32,
    ) -> Self {
        Self { min_prac, min_comf, min_rel, max_rel, max_comf, max_prac }
    }

    /// The mirror image of this span, for the other hand.
    pub const fn mirrored(&self) -> Self {
        Self {
            min_prac: -self.max_prac,
            min_comf: -self.max_comf,
            min_rel: -self.max_rel,
            max_rel: -self.min_rel,
            max_comf: -self.min_comf,
            max_prac: -self.min_prac,
        }
    }

    /// How far an interval exceeds the comfortable range, in semitones. Zero when
    /// the interval is comfortable.
    pub fn discomfort(&self, semitones: i32) -> i32 {
        if semitones > self.max_comf {
            semitones - self.max_comf
        } else if semitones < self.min_comf {
            self.min_comf - semitones
        } else {
            0
        }
    }

    /// How far an interval falls outside what the pair can physically do.
    pub fn impracticality(&self, semitones: i32) -> i32 {
        if semitones > self.max_prac {
            semitones - self.max_prac
        } else if semitones < self.min_prac {
            self.min_prac - semitones
        } else {
            0
        }
    }

    /// Whether the pair can span this interval at all.
    pub fn is_practical(&self, semitones: i32) -> bool {
        (self.min_prac..=self.max_prac).contains(&semitones)
    }

    /// Whether the pair spans this interval without stretching from its rest shape.
    pub fn is_relaxed(&self, semitones: i32) -> bool {
        (self.min_rel..=self.max_rel).contains(&semitones)
    }

    /// Whether the pair spans this interval comfortably.
    ///
    /// Wider than relaxed and narrower than practical: the range a hand will hold for
    /// a passage rather than for one chord. Parncutt's position-change rules take
    /// leaving this range as the definition of having moved the hand, and so does
    /// [`crate::playability`].
    pub fn is_comfortable(&self, semitones: i32) -> bool {
        (self.min_comf..=self.max_comf).contains(&semitones)
    }
}

/// A full set of spans for all 25 ordered finger pairs of the right hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanTable(pub [[Span; 5]; 5]);

impl SpanTable {
    /// The span for an ordered pair of fingers of the given hand.
    ///
    /// The left hand reads the table with its fingers swapped, which is the mirror
    /// identity noted in the module documentation.
    pub fn get(&self, hand: Hand, from: Finger, to: Finger) -> Span {
        match hand {
            Hand::Right => self.0[from.index()][to.index()],
            Hand::Left => self.0[to.index()][from.index()],
        }
    }
}

/// Parncutt, Sloboda, Clarke, Raekallio & Desain (1997), Table 1.
pub const PARNCUTT: SpanTable = SpanTable([
    [
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(-5, -3, 1, 5, 8, 10),
        Span::new(-4, -2, 3, 7, 10, 12),
        Span::new(-3, -1, 5, 9, 12, 14),
        Span::new(-1, 1, 7, 10, 13, 15),
    ],
    [
        Span::new(-10, -8, -5, -1, 3, 5),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 3, 5),
        Span::new(1, 1, 3, 4, 5, 7),
        Span::new(2, 2, 5, 6, 8, 10),
    ],
    [
        Span::new(-12, -10, -7, -3, 2, 4),
        Span::new(-5, -3, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 2, 4),
        Span::new(1, 1, 3, 4, 5, 7),
    ],
    [
        Span::new(-14, -12, -9, -5, 1, 3),
        Span::new(-7, -5, -4, -3, -1, -1),
        Span::new(-4, -2, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 3, 5),
    ],
    [
        Span::new(-15, -13, -10, -7, -1, 1),
        Span::new(-10, -8, -6, -5, -2, -2),
        Span::new(-7, -5, -4, -3, -1, -1),
        Span::new(-5, -3, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
    ],
]);

/// Balliauw et al., small hand.
pub const BALLIAUW_SMALL: SpanTable = SpanTable([
    [
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(-7, -5, 1, 3, 8, 10),
        Span::new(-6, -4, 3, 6, 10, 12),
        Span::new(-4, -2, 5, 8, 11, 13),
        Span::new(-2, 0, 7, 10, 12, 14),
    ],
    [
        Span::new(-10, -8, -3, -1, 5, 7),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 4, 6),
        Span::new(1, 1, 3, 4, 6, 8),
        Span::new(2, 2, 5, 6, 8, 10),
    ],
    [
        Span::new(-12, -10, -6, -3, 4, 6),
        Span::new(-6, -4, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 2, 4),
        Span::new(1, 1, 3, 4, 6, 8),
    ],
    [
        Span::new(-13, -11, -8, -5, 2, 4),
        Span::new(-8, -6, -4, -3, -1, -1),
        Span::new(-4, -2, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 4, 6),
    ],
    [
        Span::new(-14, -12, -10, -7, 0, 2),
        Span::new(-10, -8, -6, -5, -2, -2),
        Span::new(-8, -6, -4, -3, -1, -1),
        Span::new(-6, -4, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
    ],
]);

/// Balliauw et al., medium hand.
pub const BALLIAUW_MEDIUM: SpanTable = SpanTable([
    [
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(-8, -6, 1, 5, 8, 10),
        Span::new(-7, -5, 3, 9, 12, 14),
        Span::new(-5, -3, 5, 11, 13, 15),
        Span::new(-2, 0, 7, 12, 14, 16),
    ],
    [
        Span::new(-10, -8, -5, -1, 6, 8),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 5, 7),
        Span::new(1, 1, 3, 4, 6, 8),
        Span::new(2, 2, 5, 6, 10, 12),
    ],
    [
        Span::new(-14, -12, -9, -3, 5, 7),
        Span::new(-7, -5, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 2, 4),
        Span::new(1, 1, 3, 4, 6, 8),
    ],
    [
        Span::new(-15, -13, -11, -5, 3, 5),
        Span::new(-8, -6, -4, -3, -1, -1),
        Span::new(-4, -2, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 4, 6),
    ],
    [
        Span::new(-16, -14, -12, -7, 0, 2),
        Span::new(-12, -10, -6, -5, -2, -2),
        Span::new(-8, -6, -4, -3, -1, -1),
        Span::new(-6, -4, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
    ],
]);

/// Balliauw et al., large hand.
pub const BALLIAUW_LARGE: SpanTable = SpanTable([
    [
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(-10, -8, 1, 6, 9, 11),
        Span::new(-8, -6, 3, 9, 13, 15),
        Span::new(-6, -4, 5, 11, 14, 16),
        Span::new(-2, 0, 7, 12, 16, 18),
    ],
    [
        Span::new(-11, -9, -6, -1, 8, 10),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 5, 7),
        Span::new(1, 1, 3, 4, 6, 8),
        Span::new(2, 2, 5, 6, 10, 12),
    ],
    [
        Span::new(-15, -13, -9, -3, 6, 8),
        Span::new(-7, -5, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 2, 4),
        Span::new(1, 1, 3, 4, 6, 8),
    ],
    [
        Span::new(-16, -14, -11, -5, 4, 6),
        Span::new(-8, -6, -4, -3, -1, -1),
        Span::new(-4, -2, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 4, 6),
    ],
    [
        Span::new(-18, -16, -12, -7, 0, 2),
        Span::new(-12, -10, -6, -5, -2, -2),
        Span::new(-8, -6, -4, -3, -1, -1),
        Span::new(-6, -4, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
    ],
]);

/// Badgerow's pianist-specific revision.
pub const BADGEROW: SpanTable = SpanTable([
    [
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(-5, -3, 1, 5, 9, 10),
        Span::new(-4, -2, 1, 7, 11, 12),
        Span::new(-3, -1, 2, 9, 13, 14),
        Span::new(-1, 1, 7, 10, 14, 15),
    ],
    [
        Span::new(-10, -9, -5, -1, 3, 5),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 2, 5),
        Span::new(1, 1, 3, 4, 5, 7),
        Span::new(2, 2, 5, 6, 8, 10),
    ],
    [
        Span::new(-12, -11, -7, -1, 2, 4),
        Span::new(-5, -2, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 2, 4),
        Span::new(1, 1, 3, 4, 5, 7),
    ],
    [
        Span::new(-14, -13, -9, -2, 1, 3),
        Span::new(-7, -5, -4, -3, -1, -1),
        Span::new(-4, -2, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
        Span::new(1, 1, 1, 2, 3, 5),
    ],
    [
        Span::new(-15, -14, -10, -7, -1, 1),
        Span::new(-10, -8, -6, -5, -2, -2),
        Span::new(-7, -5, -4, -3, -1, -1),
        Span::new(-5, -3, -2, -1, -1, -1),
        Span::new(0, 0, 0, 0, 0, 0),
    ],
]);

/// Which published span table to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SpanModel {
    /// Parncutt et al. (1997), the original.
    Parncutt,
    /// Balliauw et al., small hand.
    BalliauwSmall,
    /// Balliauw et al., medium hand.
    #[default]
    BalliauwMedium,
    /// Balliauw et al., large hand.
    BalliauwLarge,
    /// Badgerow's revision.
    Badgerow,
}

impl SpanModel {
    /// The table itself.
    pub fn table(self) -> &'static SpanTable {
        match self {
            SpanModel::Parncutt => &PARNCUTT,
            SpanModel::BalliauwSmall => &BALLIAUW_SMALL,
            SpanModel::BalliauwMedium => &BALLIAUW_MEDIUM,
            SpanModel::BalliauwLarge => &BALLIAUW_LARGE,
            SpanModel::Badgerow => &BADGEROW,
        }
    }

    /// The table that best fits a hand of the given size.
    pub fn for_hand_size(size: on_hand::HandSize) -> Self {
        use on_hand::HandSize::*;
        match size {
            ExtraSmall | Small => SpanModel::BalliauwSmall,
            Medium => SpanModel::BalliauwMedium,
            Large | ExtraLarge => SpanModel::BalliauwLarge,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [(&str, &SpanTable); 5] = [
        ("parncutt", &PARNCUTT),
        ("balliauw small", &BALLIAUW_SMALL),
        ("balliauw medium", &BALLIAUW_MEDIUM),
        ("balliauw large", &BALLIAUW_LARGE),
        ("badgerow", &BADGEROW),
    ];

    #[test]
    fn thresholds_are_ordered_within_every_span() {
        for (name, table) in ALL {
            for a in Finger::ALL {
                for b in Finger::ALL {
                    let s = table.0[a.index()][b.index()];
                    assert!(
                        s.min_prac <= s.min_comf
                            && s.min_comf <= s.min_rel
                            && s.min_rel <= s.max_rel
                            && s.max_rel <= s.max_comf
                            && s.max_comf <= s.max_prac,
                        "{name} {a:?} to {b:?} thresholds out of order: {s:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_table_is_its_own_mirror_with_the_fingers_swapped() {
        // This identity is what lets one right-hand table serve both hands.
        for (name, table) in ALL {
            for a in Finger::ALL {
                for b in Finger::ALL {
                    let forward = table.0[a.index()][b.index()];
                    let backward = table.0[b.index()][a.index()];
                    assert_eq!(
                        forward.mirrored(),
                        backward,
                        "{name} {a:?} to {b:?} is not the mirror of {b:?} to {a:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_finger_with_itself_spans_nothing() {
        for (name, table) in ALL {
            for f in Finger::ALL {
                let s = table.0[f.index()][f.index()];
                assert_eq!((s.min_prac, s.max_prac), (0, 0), "{name} {f:?} with itself");
            }
        }
    }

    #[test]
    fn parncutt_table_one_matches_the_published_values() {
        let t = &PARNCUTT;
        let s = t.get(Hand::Right, Finger::Thumb, Finger::Index);
        assert_eq!((s.min_prac, s.min_comf, s.min_rel), (-5, -3, 1));
        assert_eq!((s.max_rel, s.max_comf, s.max_prac), (5, 8, 10));

        let s = t.get(Hand::Right, Finger::Thumb, Finger::Little);
        assert_eq!((s.min_prac, s.min_comf, s.min_rel), (-1, 1, 7));
        assert_eq!((s.max_rel, s.max_comf, s.max_prac), (10, 13, 15));

        let s = t.get(Hand::Right, Finger::Index, Finger::Little);
        assert_eq!((s.min_prac, s.min_comf, s.min_rel), (2, 2, 5));
        assert_eq!((s.max_rel, s.max_comf, s.max_prac), (6, 8, 10));
    }

    #[test]
    fn the_stretch_rule_example_from_the_paper() {
        // Parncutt's worked example: fingers 2 and 5 playing D4 to C5, ten
        // semitones, against a maximum comfortable span of eight. The Stretch rule
        // charges two points per excess semitone, so four points.
        let s = PARNCUTT.get(Hand::Right, Finger::Index, Finger::Little);
        assert_eq!(s.max_comf, 8);
        assert_eq!(s.discomfort(10), 2);
        assert_eq!(2 * s.discomfort(10), 4);
    }

    #[test]
    fn the_left_hand_reads_the_mirror_image() {
        // Right hand thumb then index ascending a fourth is relaxed; the left hand
        // making the same shape descends instead.
        let right = PARNCUTT.get(Hand::Right, Finger::Thumb, Finger::Index);
        let left = PARNCUTT.get(Hand::Left, Finger::Thumb, Finger::Index);
        assert!(right.is_relaxed(5));
        assert!(left.is_relaxed(-5));
        assert!(!left.is_relaxed(5));
    }

    #[test]
    fn bigger_hands_get_wider_tables() {
        let small = BALLIAUW_SMALL.get(Hand::Right, Finger::Thumb, Finger::Little).max_prac;
        let medium = BALLIAUW_MEDIUM.get(Hand::Right, Finger::Thumb, Finger::Little).max_prac;
        let large = BALLIAUW_LARGE.get(Hand::Right, Finger::Thumb, Finger::Little).max_prac;
        assert!(small < medium && medium < large, "{small} {medium} {large}");
    }

    #[test]
    fn an_octave_is_practical_for_every_hand_but_a_twelfth_is_not() {
        for (name, table) in ALL {
            let s = table.get(Hand::Right, Finger::Thumb, Finger::Little);
            assert!(s.is_practical(12), "{name} cannot span an octave");
            assert!(!s.is_practical(19), "{name} claims to span a twelfth");
        }
    }

    #[test]
    fn span_models_follow_hand_size() {
        use on_hand::HandSize;
        assert_eq!(SpanModel::for_hand_size(HandSize::Small), SpanModel::BalliauwSmall);
        assert_eq!(SpanModel::for_hand_size(HandSize::Medium), SpanModel::BalliauwMedium);
        assert_eq!(SpanModel::for_hand_size(HandSize::ExtraLarge), SpanModel::BalliauwLarge);
    }
}
