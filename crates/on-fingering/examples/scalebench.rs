//! Left to the rules and the biomechanics alone, does the engine find the scale
//! fingerings pianists are taught?
//!
//! Scales are the one body of piano fingering with a settled answer, which makes them
//! the only benchmark available without a licensed corpus. The reference here is the
//! project's own [`on_fingering::scales`] tables — the ones the golden tests already
//! check against the method books — rather than a set retyped for the occasion, so
//! this grades the *solver* and not a second copy of the data.
//!
//! Two numbers per key:
//!
//! * **taught** — what the program ships. The standard patterns are recognised and
//!   given a bonus, because a pianist plays a scale from training rather than by
//!   rediscovering it every time.
//! * **model** — the same search with that bonus switched off. This is the honest
//!   measure of the rules and the hand model, and it is the number to watch when
//!   either of them changes. It should never be *worse* after a change meant to make
//!   the model more accurate.
//!
//!     cargo run -p on-fingering --example scalebench

use std::collections::HashMap;

use on_fingering::scales::{scale_fingerings, Taught};
use on_fingering::{finger_score, FingeringOptions};
use on_hand::{Hand, HandProfile};
use on_score::{Note, NoteId, Score, SourceRef, TieState, TICKS_PER_QUARTER};

/// Semitones above the tonic, for a major scale.
const MAJOR: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];

const NAMES: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];

/// A run of a major scale, ascending then back down, which is how they are practised
/// and how the turn at the top gets exercised.
fn scale_pitches(tonic: u8, base: u8, octaves: usize) -> Vec<u8> {
    let mut up = Vec::new();
    for octave in 0..octaves {
        for step in MAJOR {
            up.push(base + tonic + step + 12 * octave as u8);
        }
    }
    up.push(base + tonic + 12 * octaves as u8);
    let mut out = up.clone();
    out.extend(up.iter().rev().skip(1).copied());
    out
}

