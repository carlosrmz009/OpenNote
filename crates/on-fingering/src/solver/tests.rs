use super::*;
use on_score::{Note, NoteId, Score, SourceRef, TICKS_PER_QUARTER};

fn melody(pitches: &[u8], hand: Hand, beats_per_note: f64) -> Score {
    let mut score = Score::default();
    let step = (TICKS_PER_QUARTER as f64 * beats_per_note) as i64;
    for (i, midi) in pitches.iter().enumerate() {
        score.notes.push(Note {
            id: NoteId(i as u32),
            midi: *midi,
            onset: i as i64 * step,
            duration: step,
            hand: Some(hand),
            velocity: 72,
            source: SourceRef::Midi { track: 0, event: i },
            ..Default::default()
        });
    }
    score.finalise();
    score
}

struct FailsForTheLeftHand;

impl FingeringPrior for FailsForTheLeftHand {
    fn log_probability(&self, _: Hand, _: Option<Placement>, _: Option<Placement>, _: Placement) -> f32 {
        0.0
    }

    fn chord_cost(&self, hand: Hand, _notes: &[u8], _fingers: &[Finger]) -> f32 {
        assert!(hand == Hand::Right, "the left hand's search failed");
        0.0
    }
}

#[test]
fn a_hand_whose_search_fails_is_not_quietly_left_out() {
    let mut score = melody(&[60, 62, 64], Hand::Right, 1.0);
    score.notes.extend(melody(&[48, 50, 52], Hand::Left, 1.0).notes);
    score.finalise();
    let outcome = std::panic::catch_unwind(|| {
        finger_score_with_prior(&score, &FingeringOptions::default(), Some(&FailsForTheLeftHand))
    });
    assert!(
        outcome.is_err(),
        "a solution came back with {} fingerings for six notes",
        outcome.map(|solution| solution.fingerings.len()).unwrap_or_default()
    );
}

#[test]
fn a_written_finger_is_kept_when_a_held_note_asks_for_the_impossible() {
    // C5 is held with the fifth finger while E5, above it, is marked 2: the right hand
    // cannot keep both. The struck note's written finger wins over the held one's.
    let mut score = melody(&[72, 76], Hand::Right, 1.0);
    score.notes[0].duration *= 4;
    score.notes[0].given_finger = Some(Finger::Little);
    score.notes[1].given_finger = Some(Finger::Index);
    score.finalise();
    let solution = finger_score(&score, &FingeringOptions::default());
    let e5 = score.notes.iter().find(|n| n.midi == 76).unwrap().id;
    assert_eq!(solution.finger_of(e5), Some(Finger::Index));
}

#[test]
fn a_key_struck_again_is_not_also_held() {
    let mut score = melody(&[60, 60, 64], Hand::Right, 1.0);
    score.notes[0].duration *= 3;
    score.finalise();
    let mut events = build_events(&score, Hand::Right);
    hold_sustained(&mut events, &score, 14, &Default::default());
    for event in &events {
        let mut keys = event.notes.clone();
        keys.dedup();
        assert_eq!(keys, event.notes, "the same key twice in one chord: {:?}", event.notes);
    }
}

fn ticks_for(score: &Score, seconds: f64) -> i64 {
    let note = &score.notes[1];
    (seconds * note.onset as f64 / note.onset_seconds).round() as i64
}

#[test]
fn a_note_let_go_as_the_next_lands_is_not_held() {
    let line = melody(&[60, 62, 64, 65, 67], Hand::Right, 0.5);
    for (overlap, held) in [(0.0, false), (0.002, false), (0.020, false), (0.100, true)] {
        let mut score = line.clone();
        let extra = ticks_for(&score, overlap);
        for note in &mut score.notes {
            note.duration += extra;
        }
        score.finalise();
        let mut events = build_events(&score, Hand::Right);
        hold_sustained(&mut events, &score, 14, &Default::default());
        let widest = events.iter().map(Event::len).max().unwrap_or(0);
        assert_eq!(widest > 1, held, "a note running {overlap} s past the next onset");
    }
}

