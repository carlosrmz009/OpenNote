//! Fingerings that have a known right answer.
//!
//! Every case here is one a piano teacher would mark. They are the honest test of
//! the engine: not that it produces *a* fingering, but that it produces the one
//! written in the method books.

use on_fingering::{finger_score, FingeringOptions};
use on_hand::{Finger, Hand, HandProfile, HandSize};
use on_score::{Note, NoteId, Score, SourceRef, TieState, TICKS_PER_QUARTER};

/// Build a single-line score in one hand, one note per beat fraction.
fn melody(pitches: &[u8], hand: Hand, beats: f64) -> Score {
    let step = (TICKS_PER_QUARTER as f64 * beats) as i64;
    let mut score = Score::default();
    for (i, midi) in pitches.iter().enumerate() {
        score.notes.push(Note {
            id: NoteId(i as u32),
            midi: *midi,
            onset: i as i64 * step,
            duration: step,
            onset_seconds: 0.0,
            duration_seconds: 0.0,
            staff: None,
            voice: None,
            hand: Some(hand),
            tie: TieState::default(),
            grace: false,
            chord: false,
            velocity: 72,
            given_finger: None,
            source: SourceRef::Midi { track: 0, event: i },
        });
    }
    score.finalise();
    score
}

/// Build a score from explicit `(midi, onset beat, length in beats)` entries, so a
/// note can still be sounding while later ones are struck.
fn overlapping(entries: &[(u8, f64, f64)], hand: Hand) -> Score {
    let q = TICKS_PER_QUARTER as f64;
    let mut score = Score::default();
    for (i, (midi, onset, length)) in entries.iter().enumerate() {
        score.notes.push(Note {
            id: NoteId(i as u32),
            midi: *midi,
            onset: (onset * q) as i64,
            duration: (length * q) as i64,
            onset_seconds: 0.0,
            duration_seconds: 0.0,
            staff: None,
            voice: None,
            hand: Some(hand),
            tie: TieState::default(),
            grace: false,
            chord: false,
            velocity: 72,
            given_finger: None,
            source: SourceRef::Midi { track: 0, event: i },
        });
    }
    score.finalise();
    score
}

/// Every moment where more than one note is sounding, with the fingers on them, low
/// note first.
fn sounding_shapes(score: &Score, options: &FingeringOptions) -> Vec<Vec<(u8, u8)>> {
    let solution = finger_score(score, options);
    let mut moments = Vec::new();
    for note in &score.notes {
        let at = note.onset_seconds + 1e-6;
        let mut down: Vec<(u8, u8)> = score
            .notes
            .iter()
            .filter(|other| other.onset_seconds <= at && other.offset_seconds() > at)
            .filter_map(|other| solution.finger_of(other.id).map(|f| (other.midi, f.number())))
            .collect();
        down.sort();
        if down.len() > 1 {
            moments.push(down);
        }
    }
    moments
}

/// The fingering the engine chose, in time order.
fn fingering(score: &Score, options: &FingeringOptions) -> Vec<u8> {
    let solution = finger_score(score, options);
    let mut pairs: Vec<_> = score
        .notes
        .iter()
        .filter_map(|n| solution.finger_of(n.id).map(|f| (n.onset, f.number())))
        .collect();
    pairs.sort_by_key(|(t, _)| *t);
    pairs.into_iter().map(|(_, f)| f).collect()
}

/// An ascending major scale over whole octaves.
fn major_scale(tonic: u8, octaves: usize) -> Vec<u8> {
    const DEGREES: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
    let mut out = Vec::new();
    for octave in 0..octaves {
        for degree in DEGREES {
            out.push(tonic + degree + 12 * octave as u8);
        }
    }
    out.push(tonic + 12 * octaves as u8);
    out
}

#[test]
fn two_octaves_of_c_major_are_fingered_as_every_method_book_prints_them() {
    let options = FingeringOptions::default();

    let right = melody(&major_scale(60, 2), Hand::Right, 0.25);
    assert_eq!(
        fingering(&right, &options),
        vec![1, 2, 3, 1, 2, 3, 4, 1, 2, 3, 1, 2, 3, 4, 5]
    );

    let left = melody(&major_scale(48, 2), Hand::Left, 0.25);
    assert_eq!(
        fingering(&left, &options),
        vec![5, 4, 3, 2, 1, 3, 2, 1, 4, 3, 2, 1, 3, 2, 1]
    );
}

