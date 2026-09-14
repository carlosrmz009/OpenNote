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
                if note.hand != Some(*want) {
                    println!(
                        "DIFF {:8.2} {:3} got {:?} want {want:?}",
                        note.onset_seconds, note.midi, note.hand.unwrap()
                    );
                }
            }
        }
        let wrong = score
            .notes
            .iter()
            .zip(&truth)
            .filter(|(note, want)| note.hand != Some(**want))
            .count();
        let n = score.notes.len();
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
fn truth_hands(score: &Score) -> Option<Vec<Hand>> {
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
                    .map(|n| if n.staff == Some(upper) { Hand::Right } else { Hand::Left })
                    .collect(),
            );
        }
    }

    let track_of = |note: &on_score::Note| match note.source {
        SourceRef::Midi { track, .. } => Some(track),
        _ => None,
    };
    let mut tracks: Vec<usize> = score.notes.iter().filter_map(track_of).collect();
    if tracks.len() != score.notes.len() {
        return None;
    }
    tracks.sort_unstable();
    tracks.dedup();
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
            .map(|n| if track_of(n) == Some(right) { Hand::Right } else { Hand::Left })
            .collect(),
    )
}
