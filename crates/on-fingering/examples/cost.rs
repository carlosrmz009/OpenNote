//! Where the time goes when a score is fingered, by note count.
//!
//! Fingering is the engine's expensive step and the tuner does nothing else, so what a
//! generation costs is what this prints. If the cost does not fall to nothing as the
//! score does, the difference is work being redone for every score rather than once.

use std::time::Instant;

use on_fingering::{finger_score, FingeringOptions};
use on_hand::Hand;
use on_score::{Note, NoteId, Score, SourceRef, TieState, TICKS_PER_QUARTER};

fn melody(count: usize) -> Score {
    let step = i64::from(TICKS_PER_QUARTER) / 2;
    let mut score = Score::default();
    for i in 0..count {
        score.notes.push(Note {
            id: NoteId(i as u32),
            midi: 60 + (i as u8 * 5) % 19,
            onset: i as i64 * step,
            duration: step,
            onset_seconds: 0.0,
            duration_seconds: 0.0,
            staff: None,
            voice: None,
            hand: Some(Hand::Right),
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

fn main() {
    let options = FingeringOptions::default();
    for count in [1, 10, 50, 100, 200, 400] {
        let score = melody(count);
        let runs = if count > 100 { 5 } else { 20 };
        let started = Instant::now();
        for _ in 0..runs {
            std::hint::black_box(finger_score(&score, &options));
        }
        let each = started.elapsed().as_secs_f64() / runs as f64;
        println!("{count:4} notes: {:8.2} ms  ({:6.3} ms a note)", each * 1000.0, each * 1000.0 / count as f64);
    }
}