#[test]
fn one_octave_of_c_major_is_fingered_the_same_way() {
    let options = FingeringOptions::default();
    let right = melody(&major_scale(60, 1), Hand::Right, 0.25);
    assert_eq!(fingering(&right, &options), vec![1, 2, 3, 1, 2, 3, 4, 5]);

    let left = melody(&major_scale(48, 1), Hand::Left, 0.25);
    assert_eq!(fingering(&left, &options), vec![5, 4, 3, 2, 1, 3, 2, 1]);
}

#[test]
fn f_major_crosses_after_the_fourth_finger_in_the_right_hand() {
    // The exception among the white-key scales: the thumb has to avoid B flat, so
    // the first group is four notes long instead of three.
    let options = FingeringOptions::default();
    let score = melody(&major_scale(65, 2), Hand::Right, 0.25);
    assert_eq!(
        fingering(&score, &options),
        vec![1, 2, 3, 4, 1, 2, 3, 1, 2, 3, 4, 1, 2, 3, 4]
    );
}

#[test]
fn a_descending_scale_is_the_ascending_one_backwards() {
    let options = FingeringOptions::default();
    let mut pitches = major_scale(60, 2);
    pitches.reverse();
    let score = melody(&pitches, Hand::Right, 0.25);
    let mut expected = vec![1, 2, 3, 1, 2, 3, 4, 1, 2, 3, 1, 2, 3, 4, 5];
    expected.reverse();
    assert_eq!(fingering(&score, &options), expected);
}

#[test]
fn a_five_finger_position_uses_five_fingers_and_no_crossings() {
    let options = FingeringOptions::default();
    let score = melody(&[60, 62, 64, 65, 67], Hand::Right, 0.5);
    assert_eq!(fingering(&score, &options), vec![1, 2, 3, 4, 5]);
}

#[test]
fn root_position_triads_are_fingered_one_three_five() {
    let options = FingeringOptions::default();
    for (hand, root) in [(Hand::Right, 60u8), (Hand::Left, 48)] {
        let mut score = Score::default();
        for (i, offset) in [0u8, 4, 7].iter().enumerate() {
            score.notes.push(Note {
                id: NoteId(i as u32),
                midi: root + offset,
                onset: 0,
                duration: TICKS_PER_QUARTER as i64 * 2,
                onset_seconds: 0.0,
                duration_seconds: 0.0,
                staff: None,
                voice: None,
                hand: Some(hand),
                tie: TieState::default(),
                grace: false,
                chord: i > 0,
                velocity: 72,
                given_finger: None,
                source: SourceRef::Midi { track: 0, event: i },
            });
        }
        score.finalise();
        let expected = match hand {
            Hand::Right => vec![1, 3, 5],
            Hand::Left => vec![5, 3, 1],
        };
        assert_eq!(fingering(&score, &options), expected, "{hand:?} triad");
    }
}

#[test]
fn a_tenth_punishes_a_small_hand_and_not_a_large_one() {
    // A tenth is at the edge of what a small hand can do, and comfortable for a
    // large one. The engine should price that difference rather than treat the two
    // hands alike.
    let mut score = Score::default();
    for (i, midi) in [60u8, 76].iter().enumerate() {
        score.notes.push(Note {
            id: NoteId(i as u32),
            midi: *midi,
            onset: 0,
            duration: TICKS_PER_QUARTER as i64,
            onset_seconds: 0.0,
            duration_seconds: 0.0,
            staff: None,
            voice: None,
            hand: Some(Hand::Right),
            tie: TieState::default(),
            grace: false,
            chord: i > 0,
            velocity: 72,
            given_finger: None,
            source: SourceRef::Midi { track: 0, event: i },
        });
    }
    score.finalise();

    let small = FingeringOptions::for_hand(HandProfile::from_size(HandSize::ExtraSmall));
    let large = FingeringOptions::for_hand(HandProfile::from_size(HandSize::ExtraLarge));

    let small_solution = finger_score(&score, &small);
    let large_solution = finger_score(&score, &large);

    // The model does not claim a tenth is impossible for a small hand — it can be
    // forced, which is exactly what a pianist with small hands would tell you. What
    // it says is how much that costs, and the difference is stark.
    let strain_of = |s: &on_fingering::Solution| {
        s.explanations
            .iter()
            .map(|e| e.posture_strain)
            .fold(0.0f32, f32::max)
    };
    let small_strain = strain_of(&small_solution);
    let large_strain = strain_of(&large_solution);
    assert!(
        small_strain > 10.0 * large_strain,
        "a tenth should punish a small hand far more than a large one: {small_strain} vs {large_strain}"
    );
    assert!(
        large_solution.explanations.iter().all(|e| e.reachable),
        "an extra large hand should manage a tenth comfortably"
    );
    // Both should still use the outer fingers.
    assert_eq!(
        large_solution
            .fingerings
            .iter()
            .map(|f| f.finger)
            .collect::<Vec<_>>(),
        vec![Finger::Thumb, Finger::Little]
    );
}