#[test]
fn a_few_milliseconds_of_timing_change_no_finger() {
    let wobble = [0.009, -0.008, 0.004, -0.010, 0.007, -0.003, 0.010, -0.006];
    for score in passages() {
        let plain = fingers_of(&score, &finger_score(&score, &FingeringOptions::default()));
        let mut moved = score.clone();
        let mut onsets: Vec<i64> = moved.notes.iter().map(|n| n.onset).collect();
        onsets.sort_unstable();
        onsets.dedup();
        for note in &mut moved.notes {
            let chord = onsets.binary_search(&note.onset).unwrap();
            note.onset += ticks_for(&score, wobble[chord % wobble.len()]);
        }
        moved.finalise();
        let wobbled = fingers_of(&moved, &finger_score(&moved, &FingeringOptions::default()));
        assert_eq!(plain, wobbled, "{:?}", score.notes.iter().map(|n| n.midi).collect::<Vec<_>>());
    }
}

fn passages() -> Vec<Score> {
    vec![
        melody(&[60, 62, 64, 65, 67, 69, 71, 72, 71, 69, 67, 65, 64, 62, 60], Hand::Right, 0.5),
        melody(&[60, 67, 64, 72, 69, 76, 71, 79, 72], Hand::Right, 0.5),
        melody(&[48, 43, 45, 41, 43, 36, 40, 45, 48], Hand::Left, 0.5),
        melody(&[64, 63, 64, 63, 64, 59, 62, 60, 57], Hand::Right, 0.25),
        octaves(&[48, 50, 52, 53, 55, 53, 52, 50, 48], Hand::Left, 0.5),
    ]
}

fn trained_prior() -> crate::NgramPrior {
    let mut prior = crate::NgramPrior::new();
    let finger = |n: u8| Finger::from_number(n).expect("1..=5");
    for (hand, line) in [
        (Hand::Right, [(60, 1), (62, 2), (64, 3), (65, 1), (67, 2), (69, 3), (71, 4), (72, 5)]),
        (Hand::Left, [(48, 5), (50, 4), (52, 3), (53, 2), (55, 1), (57, 3), (59, 2), (60, 1)]),
    ] {
        let placements: Vec<Placement> =
            line.iter().map(|(midi, f)| Placement::new(*midi, finger(*f))).collect();
        prior.observe(hand, &placements);
    }
    prior
}

fn runs(score: &Score) -> Vec<Solution> {
    let plain = FingeringOptions::default();
    let prior = trained_prior();
    let with_model = FingeringOptions { prior_scale: 1.5, ..FingeringOptions::default() };
    vec![finger_score(score, &plain), finger_score_with_prior(score, &with_model, Some(&prior))]
}

#[test]
fn no_alternative_is_ever_cheaper_than_what_was_chosen() {
    for score in passages() {
        for solution in runs(&score) {
            for e in &solution.explanations {
                assert!(
                    e.margin >= -1e-3,
                    "note {:?} was given {:?}, and swapping it would have been cheaper by {}",
                    e.note,
                    e.finger,
                    -e.margin
                );
            }
        }
    }
}

#[test]
fn the_explained_costs_add_up_to_the_total() {
    for score in passages().into_iter().take(4) {
        for solution in runs(&score) {
            let explained: f32 = solution.explanations.iter().map(|e| e.cost.total()).sum();
            let tolerance = 1e-3 * solution.cost.abs().max(1.0);
            assert!(
                (explained - solution.cost).abs() <= tolerance,
                "the notes explain {explained} of a total of {}",
                solution.cost
            );
        }
    }
}

