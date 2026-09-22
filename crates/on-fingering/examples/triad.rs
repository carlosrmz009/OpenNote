//! Where does the cost of a chord shape come from?

use on_fingering::{finger_score, FingeringOptions};
use on_hand::Hand;
use on_score::{Note, NoteId, Score, SourceRef, TieState, TICKS_PER_QUARTER};

fn note(id: u32, midi: u8, onset: i64, duration: i64, hand: Hand, chord: bool) -> Note {
    Note {
        id: NoteId(id), midi, onset, duration,
        onset_seconds: 0.0, duration_seconds: 0.0,
        staff: None, voice: None, hand: Some(hand),
        tie: TieState::default(), grace: false, chord,
        velocity: 72, given_finger: None,
        source: SourceRef::Midi { track: 0, event: id as usize },
    }
}

fn main() {
    let q = TICKS_PER_QUARTER as i64;
    for hand in Hand::ALL {
        for lead_in in [None, Some(84u8), Some(60u8)] {
            let mut score = Score::default();
            let mut id = 0;
            let mut start = 0;
            if let Some(midi) = lead_in {
                score.notes.push(note(id, midi, 0, q, hand, false));
                id += 1;
                start = q;
            }
            for (i, midi) in [60u8, 64, 67].iter().enumerate() {
                score.notes.push(note(id, *midi, start, 4 * q, hand, i > 0));
                id += 1;
            }
            score.finalise();
            let solution = finger_score(&score, &FingeringOptions::default());
            let chord: Vec<_> = score.notes.iter().filter(|n| n.onset == start).collect();
            let fingers: Vec<u8> = chord
                .iter()
                .filter_map(|n| solution.finger_of(n.id).map(|f| f.number()))
                .collect();
            let e = solution
                .explanations
                .iter()
                .find(|e| e.note == chord[0].id)
                .unwrap();
            println!(
                "{hand:?} lead-in {:<9} -> {fingers:?}  rules {:6.2} posture {:6.2} pattern {:6.2} motion {:6.2}  margin {:5.2}",
                format!("{lead_in:?}"),
                e.cost.rules, e.cost.posture, e.cost.pattern, e.cost.motion, e.margin
            );
        }
    }
}
