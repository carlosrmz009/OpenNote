//! `opennote annotate`: fingers a score and writes the fingering into a copy of it, and
//! optionally engraves that copy.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::Args;
use on_engrave::{engrave, EngraveOptions};
use on_hand::Hand;

use crate::ModelArgs;

/// Add fingerings to a score.
#[derive(Debug, Args)]
pub struct AnnotateArgs {
    /// The scores to read: `.musicxml`, `.mxl`, `.xml`, `.mid` or `.midi`. Each is written
    /// beside itself with `-fingered` added to its name.
    #[arg(required = true, num_args = 1..)]
    pub inputs: Vec<PathBuf>,

    /// Where to write the annotated score, instead of beside it. Only with one score.
    #[arg(long, short)]
    pub output: Option<PathBuf>,

    /// Also engrave the annotated score. The extension picks the format: `.pdf`, `.svg` or
    /// `.png`. Only with one score.
    #[arg(long)]
    pub engrave: Option<PathBuf>,

    /// Write over an annotated score that is already there.
    #[arg(long)]
    pub force: bool,

    /// Lay the engraved music out as one continuous system rather than pages.
    #[arg(long)]
    pub continuous: bool,

    /// Print the chosen fingering to the terminal as well.
    #[arg(long)]
    pub print: bool,

    #[command(flatten)]
    pub model: ModelArgs,
}

pub fn run(args: AnnotateArgs) -> Result<()> {
    if args.inputs.len() > 1 && (args.output.is_some() || args.engrave.is_some()) {
        bail!("--output and --engrave name one file, so they go with one score at a time");
    }
    let mut failed = 0;
    for input in &args.inputs {
        let output = args.output.clone().unwrap_or_else(|| default_output(input));
        if let Err(error) = annotate(input, &output, &args) {
            if args.inputs.len() == 1 {
                return Err(error);
            }
            eprintln!("{}: {error:#}", input.display());
            failed += 1;
        }
    }
    if failed > 0 {
        bail!("{failed} of {} scores could not be annotated", args.inputs.len());
    }
    Ok(())
}

fn annotate(path: &Path, output: &Path, args: &AnnotateArgs) -> Result<()> {
    if output.exists() && !args.force {
        bail!("{} is already there; pass --force to write over it", output.display());
    }
    let crate::Fingered { document: input, solution, .. } = args.model.finger(path)?;
    let score = input.score();
    let notes = score.notes.len();

    let unreachable = solution.explanations.iter().filter(|e| !e.reachable).count();
    let title = score.title.clone().unwrap_or_else(|| "untitled".into());
    println!(
        "{title}: {notes} notes, {} fingered ({} left, {} right)",
        solution.fingerings.len(),
        score.hand_notes(Hand::Left).count(),
        score.hand_notes(Hand::Right).count(),
    );
    if unreachable > 0 {
        println!(
            "  warning: {unreachable} notes fall in shapes this hand cannot hold — \
             counting what it is still holding, not only what it strikes. Try a larger \
             --hand-size, or the passage may want the pedal or redistributing"
        );
    }

    if args.print {
        print_fingering(score, &solution);
    }

    if matches!(input, on_score::Document::Midi(_)) {
        let asked_for_notation = output
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "musicxml" | "mxl" | "xml"));
        if asked_for_notation {
            bail!(
                "{} is a MIDI file, which carries no notation, so it cannot be written \
                 out as MusicXML. Give the output a `.mid` name, or convert the input \
                 to MusicXML first.",
                path.display()
            );
        }
        if args.engrave.is_some() {
            bail!(
                "engraving needs notation, and a MIDI file has none. \
                 Convert it to MusicXML first, or drop --engrave."
            );
        }
    }

    match input {
        on_score::Document::MusicXml(mut document) => {
            let written = on_engrave::annotate_and_save(&mut document, &solution.fingerings, output)?;
            println!("  wrote {} ({written} fingerings)", output.display());

            if let Some(target) = &args.engrave {
                let engrave_options = EngraveOptions { paginate: !args.continuous, ..Default::default() };
                let files = engrave(output, target, &engrave_options)?;
                for file in files {
                    println!("  engraved {}", file.display());
                }
            }
        }
        on_score::Document::Midi(document) => {
            document.write_annotated(output, &solution.fingerings)?;
            println!("  wrote {}", output.display());
        }
    }

    Ok(())
}

fn default_output(input: &Path) -> PathBuf {
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("score");
    let extension = input.extension().and_then(|s| s.to_str()).unwrap_or("musicxml");
    input.with_file_name(format!("{stem}-fingered.{extension}"))
}

fn print_fingering(score: &on_score::Score, solution: &on_fingering::Solution) {
    for hand in [Hand::Right, Hand::Left] {
        let Some(line) = fingering_line(score, solution, hand) else {
            continue;
        };
        let label = match hand {
            Hand::Right => "RH",
            Hand::Left => "LH",
        };
        println!("  {label}: {line}");
    }
}

fn fingering_line(score: &on_score::Score, solution: &on_fingering::Solution, hand: Hand) -> Option<String> {
    let mut line = String::new();
    let mut last_onset = None;
    let mut already_struck: Vec<u8> = Vec::new();
    for note in score.hand_notes(hand) {
        let Some(finger) = solution.finger_of(note.id) else {
            continue;
        };
        if last_onset == Some(note.onset) {
            if already_struck.contains(&note.midi) {
                continue;
            }
            line.push('/');
        } else {
            if last_onset.is_some() {
                line.push(' ');
            }
            already_struck.clear();
        }
        already_struck.push(note.midi);
        line.push_str(&finger.number().to_string());
        last_onset = Some(note.onset);
    }
    (!line.is_empty()).then_some(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use on_fingering::Solution;
    use on_hand::Finger;
    use on_score::{Fingering, Note, NoteId, Score, SourceRef, TieState, TICKS_PER_QUARTER};

    fn note(id: u32, midi: u8, onset: i64) -> Note {
        Note {
            id: NoteId(id),
            midi,
            onset,
            duration: TICKS_PER_QUARTER as i64,
            onset_seconds: 0.0,
            duration_seconds: 0.0,
            staff: None,
            voice: None,
            hand: Some(Hand::Right),
            tie: TieState::default(),
            grace: false,
            chord: false,
            velocity: 64,
            given_finger: None,
            source: SourceRef::Midi { track: 0, event: id as usize },
        }
    }

    #[test]
    fn a_pitch_doubled_at_one_instant_is_printed_once() {
        let mut score = Score {
            notes: vec![note(0, 60, 0), note(1, 64, 0), note(2, 64, 0), note(3, 67, 0)],
            ..Default::default()
        };
        score.finalise();

        let solution = Solution {
            fingerings: vec![
                Fingering::new(NoteId(0), Finger::Thumb),
                Fingering::new(NoteId(1), Finger::Middle),
                Fingering::new(NoteId(2), Finger::Middle),
                Fingering::new(NoteId(3), Finger::Little),
            ],
            cost: 0.0,
            explanations: Vec::new(),
            agreement: None,
            path: Vec::new(),
        };

        assert_eq!(fingering_line(&score, &solution, Hand::Right).as_deref(), Some("1/3/5"));
    }

    #[test]
    fn a_hand_with_nothing_to_play_has_no_line() {
        let score = Score::default();
        let solution = Solution {
            fingerings: Vec::new(),
            cost: 0.0,
            explanations: Vec::new(),
            agreement: None,
            path: Vec::new(),
        };
        assert_eq!(fingering_line(&score, &solution, Hand::Left), None);
    }
}
