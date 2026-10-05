pub mod ik;
pub mod keyboard;
pub mod profile;
pub mod skeleton;
pub mod strain;
pub mod torso;

pub use ik::{reach, ReachOutcome, ReachRequest};
pub use keyboard::{Keyboard, StrikeStyle};
pub use profile::{HandProfile, HandSize};
pub use skeleton::{HandPose, Posture, Skeleton};
pub use strain::{strain, StrainWeights};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Hand {
    Left,
    Right,
}

impl Hand {
    pub const ALL: [Hand; 2] = [Hand::Left, Hand::Right];

    #[inline]
    pub fn sign(self) -> f32 {
        match self {
            Hand::Right => 1.0,
            Hand::Left => -1.0,
        }
    }

    pub fn other(self) -> Hand {
        match self {
            Hand::Left => Hand::Right,
            Hand::Right => Hand::Left,
        }
    }

    pub fn tag(self) -> char {
        match self {
            Hand::Left => '<',
            Hand::Right => '>',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Finger {
    Thumb,
    Index,
    Middle,
    Ring,
    Little,
}

impl Finger {
    pub const ALL: [Finger; 5] = [
        Finger::Thumb,
        Finger::Index,
        Finger::Middle,
        Finger::Ring,
        Finger::Little,
    ];

    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    #[inline]
    pub fn number(self) -> u8 {
        self as u8 + 1
    }

    #[inline]
    pub fn from_index(i: usize) -> Finger {
        Finger::ALL[i]
    }

    pub fn from_number(n: u8) -> Option<Finger> {
        (1..=5).contains(&n).then(|| Finger::ALL[(n - 1) as usize])
    }

    pub fn is_weak(self) -> bool {
        matches!(self, Finger::Ring | Finger::Little)
    }
}

impl std::fmt::Display for Finger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.number())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finger_numbering_matches_piano_convention() {
        assert_eq!(Finger::Thumb.number(), 1);
        assert_eq!(Finger::Little.number(), 5);
        for f in Finger::ALL {
            assert_eq!(Finger::from_number(f.number()), Some(f));
            assert_eq!(Finger::from_index(f.index()), f);
        }
        assert_eq!(Finger::from_number(0), None);
        assert_eq!(Finger::from_number(6), None);
    }

    #[test]
    fn hands_are_opposite() {
        assert_eq!(Hand::Left.other(), Hand::Right);
        assert_eq!(Hand::Right.sign(), -Hand::Left.sign());
    }
}
