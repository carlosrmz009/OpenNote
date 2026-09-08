//! `opennote explain` — why each note got the finger it did.
//!
//! A fingering you cannot interrogate is a fingering you have to take on trust.
//! This prints the reasoning: which rules charged something, what the posture cost,
//! and how close the runner-up was.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Args;
use on_hand::Hand;
use on_score::hands::HandAssignment;

use crate::ModelArgs;

/// Explain a fingering.
#[derive(Debug, Args)]
pub struct ExplainArgs {
    /// The score to read.
    pub input: PathBuf,

    /// Only show notes at or after this time, in seconds.
    #[arg(long, default_value_t = 0.0)]
    pub from: f64,

    /// How many notes to explain.
    #[arg(long, default_value_t = 24)]
    pub limit: usize,

    /// Only show notes where the choice was close, which are the ones worth a
    /// second opinion.
    #[arg(long)]
    pub close_calls: bool,

    #[command(flatten)]
    pub model: ModelArgs,
}

/// A decision this close to its runner-up could reasonably have gone either way.
const CLOSE_CALL: f32 = 0.25;

/// Run the command.
pub fn run(args: ExplainArgs) -> Result<()> {
    if !args.input.exists() {
        bail!("{} does not exist", args.input.display());
    }
    let options = args.model.options()?;
    let mut input = on_score::Document::open(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let assignment = HandAssignment {
        profile: options.profile.clone(),
        ..Default::default()
    };
    on_score::assign_hands(input.score_mut(), &assignment);

    let score = input.score();
    let prior = args.model.prior()?;
    let prior_ref = prior.as_deref().map(|p| p as &dyn on_fingering::FingeringPrior);
    let solution = on_fingering::finger_score_consensus(score, &options, prior_ref);

    println!(
        "{} — total cost {:.2} over {} notes",
        score.title.clone().unwrap_or_else(|| "untitled".into()),
        solution.cost,
        solution.fingerings.len()
    );

    // How much the four published sets actually agreed. Worth printing: it is the one
    // honest measure of how much work the consensus is doing, and if they never
    // disagreed there would be no point combining them.
    let (unanimous, split) = on_fingering::Agreement::across_published(score, &options, prior_ref)
        .tally();
    let total = unanimous + split;
    if total > 0 {
        let share = 100.0 * unanimous as f32 / total as f32;
        println!(
            "the four published rule sets agreed on {unanimous} of {total} notes ({share:.0}%), and split on {split}"
        );
    }
    println!();

    let mut shown = 0;
    for explanation in &solution.explanations {
        if shown >= args.limit {
            break;
        }
        let note = score.note(explanation.note);
        if note.onset_seconds < args.from {
            continue;
        }
        if args.close_calls && explanation.margin > CLOSE_CALL {
            continue;
        }
        shown += 1;

        let hand = match note.hand {
            Some(Hand::Left) => "LH",
            _ => "RH",
        };
        println!(
            "{:>7.2}s  {hand} {:<4} finger {}   {}",
            note.onset_seconds,
            pitch_name(note.midi),
            explanation.finger.number(),
            confidence(explanation.margin),
        );

        if !explanation.reachable {
            println!("           ! this chord is out of reach for the given hand size");
        }
        if explanation.posture_strain > 2.0 {
            println!(
                "           the shape costs {:.2} to hold, which is a lot",
                explanation.posture_strain
            );
        }
        for (rule, cost) in explanation.rules.iter().take(3) {
            println!("           {:>5.2}  {}", cost, rule.describe());
        }
    }

    if shown == 0 {
        println!("(nothing to show — try --from 0 or drop --close-calls)");
    }
    Ok(())
}

/// How sure the solver was, in words.
fn confidence(margin: f32) -> &'static str {
    match margin {
        m if m <= 0.05 => "(a coin toss — the alternative costs the same)",
        m if m <= CLOSE_CALL => "(close; another finger would nearly do)",
        m if m <= 2.0 => "(clear)",
        _ => "(forced — nothing else works here)",
    }
}

/// A pitch as a musician would write it, using sharps.
fn pitch_name(midi: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = midi as i32 / 12 - 1;
    format!("{}{}", NAMES[(midi % 12) as usize], octave)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitches_are_named_the_way_musicians_write_them() {
        assert_eq!(pitch_name(60), "C4");
        assert_eq!(pitch_name(61), "C#4");
        assert_eq!(pitch_name(21), "A0");
        assert_eq!(pitch_name(108), "C8");
    }

    #[test]
    fn confidence_reads_sensibly_across_the_range() {
        assert!(confidence(0.0).contains("coin toss"));
        assert!(confidence(0.1).contains("close"));
        assert!(confidence(1.0).contains("clear"));
        assert!(confidence(50.0).contains("forced"));
    }
}