#[test]
fn an_editorial_fingering_is_honoured_and_the_rest_adapts() {
    let options = FingeringOptions::default();
    let mut score = melody(&major_scale(60, 1), Hand::Right, 0.25);
    score.notes[0].given_finger = Some(Finger::Index);
    let fingers = fingering(&score, &options);
    assert_eq!(fingers[0], 2);
    assert_eq!(fingers.len(), 8);
}

#[test]
fn the_same_passage_costs_more_when_there_is_less_time_for_it() {
    let options = FingeringOptions::default();
    let leaps = [60u8, 84, 60, 84, 60, 84];
    let slow = finger_score(&melody(&leaps, Hand::Right, 2.0), &options);
    let fast = finger_score(&melody(&leaps, Hand::Right, 0.05), &options);
    assert!(
        fast.cost > slow.cost,
        "slow {} should cost less than fast {}",
        slow.cost,
        fast.cost
    );
}

/// A staggered arpeggio, held. Taken from a passage that used to come out wound up.
///
/// Four notes entering one after another and all sustaining together is the shape that
/// breaks a search that only ever looks at one chord: each note is alone when it is
/// chosen, so nothing stops the fourth taking a finger the second is already using.
#[test]
fn a_held_arpeggio_keeps_its_fingers_in_order() {
    let score = overlapping(
        &[(43, 0.0, 4.0), (50, 0.5, 3.5), (57, 1.0, 3.0), (58, 1.5, 2.5)],
        Hand::Left,
    );
    for shape in sounding_shapes(&score, &FingeringOptions::default()) {
        let fingers: Vec<u8> = shape.iter().map(|(_, f)| *f).collect();
        assert!(
            fingers.windows(2).all(|w| w[0] > w[1]),
            "the left hand wound its fingers over each other: {shape:?}"
        );
        let mut unique = fingers.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), fingers.len(), "a finger is on two keys: {shape:?}");
    }
}

/// A hand cannot cross its own fingers over notes it is still holding.
///
/// This is the constraint the search used to be blind to. It saw one chord at a time,
/// so a note held from two beats ago placed no restriction at all on the next — and a
/// left hand would be told to take a low note with 5, the next one up with 1, and the
/// one above *that* with 3, which is three fingers wound over each other. Balliauw
/// states the same thing as a hard constraint, and it is why teachers have you work out
/// a fingering with no pedal: everything the score holds has to be held by a finger.
#[test]
fn fingers_do_not_cross_over_notes_the_hand_is_still_holding() {
    for hand in Hand::ALL {
        // A bass note held under two later ones, all within an octave, all reachable.
        let score = overlapping(
            &[(48, 0.0, 4.0), (52, 1.0, 3.0), (55, 2.0, 2.0)],
            hand,
        );
        let options = FingeringOptions::default();
        for shape in sounding_shapes(&score, &options) {
            let fingers: Vec<u8> = shape.iter().map(|(_, f)| *f).collect();
            let ordered = match hand {
                // Rising pitch takes rising finger numbers in the right hand and
                // falling ones in the left.
                Hand::Right => fingers.windows(2).all(|w| w[0] < w[1]),
                Hand::Left => fingers.windows(2).all(|w| w[0] > w[1]),
            };
            assert!(ordered, "{hand:?} wound its fingers over each other: {shape:?}");
        }
    }
}

