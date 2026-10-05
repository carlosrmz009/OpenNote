use on_hand::{Finger, Hand};

pub const CANONICAL_BONUS: f32 = 1.1;

pub const SECONDARY_BONUS: f32 = 0.35;

fn canonical_shapes(n: usize) -> &'static [&'static [u8]] {
    match n {
        2 => &[&[1, 5]],
        3 => &[&[1, 3, 5]],
        4 => &[&[1, 2, 3, 5]],
        5 => &[&[1, 2, 3, 4, 5]],
        _ => &[],
    }
}

fn secondary_shapes(n: usize) -> &'static [&'static [u8]] {
    match n {
        2 => &[&[1, 4], &[1, 3], &[2, 5]],
        3 => &[&[1, 2, 4], &[1, 2, 5], &[1, 3, 4]],
        4 => &[&[1, 2, 4, 5]],
        _ => &[],
    }
}

fn spread(notes: &[u8]) -> u8 {
    match (notes.first(), notes.last()) {
        (Some(low), Some(high)) => high - low,
        _ => 0,
    }
}

pub fn chord_bonus(hand: Hand, notes: &[u8], fingers: &[Finger]) -> f32 {
    debug_assert_eq!(notes.len(), fingers.len());
    if notes.len() < 2 || notes.len() > 5 {
        return 0.0;
    }

    let width = spread(notes);
    if width > 16 {
        return 0.0;
    }
    if notes.len() == 2 && width < 12 {
        return 0.0;
    }

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
        assert_eq!(
            chord_bonus(Hand::Left, &notes, &fingers(&[5, 3, 1])),
            CANONICAL_BONUS
        );
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
        for shape in [[1u8, 3], [2, 4], [3, 5]] {
            assert_eq!(chord_bonus(Hand::Right, &[60, 64], &fingers(&shape)), 0.0);
        }
    }

    #[test]
    fn shapes_wider_than_the_hand_shapes_to_get_nothing() {
        assert_eq!(chord_bonus(Hand::Right, &[60, 79], &fingers(&[1, 5])), 0.0);
    }

    #[test]
    fn single_notes_are_not_chords() {
        assert_eq!(chord_bonus(Hand::Right, &[60], &fingers(&[1])), 0.0);
    }
}
