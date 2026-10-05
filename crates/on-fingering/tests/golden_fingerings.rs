use on_fingering::{finger_score, FingeringOptions};
use on_hand::{Finger, Hand, HandProfile, HandSize};
use on_score::{Note, NoteId, Score, SourceRef, TieState, TICKS_PER_QUARTER};

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

#[test]
fn fingers_do_not_cross_over_notes_the_hand_is_still_holding() {
    for hand in Hand::ALL {
        let score = overlapping(
            &[(48, 0.0, 4.0), (52, 1.0, 3.0), (55, 2.0, 2.0)],
            hand,
        );
        let options = FingeringOptions::default();
        for shape in sounding_shapes(&score, &options) {
            let fingers: Vec<u8> = shape.iter().map(|(_, f)| *f).collect();
            let ordered = match hand {
                Hand::Right => fingers.windows(2).all(|w| w[0] < w[1]),
                Hand::Left => fingers.windows(2).all(|w| w[0] > w[1]),
            };
            assert!(ordered, "{hand:?} wound its fingers over each other: {shape:?}");
        }
    }
}

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

#[test]
fn d_flat_major_puts_the_thumb_on_its_two_white_notes() {
    let score = melody(&major_scale(61, 1), Hand::Right, 0.5);
    assert_eq!(
        fingering(&score, &FingeringOptions::default()),
        vec![2, 3, 1, 2, 3, 4, 1, 2],
        "D flat major, right hand"
    );
}

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

#[test]
fn a_minor_scale_is_not_fingered_as_its_relative_major() {
    let pitches: Vec<u8> = vec![69, 71, 72, 74, 76, 77, 79, 81, 83, 84, 86, 88, 89, 91, 93];
    let score = melody(&pitches, Hand::Right, 0.25);
    let chosen = fingering(&score, &FingeringOptions::default());
    assert_eq!(
        *chosen.last().expect("a fingering"),
        5,
        "the top of a two-octave minor scale takes the little finger, not {:?}",
        chosen
    );
    assert_eq!(chosen[0], 1, "and the bottom takes the thumb: {chosen:?}");
}

#[test]
fn two_octaves_of_e_harmonic_minor_are_fingered_like_e_major_both_ways() {
    const HARMONIC_MINOR: [u8; 7] = [0, 2, 3, 5, 7, 8, 11];
    let up_and_down = |tonic: u8| {
        let mut up: Vec<u8> = (0..2)
            .flat_map(|octave| HARMONIC_MINOR.map(|step| tonic + step + 12 * octave))
            .collect();
        up.push(tonic + 24);
        let mut both = up.clone();
        both.extend(up.iter().rev().skip(1));
        both
    };
    let both_ways = |up: [u8; 15]| {
        let mut both = up.to_vec();
        both.extend(up.iter().rev().skip(1));
        both
    };
    let options = FingeringOptions::default();

    let right = melody(&up_and_down(64), Hand::Right, 0.25);
    assert_eq!(
        fingering(&right, &options),
        both_ways([1, 2, 3, 1, 2, 3, 4, 1, 2, 3, 1, 2, 3, 4, 5])
    );

    let left = melody(&up_and_down(52), Hand::Left, 0.25);
    assert_eq!(
        fingering(&left, &options),
        both_ways([5, 4, 3, 2, 1, 3, 2, 1, 4, 3, 2, 1, 3, 2, 1])
    );
}

#[test]
fn two_octave_arpeggios_are_fingered_as_the_method_books_print_them() {
    let options = FingeringOptions::default();
    let mut wrong = Vec::new();
    for (name, root, third) in [("C", 0u8, 4u8), ("F", 5, 4), ("G", 7, 4), ("A minor", 9, 3), ("D minor", 2, 3), ("E minor", 4, 3)] {
        for (hand, base, up) in [(Hand::Right, 60u8, [1u8, 2, 3, 1, 2, 3, 5]), (Hand::Left, 48, [5, 4, 2, 1, 4, 2, 1])] {
            let tonic = base + root;
            let mut pitches: Vec<u8> = (0..2).flat_map(|o| [0, third, 7].map(|s| tonic + 12 * o + s)).collect();
            pitches.push(tonic + 24);
            let mut both = pitches.clone();
            both.extend(pitches.iter().rev().skip(1));
            let mut want = up.to_vec();
            want.extend(up.iter().rev().skip(1));
            let got = fingering(&melody(&both, hand, 0.25), &options);
            if got != want {
                wrong.push(format!("{name} {hand:?}: {got:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
