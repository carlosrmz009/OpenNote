//! `opennote annotate` — the main job.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::Args;
use on_engrave::{engrave, EngraveOptions};
use on_hand::Hand;
use on_score::hands::HandAssignment;

use crate::ModelArgs;

/// Add fingerings to a score.
#[derive(Debug, Args)]
pub struct AnnotateArgs {
    /// The score to read: `.musicxml`, `.mxl`, `.xml`, `.mid` or `.midi`.
    pub input: PathBuf,

    /// Where to write the annotated score. Defaults to the input name with
    /// `-fingered` appended.
    #[arg(long, short)]
    pub output: Option<PathBuf>,

    /// Also engrave the annotated score. The extension picks the format:
    /// `.pdf`, `.svg` or `.png`.
    #[arg(long)]
    pub engrave: Option<PathBuf>,

    /// Lay the engraved music out as one continuous system rather than pages.
    #[arg(long)]
    pub continuous: bool,

    /// Print the chosen fingering to the terminal as well.
    #[arg(long)]
    pub print: bool,

    #[command(flatten)]
    pub model: ModelArgs,
}

/// Run the command.
pub fn run(args: AnnotateArgs) -> Result<()> {
    if !args.input.exists() {
        bail!("{} does not exist", args.input.display());
    }
    let options = args.model.options()?;

    let mut input = on_score::Document::open(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;

    // MIDI carries no staves, so the hands have to be worked out; MusicXML usually
    // says, and `assign_hands` uses that when it can.
    let assignment = HandAssignment {
        profile: options.profile.clone(),
        ..Default::default()
    };
    on_score::assign_hands(input.score_mut(), &assignment);

    let score = input.score();
    let notes = score.notes.len();
    let prior = args.model.prior()?;
    let solution = on_fingering::finger_score_with_prior(
        score,
        &options,
        prior.as_deref().map(|p| p as &dyn on_fingering::FingeringPrior),
    );

    let unreachable = solution.explanations.iter().filter(|e| !e.reachable).count();
    let title = score.title.clone().unwrap_or_else(|| "untitled".into());
    println!(
        "{title}: {notes} notes, {} fingered ({} left, {} right)",
        solution.fingerings.len(),
        score.hand_notes(Hand::Left).count(),
        score.hand_notes(Hand::Right).count(),
    );
    if unreachable > 0 {
        // "shape" rather than "chord": what the hand is asked to hold at a moment is
        // everything still sounding, not only what is struck together, and the usual
        // answer to a shape a hand cannot hold is the sustain pedal rather than a
        // different fingering.
        println!(
            "  warning: {unreachable} notes fall in shapes this hand cannot hold — \
             counting what it is still holding, not only what it strikes. Try a larger \
             --hand-size, or the passage may want the pedal or redistributing"
        );
    }

    if args.print {
        print_fingering(score, &solution);
    }

    let output = args.output.unwrap_or_else(|| default_output(&args.input));

    // A MIDI file carries sound and not notation, so a fingered MIDI is the only thing
    // that can come back out of one. Both of the things a name ending `.musicxml` would
    // promise have to be refused rather than half-done: the file itself, which would
    // otherwise be MIDI bytes under a name nothing could read back, this program
    // included; and the engraving, which has no notation to draw.
    if matches!(input, on_score::Document::Midi(_)) {
        let asked_for_notation = output
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| {
                matches!(e.to_ascii_lowercase().as_str(), "musicxml" | "mxl" | "xml")
            });
        if asked_for_notation {
            bail!(
                "{} is a MIDI file, which carries no notation, so it cannot be written \
                 out as MusicXML. Give the output a `.mid` name, or convert the input \
                 to MusicXML first.",
                args.input.display()
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
            let written = on_engrave::annotate_and_save(&mut document, &solution.fingerings, &output)?;
            println!("  wrote {} ({written} fingerings)", output.display());

            if let Some(target) = &args.engrave {
                let engrave_options =
                    EngraveOptions { paginate: !args.continuous, ..Default::default() };
                let files = engrave(&output, target, &engrave_options)?;
                for file in files {
                    println!("  engraved {}", file.display());
                }
            }
        }
        on_score::Document::Midi(document) => {
            document.write_annotated(&output, &solution.fingerings)?;
            println!("  wrote {}", output.display());
        }
    }

    Ok(())
}

/// Where to write when the user did not say.
fn default_output(input: &Path) -> PathBuf {
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("score");
    let extension = input
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("musicxml");
    input.with_file_name(format!("{stem}-fingered.{extension}"))
}

/// Show the fingering as a pianist would read it: one line per hand, bar by bar.
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

/// One hand's fingering as a pianist would read it: a number per key press, chord
/// members stacked with slashes.
///
/// `None` when the hand has nothing to play.
fn fingering_line(
    score: &on_score::Score,
    solution: &on_fingering::Solution,
    hand: Hand,
) -> Option<String> {
    let mut line = String::new();
    let mut last_onset = None;
    // A pitch written twice at one instant is one key press, and the search gives both
    // copies the same finger. Printing both puts the same number in the chord twice,
    // which reads as a mistake rather than as the doubling it is.
    let mut already_struck: Vec<u8> = Vec::new();
    for note in score.hand_notes(hand) {
        // A note that only continues a tie is not struck and has no finger of its own.
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
        // MIDI files carry the same pitch twice at the same moment all the time, from a
        // doubled voice or an overlapping repeat. It is one key press, and the search
        // gives both copies the same finger; printing both put the same number in the
        // chord twice, which reads as a mistake rather than as the doubling it is.
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
        };

        assert_eq!(
            fingering_line(&score, &solution, Hand::Right).as_deref(),
            Some("1/3/5")
        );
    }

    #[test]
    fn a_hand_with_nothing_to_play_has_no_line() {
        let score = Score::default();
        let solution = Solution { fingerings: Vec::new(), cost: 0.0, explanations: Vec::new() };
        assert_eq!(fingering_line(&score, &solution, Hand::Left), None);
    }
}

