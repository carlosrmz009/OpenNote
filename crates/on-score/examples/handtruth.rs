//! How often the hand search agrees with whoever wrote the music down.
//!
//! The search only runs when a source does not say which hand plays what. Plenty of
//! sources do say, and those are ground truth for it: engraved music puts the right
//! hand on staff 1 and the left on staff 2, and a two-track piano MIDI is the same
//! split by another name. Strip the answer out, run the search, and count.
//!
//! Neither is infallible — an arranger decides where to put a note that could go
//! either way, and sometimes decides oddly. What the number is good for is movement:
//! a change to the search that sends it down has broken something.
//!
//! A MIDI's tracks are a sequencer's, not a pianist's, and they show it: one file has
//! the right hand playing G1. Engraved staves are the trustworthy half. Nothing here is
//! committed with the repository — point it at your own scores.
//!
//! Held-out scores matter more than the number, because the search has been tuned
//! against the same handful of files for a while now. Two-track piano MIDI is common
//! enough that they are not hard to come by.
use on_hand::Hand;
use on_score::hands::HandAssignment;
use on_score::{MidiDocument, MusicXmlDocument, Score, SourceRef};

fn main() -> anyhow::Result<()> {
    let mut total = (0usize, 0usize);
    for path in std::env::args().skip(1) {
        let name = std::path::Path::new(&path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let midi = path.to_lowercase().ends_with(".mid") || path.to_lowercase().ends_with(".midi");
        let mut score = if midi {
            MidiDocument::read(&path)?.score().clone()
        } else {
            MusicXmlDocument::read(&path)?.score().clone()
        };

        let Some(truth) = truth_hands(&score) else {
            println!("{name:<28} no hand split in the source, skipped");
            continue;
        };

        // Take the answer away and make it work for it.
        for note in &mut score.notes {
            note.staff = None;
            note.hand = None;
        }
        on_score::assign_hands(&mut score, &HandAssignment::default());

        // `ON_SHOW=1` lists every disagreement, which is how you find out whether they
        // cluster in one passage or are scattered.
        if std::env::var("ON_SHOW").is_ok() {
            for (note, want) in score.notes.iter().zip(&truth) {
                let Some(want) = want else { continue };
                if note.hand != Some(*want) {
                    println!(
                        "DIFF {:8.2} {:3} got {:?} want {want:?}",
                        note.onset_seconds, note.midi, note.hand.unwrap()
                    );
                }
            }
        }
        let judged: Vec<_> = score
            .notes
            .iter()
            .zip(&truth)
            .filter_map(|(note, want)| want.map(|w| (note, w)))
            .collect();
        let wrong = judged.iter().filter(|(note, want)| note.hand != Some(*want)).count();
        let n = judged.len();
        total = (total.0 + n - wrong, total.1 + n);
        println!(
            "{name:<28} {n:5} notes  {wrong:4} disagree  {:6.2}% right",
            100.0 * (n - wrong) as f32 / n as f32
        );
    }
    if total.1 > 0 {
        println!(
            "{:<28} {:5} notes  {:4} disagree  {:6.2}% right",
            "TOTAL",
            total.1,
            total.1 - total.0,
            100.0 * total.0 as f32 / total.1 as f32
        );
    }
    Ok(())
}

/// The hand each note belongs to according to the source, if the source says.
///
/// Engraved music says outright. A MIDI says it by putting the hands on separate
/// tracks, which is what a piano transcription does — so a file whose notes live on
/// exactly two tracks is read that way, the higher-sounding track being the right
/// hand. Anything else (one track, a sequencer's dozen) is not a hand split and is
/// not treated as one.
fn truth_hands(score: &Score) -> Option<Vec<Option<Hand>>> {
    if score.notes.iter().all(|n| n.staff.is_some()) {
        let mut staves: Vec<u8> = score.notes.iter().filter_map(|n| n.staff).collect();
        staves.sort_unstable();
        staves.dedup();
        if staves.len() == 2 {
            let upper = staves[0];
            return Some(
                score
                    .notes
                    .iter()
                    .map(|n| Some(if n.staff == Some(upper) { Hand::Right } else { Hand::Left }))
                    .collect(),
            );
        }
    }

    let track_of = |note: &on_score::Note| match note.source {
        SourceRef::Midi { track, .. } => Some(track),
        _ => None,
    };
    let all: Vec<usize> = score.notes.iter().filter_map(track_of).collect();
    if all.len() != score.notes.len() {
        return None;
    }
    // Tracks holding a handful of notes are not a hand. Sequencers leave them behind —
    // a stray pair of notes on a third track was enough to disqualify a whole rag —
    // and the notes on them are left unjudged rather than guessed at.
    let mut tracks: Vec<usize> = Vec::new();
    for track in {
        let mut seen = all.clone();
        seen.sort_unstable();
        seen.dedup();
        seen
    } {
        if all.iter().filter(|t| **t == track).count() * 100 >= all.len() {
            tracks.push(track);
        }
    }
    if tracks.len() != 2 {
        return None;
    }
    let mean = |track: usize| {
        let pitches: Vec<f32> = score
            .notes
            .iter()
            .filter(|n| track_of(n) == Some(track))
            .map(|n| f32::from(n.midi))
            .collect();
        pitches.iter().sum::<f32>() / pitches.len() as f32
    };
    let right = if mean(tracks[0]) > mean(tracks[1]) { tracks[0] } else { tracks[1] };
    Some(
        score
            .notes
            .iter()
            .map(|n| match track_of(n) {
                Some(t) if t == right => Some(Hand::Right),
                Some(t) if tracks.contains(&t) => Some(Hand::Left),
                _ => None,
            })
            .collect(),
    )
}