fn octaves(pitches: &[u8], hand: Hand, beats_per_note: f64) -> Score {
    let mut score = melody(pitches, hand, beats_per_note);
    let upper: Vec<Note> = score
        .notes
        .iter()
        .enumerate()
        .map(|(i, n)| Note { id: NoteId((pitches.len() + i) as u32), midi: n.midi + 12, ..n.clone() })
        .collect();
    score.notes.extend(upper);
    score.finalise();
    score
}

fn fingers_of(score: &Score, solution: &Solution) -> Vec<u8> {
    let mut out: Vec<_> =
        score.notes.iter().filter_map(|n| solution.finger_of(n.id).map(|f| (n.onset, f.number()))).collect();
    out.sort_by_key(|(t, _)| *t);
    out.into_iter().map(|(_, f)| f).collect()
}

const C_MAJOR_UP: [u8; 8] = [60, 62, 64, 65, 67, 69, 71, 72];

#[test]
fn candidates_respect_finger_order_within_a_chord() {
    let options = FingeringOptions::default();
    for hand in Hand::ALL {
        let solver = HandSolver::new(hand, &options, None, None);
        let event = Event {
            notes: vec![60, 64, 67],
            ids: vec![NoteId(0), NoteId(1), NoteId(2)],
            struck: vec![true; 3],
            onset_seconds: 0.0,
            pinned: vec![None; 3],
        };
        let candidates = solver.candidates(&event);
        assert_eq!(candidates.len(), 10, "{hand:?}");
        for c in &candidates {
            let indices: Vec<_> = c.fingers.iter().map(|f| f.index()).collect();
            let ordered = match hand {
                Hand::Right => indices.windows(2).all(|w| w[0] < w[1]),
                Hand::Left => indices.windows(2).all(|w| w[0] > w[1]),
            };
            assert!(ordered, "{hand:?} produced tangled fingers {indices:?}");
        }
    }
}

#[test]
fn a_pinned_finger_is_obeyed() {
    let options = FingeringOptions::default();
    let solver = HandSolver::new(Hand::Right, &options, None, None);
    let event = Event {
        notes: vec![60, 64],
        ids: vec![NoteId(0), NoteId(1)],
        struck: vec![true; 2],
        onset_seconds: 0.0,
        pinned: vec![Some(Finger::Index), None],
    };
    for c in solver.candidates(&event) {
        assert_eq!(c.fingers[0], Finger::Index);
    }
}

#[test]
fn the_right_hand_fingers_a_rising_c_major_scale_the_way_a_teacher_would() {
    let score = melody(&C_MAJOR_UP, Hand::Right, 0.5);
    let solution = finger_score(&score, &FingeringOptions::default());
    assert_eq!(fingers_of(&score, &solution), vec![1, 2, 3, 1, 2, 3, 4, 5]);
}

#[test]
fn the_left_hand_fingers_a_rising_c_major_scale_the_way_a_teacher_would() {
    let score = melody(&C_MAJOR_UP, Hand::Left, 0.5);
    let solution = finger_score(&score, &FingeringOptions::default());
    assert_eq!(fingers_of(&score, &solution), vec![5, 4, 3, 2, 1, 3, 2, 1]);
}

#[test]
fn a_descending_scale_mirrors_the_ascending_one() {
    let mut descending = C_MAJOR_UP;
    descending.reverse();
    let score = melody(&descending, Hand::Right, 0.5);
    let solution = finger_score(&score, &FingeringOptions::default());
    assert_eq!(fingers_of(&score, &solution), vec![5, 4, 3, 2, 1, 3, 2, 1]);
}

#[test]
fn a_five_note_run_within_the_hand_needs_no_thumb_crossing() {
    let score = melody(&[60, 62, 64, 65, 67], Hand::Right, 0.5);
    let solution = finger_score(&score, &FingeringOptions::default());
    assert_eq!(fingers_of(&score, &solution), vec![1, 2, 3, 4, 5]);
}

