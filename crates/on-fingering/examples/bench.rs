//! Every measurement this engine can make without a licensed corpus.
//!
//! Agreement with expert fingerings on real repertoire needs PIG, which is
//! registration-gated. Everything here is measured against the taught scale tables
//! instead — the one body of piano fingering with a settled answer — plus two
//! experiments that need no ground truth at all, because they ask whether the engine
//! *responds* to something rather than whether it is right.
//!
//! Those two are the interesting ones. Every published fingering system is
//! score-symbolic: pitches, intervals, key colour. None of them has a hand, so none of
//! them can have an opinion about tempo or hand size. This engine does, and that is
//! testable without any annotations at all.
//!
//!     cargo run -p on-fingering --example bench

use std::collections::HashMap;

use on_fingering::rules::{RuleSet, RuleWeights};
use on_fingering::scales::{scale_fingerings, Taught};
use on_fingering::{finger_score_with_prior, FingeringOptions, Ruler};
use on_hand::{Hand, HandProfile, HandSize};
use on_score::{Note, NoteId, Score, SourceRef, TieState, TICKS_PER_QUARTER};

const MAJOR: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];

/// Two octaves up and back down, which is how scales are practised.
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

/// A one-hand line at a given speed, in notes per second.
fn melody(pitches: &[u8], hand: Hand, per_second: f64) -> Score {
    let step = (TICKS_PER_QUARTER as f64 * (2.0 / per_second)).max(1.0) as i64;
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

fn fingers_of(score: &Score, options: &FingeringOptions) -> Vec<u8> {
    let solution = finger_score_with_prior(score, options, None);
    let mut out = vec![0u8; score.notes.len()];
    for f in &solution.fingerings {
        out[f.note.0 as usize] = f.finger.number();
    }
    out
}

/// Agreement with the taught tables over all twelve keys and both hands.
fn taught_agreement(options: &FingeringOptions) -> (usize, usize) {
    let (mut same, mut of) = (0usize, 0usize);
    for tonic in 0..12u8 {
        for hand in [Hand::Right, Hand::Left] {
            let base = if hand == Hand::Right { 60 } else { 36 };
            let pitches = scale_pitches(tonic, base);
            let score = melody(&pitches, hand, 8.0);
            let onsets: Vec<f64> = score.notes.iter().map(|n| n.onset_seconds).collect();
            let optional: Vec<Option<u8>> = pitches.iter().map(|p| Some(*p)).collect();
            let want: HashMap<usize, Taught> = scale_fingerings(hand, &optional, &onsets);
            let got = fingers_of(&score, options);
            for (index, taught) in &want {
                of += 1;
                if got[*index] == taught.finger.number() {
                    same += 1;
                }
            }
        }
    }
    (same, of)
}

fn pct(v: (usize, usize)) -> f64 {
    if v.1 == 0 {
        0.0
    } else {
        100.0 * v.0 as f64 / v.1 as f64
    }
}

/// How many of two fingerings differ.
fn differing(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

fn show(fingers: &[u8], take: usize) -> String {
    fingers
        .iter()
        .take(take)
        .map(|f| f.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() {
    let base = FingeringOptions::for_hand(HandProfile::default());

    println!("== 1. Ablation: what each layer is worth");
    println!("Agreement with the taught scale tables, 12 keys, both hands, 2 octaves\n");
    println!("{:<44} {:>10} {:>9}", "configuration", "agreement", "delta");

    let mut previous: Option<f64> = None;
    let row = |label: &str, options: &FingeringOptions, previous: &mut Option<f64>| {
        let p = pct(taught_agreement(options));
        let delta = previous.map(|prev| p - prev);
        println!(
            "{:<44} {:>9.1}% {:>9}",
            label,
            p,
            delta.map(|d| format!("{d:+.1}")).unwrap_or_else(|| "-".into())
        );
        *previous = Some(p);
    };

    let mut bare = base.clone();
    bare.pattern_scale = 0.0;
    bare.biomech.posture = 0.0;
    bare.biomech.motion = 0.0;
    row("published rules alone", &bare, &mut previous);

    let mut with_hand = base.clone();
    with_hand.pattern_scale = 0.0;
    row("+ biomechanics (22-DOF hand, strain, IK)", &with_hand, &mut previous);

    row("+ taught conventions (as shipped)", &base, &mut previous);

    println!("\n== 2. The physical ruler, isolated");
    println!("Same search, the only difference being how an interval is measured\n");
    for (label, ruler) in [
        ("semitone counting (as the papers do)", Ruler::Chromatic),
        ("millimetres on a real keyboard", Ruler::Physical),
    ] {
        let mut o = base.clone();
        o.ruler = ruler;
        o.pattern_scale = 0.0;
        println!("  {label:<44} {:>9.1}%", pct(taught_agreement(&o)));
    }

    println!("\n== 3. The four published rule sets, one harness, one metric");
    println!("Same search, same hand model, same evaluation; only the rules differ\n");
    for set in [
        RuleSet::Parncutt,
        RuleSet::Jacobs,
        RuleSet::Balliauw,
        RuleSet::Badgerow,
        RuleSet::Consensus,
    ] {
        let mut o = base.clone();
        o.rule_set = set;
        o.rule_weights = RuleWeights::default();
        o.pattern_scale = 0.0;
        println!("  {:<44} {:>9.1}%", format!("{set:?}"), pct(taught_agreement(&o)));
    }

    println!("
== 4. Tempo dependence");
    println!("The cost function is tempo-aware: the work of reconfiguring the hand is");
    println!("divided by the time available and squared. Conventions off, so nothing");
    println!("else decides. Broken octaves, where the choice is stretch against shift
");
    let mut bare = base.clone();
    bare.pattern_scale = 0.0;
    let octaves = vec![60u8, 72, 62, 74, 64, 76, 65, 77, 67, 79];
    println!(
        "  {:>8}  {:>9} {:>9} {:>9}  {}",
        "notes/s", "rules", "motion", "motion %", "fingering"
    );
    for rate in [0.5f64, 1.0, 2.0, 4.0, 8.0, 16.0, 24.0] {
        let score = melody(&octaves, Hand::Right, rate);
        let solution = finger_score_with_prior(&score, &bare, None);
        let (mut rules, mut motion) = (0.0f32, 0.0f32);
        for e in &solution.explanations {
            rules += e.cost.rules;
            motion += e.cost.motion;
        }
        let mut got = vec![0u8; octaves.len()];
        for f in &solution.fingerings {
            got[f.note.0 as usize] = f.finger.number();
        }
        let total = rules.abs() + motion.abs();
        println!(
            "  {rate:>8.1}  {rules:>9.2} {motion:>9.2} {:>8.1}%  {}",
            if total > 0.0 { 100.0 * motion.abs() / total } else { 0.0 },
            show(&got, octaves.len())
        );
    }

    println!("
== 5. Hand size");
    println!("The same notes for hands of different reach, conventions off. A spread of");
    println!("tenths, where what the hand can hold is what decides
");
    let spread = vec![60u8, 76, 62, 77, 64, 79, 65, 81, 67, 83];
    println!("  {:<14} {:>12}  {}", "hand", "differs by", "fingering");
    let medium = {
        let mut o = FingeringOptions::for_hand(HandProfile::from_size(HandSize::Medium));
        o.pattern_scale = 0.0;
        fingers_of(&melody(&spread, Hand::Right, 4.0), &o)
    };
    for size in [
        HandSize::ExtraSmall,
        HandSize::Small,
        HandSize::Medium,
        HandSize::Large,
        HandSize::ExtraLarge,
    ] {
        let mut o = FingeringOptions::for_hand(HandProfile::from_size(size));
        o.pattern_scale = 0.0;
        let got = fingers_of(&melody(&spread, Hand::Right, 4.0), &o);
        println!(
            "  {:<14} {:>12}  {}",
            format!("{size:?}"),
            format!("{} of {}", differing(&medium, &got), got.len()),
            show(&got, got.len())
        );
    }
}