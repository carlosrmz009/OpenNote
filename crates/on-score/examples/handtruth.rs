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

        for note in &mut score.notes {
            note.staff = None;
            note.hand = None;
        }
        on_score::assign_hands(&mut score, &HandAssignment::default());

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