#[test]
fn a_triad_is_fingered_one_three_five() {
    let mut score = Score::default();
    for (i, midi) in [60u8, 64, 67].iter().enumerate() {
        score.notes.push(Note {
            id: NoteId(i as u32),
            midi: *midi,
            onset: 0,
            duration: TICKS_PER_QUARTER as i64,
            hand: Some(Hand::Right),
            chord: i > 0,
            velocity: 72,
            source: SourceRef::Midi { track: 0, event: i },
            ..Default::default()
        });
    }
    score.finalise();
    let solution = finger_score(&score, &FingeringOptions::default());
    let mut fingers: Vec<_> = solution.fingerings.iter().map(|f| f.finger.number()).collect();
    fingers.sort_unstable();
    assert_eq!(fingers, vec![1, 3, 5]);
}

#[test]
fn tied_notes_are_not_fingered_twice() {
    let mut score = melody(&[60, 60], Hand::Right, 1.0);
    score.notes[0].tie.start = true;
    score.notes[1].tie.stop = true;
    let solution = finger_score(&score, &FingeringOptions::default());
    assert_eq!(solution.fingerings.len(), 1, "the tie continuation got its own finger");
}

#[test]
fn an_editorial_fingering_is_kept_and_the_rest_fits_around_it() {
    let mut score = melody(&C_MAJOR_UP, Hand::Right, 0.5);
    score.notes[0].given_finger = Some(Finger::Index);
    let solution = finger_score(&score, &FingeringOptions::default());
    let fingers = fingers_of(&score, &solution);
    assert_eq!(fingers[0], 2, "the editorial fingering was overwritten");
    assert_eq!(fingers.len(), C_MAJOR_UP.len());
}

#[test]
fn every_note_gets_exactly_one_finger() {
    let score = melody(&[60, 63, 66, 70, 73, 68, 61, 59], Hand::Right, 0.25);
    let solution = finger_score(&score, &FingeringOptions::default());
    assert_eq!(solution.fingerings.len(), score.notes.len());
    assert_eq!(solution.explanations.len(), score.notes.len());
    let mut ids: Vec<_> = solution.fingerings.iter().map(|f| f.note).collect();
    ids.dedup();
    assert_eq!(ids.len(), score.notes.len());
}

#[test]
fn explanations_name_the_rules_that_fired() {
    let score = melody(&[65, 66, 67], Hand::Right, 0.5);
    let solution = finger_score(&score, &FingeringOptions::default());
    assert!(solution.explanations.iter().all(|e| e.reachable));
    assert!(solution.explanations.iter().all(|e| e.margin >= 0.0));
}

#[test]
fn tempo_changes_the_answer() {
    let wide = [60u8, 84, 60, 84, 60, 84];
    let slow = finger_score(&melody(&wide, Hand::Right, 2.0), &FingeringOptions::default());
    let fast = finger_score(&melody(&wide, Hand::Right, 0.1), &FingeringOptions::default());
    assert_eq!(slow.fingerings.len(), wide.len());
    assert_eq!(fast.fingerings.len(), wide.len());
    assert!(fast.cost > slow.cost, "slow {} fast {}", slow.cost, fast.cost);
}

#[test]
fn a_two_octave_arpeggio_is_fingered_the_way_the_charts_print_it() {
    for (tonic, register) in [(0u8, 60u8), (5, 60), (7, 60), (3, 60)] {
        let base = tonic + register;
        let up: Vec<u8> = [0u8, 4, 7, 12, 16, 19, 24].iter().map(|step| base + step).collect();
        let mut pitches = up.clone();
        pitches.extend(up.iter().rev().skip(1));

        let score = melody(&pitches, Hand::Right, 0.5);
        let right = fingers_of(&score, &finger_score(&score, &FingeringOptions::default()));
        assert_eq!(right, vec![1, 2, 3, 1, 2, 3, 5, 3, 2, 1, 3, 2, 1], "right hand, arpeggio on {base}");

        let score = melody(&pitches, Hand::Left, 0.5);
        let left = fingers_of(&score, &finger_score(&score, &FingeringOptions::default()));
        let want = if tonic == 3 {
            vec![5, 3, 2, 1, 3, 2, 1, 2, 3, 1, 2, 3, 5]
        } else {
            vec![5, 4, 2, 1, 4, 2, 1, 2, 4, 1, 2, 4, 5]
        };
        assert_eq!(left, want, "left hand, arpeggio on {base}");
    }
}

