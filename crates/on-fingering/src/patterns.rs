//! Fingerings that pianists are taught rather than derive.
//!
//! An ergonomic model can tell you that a shape is reachable and roughly what it
//! costs. It cannot tell you that a root-position triad is fingered 1-3-5, because
//! the reason is only partly biomechanical: 1-3-5 is a stable, symmetric shape that
//! leaves the hand where the next chord will want it, and it is what every teacher
//! and every edition uses, so it is what a pianist's hand already knows.
//!
//! This module supplies that knowledge as a small bonus on the shapes pedagogy
//! actually teaches. It is deliberately narrow: only the handful of shapes that are
//! genuinely standard, and only a bonus large enough to settle a close decision, so
//! it can break a tie between two comparable fingerings but never overrule the hand
//! model when a shape is awkward or out of reach.

use on_hand::{Finger, Hand};

/// Size of the bonus given to a canonical shape.
///
/// Calibrated against the rest of the cost function: comparable chord shapes differ
/// by a few tenths, so this settles those, while an unreachable shape costs tens and
/// stays unreachable.
pub const CANONICAL_BONUS: f32 = 1.1;

/// A smaller bonus for shapes that are standard in some contexts but not the first
/// thing a teacher would write.
pub const SECONDARY_BONUS: f32 = 0.35;

/// The canonical fingerings for a chord of `n` notes played together, as right-hand
/// finger numbers from the lowest note upward.
///
/// The left hand mirrors these: the same shape read from the top down.
fn canonical_shapes(n: usize) -> &'static [&'static [u8]] {
    match n {
        2 => &[&[1, 5]],
        3 => &[&[1, 3, 5]],
        4 => &[&[1, 2, 3, 5]],
        5 => &[&[1, 2, 3, 4, 5]],
        _ => &[],
    }
}

/// Shapes that are standard in narrower circumstances.
fn secondary_shapes(n: usize) -> &'static [&'static [u8]] {
    match n {
        2 => &[&[1, 4], &[1, 3], &[2, 5]],
        3 => &[&[1, 2, 4], &[1, 2, 5], &[1, 3, 4]],
        4 => &[&[1, 2, 4, 5]],
        _ => &[],
    }
}

/// The interval in semitones from the lowest note of a chord to its highest.
fn spread(notes: &[u8]) -> u8 {
    match (notes.first(), notes.last()) {
        (Some(low), Some(high)) => high - low,
        _ => 0,
    }
}

/// The bonus, as a negative cost, for a chord fingering that matches what pianists
/// are taught.
///
/// Returns zero for anything that is not a recognisable chord: single notes, chords
/// spanning more than the hand naturally shapes to, and anything whose fingering is
/// not one of the standard ones.
pub fn chord_bonus(hand: Hand, notes: &[u8], fingers: &[Finger]) -> f32 {
    debug_assert_eq!(notes.len(), fingers.len());
    if notes.len() < 2 || notes.len() > 5 {
        return 0.0;
    }

    // Beyond a tenth the hand is stretching rather than shaping, and the standard
    // fingerings stop applying. An octave or wider two-note interval is the one case
    // where the shape is unambiguous.
    let width = spread(notes);
    if width > 16 {
        return 0.0;
    }
    if notes.len() == 2 && width < 12 {
        // Sixths and sevenths have no single standard fingering; which one is right
        // depends entirely on what surrounds them.
        return 0.0;
    }

    // Read the shape as right-hand finger numbers from the bottom up. The left hand
    // plays the mirror image, so its lowest note takes the highest finger.
    let shape: Vec<u8> = match hand {
        Hand::Right => fingers.iter().map(|f| f.number()).collect(),
        Hand::Left => fingers.iter().rev().map(|f| f.number()).collect(),
    };

    if canonical_shapes(notes.len()).contains(&shape.as_slice()) {
        return CANONICAL_BONUS;
    }
    if secondary_shapes(notes.len()).contains(&shape.as_slice()) {
        return SECONDARY_BONUS;
    }
    0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingers(numbers: &[u8]) -> Vec<Finger> {
        numbers
            .iter()
            .map(|n| Finger::from_number(*n).unwrap())
            .collect()
    }

    #[test]
    fn a_root_position_triad_is_one_three_five() {
        let notes = [60u8, 64, 67];
        assert_eq!(
            chord_bonus(Hand::Right, &notes, &fingers(&[1, 3, 5])),
            CANONICAL_BONUS
        );
        assert_eq!(
            chord_bonus(Hand::Right, &notes, &fingers(&[1, 2, 4])),
            SECONDARY_BONUS
        );
        assert_eq!(chord_bonus(Hand::Right, &notes, &fingers(&[2, 3, 4])), 0.0);
    }

    #[test]
    fn the_left_hand_plays_the_mirror_of_the_same_shape() {
        let notes = [48u8, 52, 55];
        // Left hand takes the lowest note with the little finger.
        assert_eq!(
            chord_bonus(Hand::Left, &notes, &fingers(&[5, 3, 1])),
            CANONICAL_BONUS
        );
        // The right hand's shape is wrong for the left.
        assert_eq!(chord_bonus(Hand::Left, &notes, &fingers(&[1, 3, 5])), 0.0);
    }

    #[test]
    fn a_seventh_chord_is_one_two_three_five() {
        let notes = [60u8, 64, 67, 70];
        assert_eq!(
            chord_bonus(Hand::Right, &notes, &fingers(&[1, 2, 3, 5])),
            CANONICAL_BONUS
        );
        assert_eq!(
            chord_bonus(Hand::Left, &notes, &fingers(&[5, 3, 2, 1])),
            CANONICAL_BONUS
        );
    }

    #[test]
    fn an_octave_is_thumb_and_little_finger() {
        assert_eq!(
            chord_bonus(Hand::Right, &[60, 72], &fingers(&[1, 5])),
            CANONICAL_BONUS
        );
        assert_eq!(
            chord_bonus(Hand::Right, &[60, 72], &fingers(&[1, 4])),
            SECONDARY_BONUS
        );
    }

    #[test]
    fn narrow_two_note_intervals_have_no_standard_fingering() {
        // A third could reasonably be 1-3, 2-4 or 3-5; context decides, not habit.
        for shape in [[1u8, 3], [2, 4], [3, 5]] {
            assert_eq!(chord_bonus(Hand::Right, &[60, 64], &fingers(&shape)), 0.0);
        }
    }

    #[test]
    fn shapes_wider_than_the_hand_shapes_to_get_nothing() {
        // A twelfth is a stretch, not a shape.
        assert_eq!(chord_bonus(Hand::Right, &[60, 79], &fingers(&[1, 5])), 0.0);
    }

    #[test]
    fn single_notes_are_not_chords() {
        assert_eq!(chord_bonus(Hand::Right, &[60], &fingers(&[1])), 0.0);
    }
}
