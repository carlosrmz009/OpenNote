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
use on_score::hands::{truth_hands, HandAssignment};
use on_score::{MidiDocument, MusicXmlDocument};

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
