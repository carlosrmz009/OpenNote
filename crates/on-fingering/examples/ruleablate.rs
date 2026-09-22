//! Which rules are helping, and which are costing points?
//!
//! Zeroes one rule's weight at a time and re-measures agreement with the taught scale
//! tables. A rule whose removal *improves* the score is either wrong, wrongly weighted,
//! or saying the same thing as another rule that is already saying it.
//!
//! Conventions are off throughout: with them on the tables win everything and no rule
//! can be seen to matter.
//!
//!     cargo run -p on-fingering --example ruleablate

use std::collections::HashMap;

use on_fingering::rules::{Rule, RuleSet};
use on_fingering::scales::{scale_fingerings, Taught};
use on_fingering::{finger_score_with_prior, FingeringOptions};
use on_hand::{Hand, HandProfile};
use on_score::{Note, NoteId, Score, SourceRef, TieState, TICKS_PER_QUARTER};

const MAJOR: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];

fn scale_pitches(tonic: u8, base: u8) -> Vec<u8> {
    let mut up = Vec::new();
    for octave in 0..2u8 {
        for step in MAJOR {
            up.push(base + tonic + step + 12 * octave);
        }
    }
    up.push(base + tonic + 24);
    let mut out = up.clone();
    out.extend(up.iter().rev().skip(1).copied());
    out
}

fn melody(pitches: &[u8], hand: Hand) -> Score {
    let step = TICKS_PER_QUARTER as i64 / 4;
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

fn agreement(options: &FingeringOptions) -> f64 {
    let (mut same, mut of) = (0usize, 0usize);
    for tonic in 0..12u8 {
        for hand in [Hand::Right, Hand::Left] {
            let base = if hand == Hand::Right { 60 } else { 36 };
            let pitches = scale_pitches(tonic, base);
            let score = melody(&pitches, hand);
            let onsets: Vec<f64> = score.notes.iter().map(|n| n.onset_seconds).collect();
            let optional: Vec<Option<u8>> = pitches.iter().map(|p| Some(*p)).collect();
            let want: HashMap<usize, Taught> = scale_fingerings(hand, &optional, &onsets);
            let solution = finger_score_with_prior(&score, options, None);
            let mut got = vec![0u8; pitches.len()];
            for f in &solution.fingerings {
                got[f.note.0 as usize] = f.finger.number();
            }
            for (index, taught) in &want {
                of += 1;
                if got[*index] == taught.finger.number() {
                    same += 1;
                }
            }
        }
    }
    if of == 0 {
        0.0
    } else {
        100.0 * same as f64 / of as f64
    }
}

fn main() {
    let mut base = FingeringOptions::for_hand(HandProfile::default());
    base.pattern_scale = 0.0;

    for set in [RuleSet::Consensus, RuleSet::Balliauw] {
        let mut options = base.clone();
        options.rule_set = set;
        let reference = agreement(&options);
        println!("\n{set:?}: {reference:.1}% with every rule in place");
        println!("  {:<32} {:>9} {:>9}", "rule removed", "agreement", "change");

        let mut rows: Vec<(f64, Rule, f64)> = Vec::new();
        for rule in set.rules() {
            let mut o = options.clone();
            let mut w = o.rule_weights;
            w.0[rule.index()] = 0.0;
            o.rule_weights = w;
            let score = agreement(&o);
            rows.push((score - reference, *rule, score));
        }
        // Biggest improvement first: those are the rules costing points.
        rows.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (delta, rule, score) in rows {
            let mark = if delta > 0.05 {
                "  <- removing it helps"
            } else if delta < -0.05 {
                ""
            } else {
                "  (no effect)"
            };
            println!("  {:<32} {score:>8.1}% {delta:>+8.1}{mark}", format!("{rule:?}"));
        }
    }
}