#[test]
fn a_chromatic_run_takes_the_third_finger_on_every_black_key() {
    for (hand, seconds) in [(Hand::Right, [0u8, 5]), (Hand::Left, [4, 11])] {
        let score = melody(&(60..=72).collect::<Vec<u8>>(), hand, 0.25);
        let solution = finger_score_consensus(&score, &FingeringOptions::default(), None);
        let fingers = fingers_of(&score, &solution);
        for (i, midi) in (60u8..=72).enumerate().take(12).skip(1) {
            let wanted = match midi % 12 {
                1 | 3 | 6 | 8 | 10 => 3,
                p if seconds.contains(&p) => 2,
                _ => 1,
            };
            assert_eq!(fingers[i], wanted, "{hand:?} at midi {midi}");
        }
    }
}

#[test]
fn a_run_of_octaves_is_the_thumb_and_the_little_finger_all_the_way() {
    let score = octaves(&[72, 74, 76, 77, 79, 81], Hand::Right, 0.5);
    let options = FingeringOptions::default();
    let solution = finger_score_consensus(&score, &options, None);

    let mut by_onset: std::collections::BTreeMap<i64, Vec<u8>> = Default::default();
    for note in &score.notes {
        if let Some(finger) = solution.finger_of(note.id) {
            by_onset.entry(note.onset).or_default().push(finger.number());
        }
    }
    for (onset, mut shape) in by_onset {
        shape.sort_unstable();
        assert_eq!(shape, vec![1, 5], "the octave at tick {onset}");
    }
}

#[test]
fn the_same_score_is_fingered_the_same_way_every_time() {
    let score = melody(&[60, 64, 67, 72, 71, 69, 65, 62, 60], Hand::Right, 0.25);
    let options = FingeringOptions::default();
    let first = fingers_of(&score, &finger_score_consensus(&score, &options, None));
    for _ in 0..8 {
        let again = fingers_of(&score, &finger_score_consensus(&score, &options, None));
        assert_eq!(first, again);
    }

    let agreement = Agreement::across_published(&score, &options, None);
    assert_eq!(agreement.voters, 4, "all four sets should have been heard from");
}

#[test]
fn asking_for_the_default_gets_the_consensus_and_not_a_lookalike() {
    let score = melody(&[63, 65, 69, 66, 62, 65, 62, 66, 60], Hand::Right, 0.25);
    let options = FingeringOptions::default();
    assert_eq!(options.rule_set, RuleSet::Consensus, "the default is the consensus");
    assert_eq!(
        fingers_of(&score, &finger_score(&score, &options)),
        fingers_of(&score, &finger_score_consensus(&score, &options, None))
    );
}

#[test]
fn a_scale_played_legato_keeps_its_taught_fingering() {
    let c_major = [60, 62, 64, 65, 67, 69, 71, 72, 74, 76, 77, 79, 81, 83, 84];
    let detached = melody(&c_major, Hand::Right, 0.5);
    let mut legato = detached.clone();
    let overlap = ticks_for(&legato, 0.100);
    for note in &mut legato.notes {
        note.duration += overlap;
    }
    legato.finalise();
    let options = FingeringOptions::default();
    let plain = fingers_of(&detached, &finger_score(&detached, &options));
    let joined = fingers_of(&legato, &finger_score(&legato, &options));
    assert_eq!(plain, joined, "overlapping each note by 100 ms changed the fingering");
}
