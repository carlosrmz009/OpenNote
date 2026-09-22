//! Where the time goes when a score is fingered, by note count.
//!
//! Fingering is the engine's expensive step and the tuner does nothing else, so what a
//! generation costs is what this prints. If the cost does not fall to nothing as the
//! score does, the difference is work being redone for every score rather than once.
//!
//! The second half prints what a generation really looks like: many different scores,
//! one after another, on one thread. The hand geometry is shared between them, so the
//! first score pays for the chord shapes it meets and the rest mostly do not.

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

/// A polyphonic score: a right-hand figure over left-hand chords, `seed` deciding which
/// key it sits in and how the figure moves, so no two of these meet the same shapes.
fn piece(seed: usize, bars: usize) -> Score {
    let step = i64::from(TICKS_PER_QUARTER) / 4;
    let mut score = Score::default();
    let key = (seed * 7 % 12) as u8;
    let push = |score: &mut Score, midi: u8, onset: i64, duration: i64, hand: Hand| {
        score.notes.push(Note {
            id: NoteId(0),
            midi,
            onset,
            duration,
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
            source: SourceRef::Midi { track: 0, event: 0 },
        });
    };
    for bar in 0..bars {
        let at = bar as i64 * step * 8;
        // Left hand: a chord held for the bar, walking down the circle of fifths.
        let root = 36 + key + ((bar * 5) % 15) as u8;
        for interval in [0, 7, 10] {
            push(&mut score, root + interval, at, step * 8, Hand::Left);
        }
        // Right hand: eight notes, the shape of the figure turning over with the seed.
        for i in 0..8u8 {
            let shape = [0, 4, 7, 11, 12, 9, 5, 2][(i as usize + seed) % 8];
            push(
                &mut score,
                60 + key + shape + (bar % 3) as u8 * 2,
                at + i as i64 * step,
                step,
                Hand::Right,
            );
        }
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

    println!("
thirty different scores in a row, as a tuner generation does:");
    let scores: Vec<Score> = (0..30).map(|seed| piece(seed, 16)).collect();
    let mut total = 0.0;
    for (i, score) in scores.iter().enumerate() {
        let started = Instant::now();
        std::hint::black_box(finger_score(score, &options));
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        total += ms;
        if i < 3 || (i + 1) % 10 == 0 {
            println!("  score {:3} ({} notes): {ms:8.2} ms", i + 1, score.notes.len());
        }
    }
    println!("  mean over all thirty: {:8.2} ms", total / scores.len() as f64);
}
