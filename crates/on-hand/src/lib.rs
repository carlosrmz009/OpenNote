//! A biomechanical model of a pianist's hand at the keyboard.
//!
//! This crate is deliberately the foundation of everything else in OpenNote. The
//! same model that decides *which finger should play this note* also drives the 3D
//! hands in the visualizer, so the animation is a direct picture of the reasoning
//! rather than a separate hand-authored performance. Thumb crossings, wrist
//! rotation and hand repositioning are not scripted anywhere; they fall out of
//! minimising [`strain`] over a physically constrained skeleton.
//!
//! The pieces:
//!
//! * [`keyboard`] — real key geometry in millimetres, including the depth axis
//! * [`profile`] — measured hand anthropometry, scaled to an individual
//! * [`skeleton`] — forward kinematics with joint limits and tendon coupling
//! * [`strain`] — how uncomfortable a posture is
//! * [`ik`] — the inverse problem: what posture reaches these keys most easily

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

/// Which hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Hand {
    /// The left hand, normally the lower staff.
    Left,
    /// The right hand, normally the upper staff.
    Right,
}

impl Hand {
    /// Both hands, left first.
    pub const ALL: [Hand; 2] = [Hand::Left, Hand::Right];

    /// The chirality factor: `+1` for the right hand, `-1` for the left.
    ///
    /// Multiplies every geometric quantity and joint angle whose sign is defined
    /// relative to the palm, which is all the chirality handling this crate needs.
    #[inline]
    pub fn sign(self) -> f32 {
        match self {
            Hand::Right => 1.0,
            Hand::Left => -1.0,
        }
    }

    /// The other hand.
    pub fn other(self) -> Hand {
        match self {
            Hand::Left => Hand::Right,
            Hand::Right => Hand::Left,
        }
    }

    /// Single-character tag used in fingering notation and corpus files.
    pub fn tag(self) -> char {
        match self {
            Hand::Left => '<',
            Hand::Right => '>',
        }
    }
}

/// A finger, numbered the way pianists number them: thumb is 1, little finger is 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Finger {
    /// Finger 1.
    Thumb,
    /// Finger 2.
    Index,
    /// Finger 3.
    Middle,
    /// Finger 4.
    Ring,
    /// Finger 5.
    Little,
}

impl Finger {
    /// All five fingers, thumb first.
    pub const ALL: [Finger; 5] = [
        Finger::Thumb,
        Finger::Index,
        Finger::Middle,
        Finger::Ring,
        Finger::Little,
    ];

    /// Zero-based index, for array lookups.
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    /// The finger number a pianist would write on the score, 1 through 5.
    #[inline]
    pub fn number(self) -> u8 {
        self as u8 + 1
    }

    /// From a zero-based index.
    ///
    /// # Panics
    /// If `i` is not in `0..5`.
    #[inline]
    pub fn from_index(i: usize) -> Finger {
        Finger::ALL[i]
    }

    /// From a written finger number, 1 through 5.
    pub fn from_number(n: u8) -> Option<Finger> {
        (1..=5).contains(&n).then(|| Finger::ALL[(n - 1) as usize])
    }

    /// Whether this is one of the fingers pedagogy treats as weak.
    ///
    /// The ring finger shares tendon slips with its neighbours through the
    /// juncturae tendinum and has no independent extensor, which is a real
    /// anatomical constraint rather than a matter of training.
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
