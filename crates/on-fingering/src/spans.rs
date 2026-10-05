use on_hand::{Finger, Hand};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub min_prac: i32,
    pub min_comf: i32,
    pub min_rel: i32,
    pub max_rel: i32,
    pub max_comf: i32,
    pub max_prac: i32,
}

impl Span {
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

    pub fn discomfort(&self, semitones: i32) -> i32 {
        if semitones > self.max_comf {
            semitones - self.max_comf
        } else if semitones < self.min_comf {
            self.min_comf - semitones
        } else {
            0
        }
    }

    pub fn impracticality(&self, semitones: i32) -> i32 {
        if semitones > self.max_prac {
            semitones - self.max_prac
        } else if semitones < self.min_prac {
            self.min_prac - semitones
        } else {
            0
        }
    }

    pub fn is_practical(&self, semitones: i32) -> bool {
        (self.min_prac..=self.max_prac).contains(&semitones)
    }

    pub fn is_relaxed(&self, semitones: i32) -> bool {
        (self.min_rel..=self.max_rel).contains(&semitones)
    }

    pub fn is_comfortable(&self, semitones: i32) -> bool {
        (self.min_comf..=self.max_comf).contains(&semitones)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanTable(pub [[Span; 5]; 5]);

impl SpanTable {
    pub fn get(&self, hand: Hand, from: Finger, to: Finger) -> Span {
        match hand {
            Hand::Right => self.0[from.index()][to.index()],
            Hand::Left => self.0[to.index()][from.index()],
        }
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SpanModel {
    Parncutt,
    BalliauwSmall,
    #[default]
    BalliauwMedium,
    BalliauwLarge,
    Badgerow,
}

impl SpanModel {
    pub fn table(self) -> &'static SpanTable {
        match self {
            SpanModel::Parncutt => &PARNCUTT,
            SpanModel::BalliauwSmall => &BALLIAUW_SMALL,
            SpanModel::BalliauwMedium => &BALLIAUW_MEDIUM,
            SpanModel::BalliauwLarge => &BALLIAUW_LARGE,
            SpanModel::Badgerow => &BADGEROW,
        }
    }

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
        let s = PARNCUTT.get(Hand::Right, Finger::Index, Finger::Little);
        assert_eq!(s.max_comf, 8);
        assert_eq!(s.discomfort(10), 2);
        assert_eq!(2 * s.discomfort(10), 4);
    }

    #[test]
    fn the_left_hand_reads_the_mirror_image() {
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
