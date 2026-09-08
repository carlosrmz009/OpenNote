//! Price the taught scale fingering against the one the model prefers.
//!
//! Where the model disagrees with the books about a scale it usually picks a *rotation*
//! of the same cycle — the right seven-finger pattern starting in the wrong place. This
//! pins the taught answer with `given_finger`, solves for the model's own answer with
//! the pattern bonus off, and reports what each costs and which rules account for the
//! difference.
//!
//!     cargo run -p on-fingering --example whyrotation -- Eb right

use on_fingering::scales::scale_fingerings;
use on_fingering::{finger_score, FingeringOptions, Rule};
use on_hand::{Finger, Hand, HandProfile};
use on_score::{Note, NoteId, Score, SourceRef, TieState, TICKS_PER_QUARTER};

const MAJOR: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
const NAMES: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];

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

fn melody(pitches: &[u8], hand: Hand, pinned: &[Option<Finger>]) -> Score {
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
            given_finger: pinned.get(i).copied().flatten(),
            source: SourceRef::Midi { track: 0, event: i },
        });
    }
    score.finalise();
    score
}

/// Total per-rule charges over a whole solution.
fn per_rule(solution: &on_fingering::Solution) -> Vec<(Rule, f32)> {
    let mut totals: std::collections::BTreeMap<String, (Rule, f32)> = Default::default();
    for e in &solution.explanations {
        for (rule, amount) in &e.rules {
            let slot = totals.entry(format!("{rule:?}")).or_insert((*rule, 0.0));
            slot.1 += amount;
        }
    }
    let mut out: Vec<(Rule, f32)> = totals.into_values().collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let key = args.next().unwrap_or_else(|| "Eb".into());
    let hand = if args.next().is_some_and(|h| h.starts_with('l')) {
        Hand::Left
    } else {
        Hand::Right
    };
    let tonic = NAMES.iter().position(|n| n.eq_ignore_ascii_case(&key)).unwrap_or(3) as u8;
    let base = match hand {
        Hand::Right => 60,
        Hand::Left => 36,
    };
    let pitches = scale_pitches(tonic, base, 2);
    let optional: Vec<Option<u8>> = pitches.iter().map(|p| Some(*p)).collect();
    let books = scale_fingerings(hand, &optional);

    // No pattern bonus in either run: the question is what the *rules and the hand*
    // make of the two fingerings, not what the bonus is worth.
    let options = FingeringOptions {
        pattern_scale: 0.0,
        ..FingeringOptions::for_hand(HandProfile::default())
    };

    let free = finger_score(&melody(&pitches, hand, &[]), &options);
    let pinned: Vec<Option<Finger>> = (0..pitches.len())
        .map(|i| books.get(&i).map(|t| t.finger))
        .collect();
    let taught = finger_score(&melody(&pitches, hand, &pinned), &options);

    println!("{} major, {hand:?} hand, two octaves up and back down\n", NAMES[tonic as usize]);
    println!("  model's own choice   cost {:>9.2}", free.cost);
    println!("  the books' answer    cost {:>9.2}", taught.cost);
    println!(
        "  the books cost {:+.2}, which is {} the model prefers its own\n",
        taught.cost - free.cost,
        if taught.cost > free.cost { "why" } else { "NOT why" }
    );

    let a = per_rule(&free);
    let b = per_rule(&taught);
    println!("  {:>28}  {:>9}  {:>9}  {:>9}", "rule", "model", "books", "books-model");
    let mut names: Vec<String> = a.iter().chain(b.iter()).map(|(r, _)| format!("{r:?}")).collect();
    names.sort();
    names.dedup();
    let mut rows: Vec<(String, f32, f32)> = names
        .into_iter()
        .map(|n| {
            let x = a.iter().find(|(r, _)| format!("{r:?}") == n).map_or(0.0, |(_, v)| *v);
            let y = b.iter().find(|(r, _)| format!("{r:?}") == n).map_or(0.0, |(_, v)| *v);
            (n, x, y)
        })
        .collect();
    rows.sort_by(|p, q| (q.2 - q.1).abs().total_cmp(&(p.2 - p.1).abs()));
    for (name, x, y) in rows.iter().take(8) {
        println!("  {name:>28}  {x:>9.2}  {y:>9.2}  {:>+9.2}", y - x);
    }

    // Which notes of the taught fingering are charged as unplayable, and what the
    // finger before them was. If these are all thumb crossings, the rule is firing on
    // a movement rather than on a hand shape.
    println!("
  notes of the books' fingering charged as impractical:");
    let finger_at = |i: usize| taught.finger_of(NoteId(i as u32)).map(|f| f.number());
    let mut shown = 0;
    for e in &taught.explanations {
        let amount: f32 = e
            .rules
            .iter()
            .filter(|(r, _)| matches!(r, Rule::Impractical))
            .map(|(_, v)| *v)
            .sum();
        if amount <= 0.0 {
            continue;
        }
        let i = e.note.0 as usize;
        let from = if i > 0 { finger_at(i - 1) } else { None };
        println!(
            "    note {i:>2}: {} -> {}   fingers {:?} -> {}   charged {amount:.2}",
            if i > 0 { NAMES[(pitches[i - 1] % 12) as usize] } else { "-" },
            NAMES[(pitches[i] % 12) as usize],
            from,
            e.finger.number(),
        );
        shown += 1;
        if shown >= 8 {
            break;
        }
    }
}
