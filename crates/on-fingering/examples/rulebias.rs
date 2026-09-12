//! Which rules charge the taught fingering more than they charge the engine's own?
//!
//! A rule that consistently costs more on the answer in the method books than on the
//! answer the search prefers is pushing away from the books. That is not proof it is
//! wrong — the taught answer is not free, and some awkwardness is genuinely accepted
//! at the top of a scale because there is no alternative — but it is where to look,
//! and it is how the inverted `FourOnBlack` was found.
//!
//! Both fingerings are scored by the same scorer, so the comparison is of rules and
//! not of configurations.
//!
//!     cargo run -p on-fingering --example rulebias

use on_fingering::rules::{Placement, Rule, RuleScorer, RuleSet, RuleWeights, Trigram};
use on_fingering::scales::scale_fingerings;
use on_fingering::{finger_score_with_prior, FingeringOptions};
use on_hand::{Finger, Hand, HandProfile};
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

/// Total raw charge of one rule over the part of a line the tables speak for.
///
/// Only windows whose three notes are all inside `covered`. The tables have an opinion
/// about the interior of a run and not its ends, so a line assembled from the taught
/// answer inside and the model's answer outside has a seam — and a window straddling
/// that seam belongs to neither fingering. Counting those made `RepeatedFinger` look
/// like the worst offender in the set, entirely from repeats invented at the join.
fn charge(
    scorer: &RuleScorer,
    hand: Hand,
    pitches: &[u8],
    fingers: &[u8],
    rule: Rule,
    covered: &dyn Fn(usize) -> bool,
) -> f32 {
    let mut total = 0.0;
    let at = |i: usize| -> Option<Placement> {
        let f = Finger::from_number(*fingers.get(i)?)?;
        Some(Placement::new(*pitches.get(i)?, f))
    };
    for i in 1..pitches.len().saturating_sub(1) {
        if !covered(i - 1) || !covered(i) || !covered(i + 1) {
            continue;
        }
        let Some(current) = at(i) else { continue };
        let t = Trigram { hand, prev: at(i - 1), current, next: at(i + 1) };
        total += scorer.score_with(&t, &[rule]).total();
    }
    total
}

fn main() {
    let mut options = FingeringOptions::for_hand(HandProfile::default());
    options.pattern_scale = 0.0;
    let set = RuleSet::Consensus;
    options.rule_set = set;
    let scorer = RuleScorer::new(
        set,
        RuleWeights::default(),
        options.span_model.table(),
        options.ruler,
    );

    // Per rule: what it charges the taught fingering, and what it charges the model's.
    let mut taught_total = vec![0.0f32; 32];
    let mut model_total = vec![0.0f32; 32];
    let mut graded = 0usize;

    for tonic in 0..12u8 {
        for hand in [Hand::Right, Hand::Left] {
            let base = if hand == Hand::Right { 60 } else { 36 };
            let pitches = scale_pitches(tonic, base);
            let score = melody(&pitches, hand);
            let onsets: Vec<f64> = score.notes.iter().map(|n| n.onset_seconds).collect();
            let optional: Vec<Option<u8>> = pitches.iter().map(|p| Some(*p)).collect();
            let want = scale_fingerings(hand, &optional, &onsets);
            if want.is_empty() {
                continue;
            }

            // The model's own answer.
            let solution = finger_score_with_prior(&score, &options, None);
            let mut model = vec![0u8; pitches.len()];
            for f in &solution.fingerings {
                model[f.note.0 as usize] = f.finger.number();
            }
            // The taught answer, where the tables have an opinion; the model's
            // elsewhere, so the two lines differ only where the books speak.
            let mut taught = model.clone();
            for (index, t) in &want {
                taught[*index] = t.finger.number();
            }
            if taught == model {
                continue;
            }
            graded += 1;

            let covered = |i: usize| want.contains_key(&i);
            for rule in set.rules() {
                taught_total[rule.index()] +=
                    charge(&scorer, hand, &pitches, &taught, *rule, &covered);
                model_total[rule.index()] +=
                    charge(&scorer, hand, &pitches, &model, *rule, &covered);
            }
        }
    }

    println!(
        "Across {graded} scales where the engine and the books disagree.\n\
         A rule charging the taught answer more than its own is pushing away from it.\n"
    );
    println!(
        "  {:<32} {:>9} {:>9} {:>9}",
        "rule", "taught", "model", "bias"
    );
    let mut rows: Vec<(f32, Rule)> = set
        .rules()
        .iter()
        .map(|r| (taught_total[r.index()] - model_total[r.index()], *r))
        .collect();
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (bias, rule) in rows {
        let mark = if bias > 1.0 {
            "  <- charges the books"
        } else if bias < -1.0 {
            "  (charges the model)"
        } else {
            ""
        };
        println!(
            "  {:<32} {:>9.1} {:>9.1} {:>+9.1}{mark}",
            format!("{rule:?}"),
            taught_total[rule.index()],
            model_total[rule.index()],
            bias
        );
    }
}