fn melody(pitches: &[u8], hand: Hand) -> Score {
    let step = TICKS_PER_QUARTER as i64 / 2;
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

/// How many of the notes the tables have an opinion about the engine fingers their way.
fn agreement(
    pitches: &[u8],
    want: &HashMap<usize, Taught>,
    hand: Hand,
    options: &FingeringOptions,
) -> (usize, usize) {
    let score = melody(pitches, hand);
    let solution = finger_score(&score, options);
    let mut got = vec![0u8; pitches.len()];
    for f in &solution.fingerings {
        got[f.note.0 as usize] = f.finger.number();
    }
    let same = want
        .iter()
        .filter(|(index, taught)| got[**index] == taught.finger.number())
        .count();
    (same, want.len())
}

/// Print one scale note by note: what the books say, and what the model says without
/// their help. Run as `scalebench Eb right`.
fn show(tonic: u8, hand: Hand, octaves: usize, options: &FingeringOptions) {
    let base = match hand {
        Hand::Right => 60,
        Hand::Left => 36,
    };
    let pitches = scale_pitches(tonic, base, octaves);
    let optional: Vec<Option<u8>> = pitches.iter().map(|p| Some(*p)).collect();
    let want = scale_fingerings(hand, &optional);
    let score = melody(&pitches, hand);
    let solution = finger_score(&score, options);
    let mut got = vec![0u8; pitches.len()];
    for f in &solution.fingerings {
        got[f.note.0 as usize] = f.finger.number();
    }
    println!("{} major, {hand:?} hand
", NAMES[tonic as usize]);
    print!("  note  ");
    for m in &pitches {
        print!("{:>3}", NAMES[(*m % 12) as usize]);
    }
    println!();
    print!("  books ");
    for i in 0..pitches.len() {
        match want.get(&i) {
            Some(t) => print!("{:>3}", t.finger.number()),
            None => print!("  ."),
        }
    }
    println!();
    print!("  model ");
    for (i, g) in got.iter().enumerate() {
        let agree = want.get(&i).map(|t| t.finger.number() == *g).unwrap_or(true);
        print!("{:>3}", if agree { format!("{g}") } else { format!("{g}*") });
    }
    println!("

  (* where the model disagrees with the books)");
}

fn main() {
    let mut args = std::env::args().skip(1);
    if let (Some(key), Some(hand)) = (args.next(), args.next()) {
        let tonic = NAMES.iter().position(|n| n.eq_ignore_ascii_case(&key)).unwrap_or(0) as u8;
        let hand = if hand.starts_with('l') { Hand::Left } else { Hand::Right };
        let bare = FingeringOptions {
            pattern_scale: 0.0,
            ..FingeringOptions::for_hand(HandProfile::default())
        };
        show(tonic, hand, 2, &bare);
        return;
    }
    let taught_options = FingeringOptions::for_hand(HandProfile::default());
    let model_options = FingeringOptions { pattern_scale: 0.0, ..taught_options.clone() };
    let octaves = 2;

    println!("Agreement with the taught scale fingerings, two octaves up and back down.");
    println!("Graded only on the notes the tables have an opinion about.\n");
    println!("{:>5}   {:>13}   {:>13}", "key", "right", "left");
    println!("{:>5}   {:>13}   {:>13}", "", "taught model", "taught model");

    let mut totals = [(0usize, 0usize); 4];
    let mut worst: Vec<(f64, String)> = Vec::new();
    for tonic in 0..12u8 {
        let mut cells = Vec::new();
        for (slot, hand) in [Hand::Right, Hand::Left].into_iter().enumerate() {
            let base = match hand {
                Hand::Right => 60,
                Hand::Left => 36,
            };
            let pitches = scale_pitches(tonic, base, octaves);
            let optional: Vec<Option<u8>> = pitches.iter().map(|p| Some(*p)).collect();
            let want = scale_fingerings(hand, &optional);
            for (which, options) in [(0, &taught_options), (1, &model_options)] {
                let (same, of) = agreement(&pitches, &want, hand, options);
                let index = slot * 2 + which;
                totals[index].0 += same;
                totals[index].1 += of;
                cells.push(format!("{same}/{of}"));
                if which == 1 && of > 0 && same < of {
                    worst.push((
                        same as f64 / of as f64,
                        format!("{} {hand:?}: {same}/{of}", NAMES[tonic as usize]),
                    ));
                }
            }
        }
        println!(
            "{:>5}   {:>6} {:>6}   {:>6} {:>6}",
            NAMES[tonic as usize], cells[0], cells[1], cells[2], cells[3]
        );
    }

    println!();
    let pct = |(a, b): (usize, usize)| {
        if b == 0 {
            100.0
        } else {
            100.0 * a as f64 / b as f64
        }
    };
    println!(
        "  right hand   taught {:>5.1}%   model alone {:>5.1}%",
        pct(totals[0]),
        pct(totals[1])
    );
    println!(
        "  left hand    taught {:>5.1}%   model alone {:>5.1}%",
        pct(totals[2]),
        pct(totals[3])
    );
    let both_taught = (totals[0].0 + totals[2].0, totals[0].1 + totals[2].1);
    let both_model = (totals[1].0 + totals[3].0, totals[1].1 + totals[3].1);
    println!(
        "  overall      taught {:>5.1}%   model alone {:>5.1}%",
        pct(both_taught),
        pct(both_model)
    );

    // How hard the convention has to push before it wins everywhere. The chromatic
    // bonus was calibrated exactly this way — run it up until the answer stops
    // changing — and the scale bonus never was.
    println!("
  what the taught patterns cost to enforce:");
    println!("    {:>8}  {:>9}", "scale", "agreement");
    for scale in [0.0f32, 1.0, 2.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0] {
        let options = FingeringOptions { pattern_scale: scale, ..taught_options.clone() };
        let mut hit = (0usize, 0usize);
        for tonic in 0..12u8 {
            for hand in [Hand::Right, Hand::Left] {
                let base = match hand {
                    Hand::Right => 60,
                    Hand::Left => 36,
                };
                let pitches = scale_pitches(tonic, base, octaves);
                let optional: Vec<Option<u8>> = pitches.iter().map(|p| Some(*p)).collect();
                let want = scale_fingerings(hand, &optional);
                let (same, of) = agreement(&pitches, &want, hand, &options);
                hit.0 += same;
                hit.1 += of;
            }
        }
        println!("    {scale:>8.0}  {:>8.1}%", pct(hit));
    }

    worst.sort_by(|a, b| a.0.total_cmp(&b.0));
    if !worst.is_empty() {
        println!("\n  where the model alone does worst:");
        for (_, line) in worst.iter().take(6) {
            println!("    {line}");
        }
    }
}