/// And it cannot put one finger on two notes that are sounding together.
#[test]
fn one_finger_never_holds_two_sounding_notes() {
    for hand in Hand::ALL {
        let score = overlapping(
            &[(60, 0.0, 4.0), (64, 1.0, 3.0), (67, 2.0, 2.0), (71, 3.0, 1.0)],
            hand,
        );
        for shape in sounding_shapes(&score, &FingeringOptions::default()) {
            let mut fingers: Vec<u8> = shape.iter().map(|(_, f)| *f).collect();
            let before = fingers.len();
            fingers.sort_unstable();
            fingers.dedup();
            assert_eq!(before, fingers.len(), "{hand:?} reused a finger: {shape:?}");
        }
    }
}

/// The thumb stays off the black keys.
///
/// The first rule in every method book, and the one with the clearest physical reason:
/// the thumb is short, so reaching it up between the long fingers to a raised key
/// twists the whole hand. D flat major is where it bites hardest — only two of its
/// seven notes are white, and the standard fingering exists precisely to put the thumb
/// on those two.
#[test]
fn the_thumb_stays_off_the_black_keys_in_d_flat_major() {
    let d_flat = major_scale(61, 2);
    for hand in Hand::ALL {
        let score = melody(&d_flat, hand, 0.5);
        let solution = finger_score(&score, &FingeringOptions::default());
        for note in &score.notes {
            let Some(finger) = solution.finger_of(note.id) else {
                continue;
            };
            if finger != Finger::Thumb {
                continue;
            }
            assert!(
                !on_hand::keyboard::is_black(note.midi),
                "{hand:?} put the thumb on a black key ({}) in D flat major",
                note.midi
            );
        }
    }
}

/// And the little finger too, for the same reason.
///
/// Less absolute than the thumb — a scale ending on a black key has to finish
/// somewhere — so this only asks that it is rare rather than never.
#[test]
fn the_little_finger_mostly_stays_off_the_black_keys() {
    let mut on_black = 0;
    let mut total = 0;
    for tonic in [61u8, 63, 66, 68, 70] {
        for hand in Hand::ALL {
            let score = melody(&major_scale(tonic, 2), hand, 0.5);
            let solution = finger_score(&score, &FingeringOptions::default());
            for note in &score.notes {
                if solution.finger_of(note.id) == Some(Finger::Little) {
                    total += 1;
                    if on_hand::keyboard::is_black(note.midi) {
                        on_black += 1;
                    }
                }
            }
        }
    }
    assert!(
        total == 0 || on_black * 4 <= total,
        "the little finger landed on a black key {on_black} times out of {total}"
    );
}

/// D flat major, right hand, as every method book prints it.
///
/// The scale that exists to demonstrate the thumb rule: five of its seven notes are
/// black, and the fingering is built around putting the thumb on the two that are not.
#[test]
fn d_flat_major_puts_the_thumb_on_its_two_white_notes() {
    let score = melody(&major_scale(61, 1), Hand::Right, 0.5);
    assert_eq!(
        fingering(&score, &FingeringOptions::default()),
        vec![2, 3, 1, 2, 3, 4, 1, 2],
        "D flat major, right hand"
    );
}

/// A finger cannot play two different notes in a row.
///
/// It has to leave the first to reach the second, so the line is not legato however
/// close the two notes are. The rules charge for a finger pair being asked to span more
/// than it can, which is the right shape for a stretch — but a finger against itself
/// spans exactly nothing, so that arithmetic charged a semitone step almost nothing and
/// a chromatic scale came out with the same finger twice in a row, twice.
#[test]
fn no_finger_plays_two_different_notes_in_a_row() {
    let chromatic: Vec<u8> = (60..=84).collect();
    let cases: [(&str, Vec<u8>); 3] = [
        ("chromatic", chromatic),
        ("c major", major_scale(60, 2)),
        ("d flat major", major_scale(61, 2)),
    ];
    for (name, pitches) in cases {
        for hand in Hand::ALL {
            let score = melody(&pitches, hand, 0.25);
            let chosen = fingering(&score, &FingeringOptions::default());
            for (i, pair) in chosen.windows(2).enumerate() {
                assert_ne!(
                    pair[0], pair[1],
                    "{name}, {hand:?}: finger {} plays both note {i} and note {}                      ({} then {})",
                    pair[0],
                    i + 1,
                    pitches[i],
                    pitches[i + 1]
                );
            }
        }
    }
}
