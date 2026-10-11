use on_train::corpus::Corpus;

fn main() -> anyhow::Result<()> {
    let (mut right, mut total, mut switches, mut theirs) = (0usize, 0usize, 0usize, 0usize);
    for root in std::env::args().skip(1) {
        for piece in Corpus::at(&root)?.load()? {
            let (mut score, _) = on_train::eval::rebuild(&piece);
            let truth: Vec<_> = score.notes.iter().map(|n| n.hand).collect();
            let mut seen: std::collections::HashMap<u8, (f64, Option<on_hand::Hand>)> = Default::default();
            for note in &score.notes {
                if let Some((at, hand)) = seen.insert(note.midi, (note.onset_seconds, note.hand)) {
                    theirs += usize::from(note.onset_seconds - at < 0.6 && hand != note.hand);
                }
            }
            for note in &mut score.notes {
                note.staff = None;
                note.hand = None;
            }
            on_score::assign_hands(&mut score, &Default::default());
            right += score.notes.iter().zip(&truth).filter(|(n, t)| n.hand == **t).count();
            total += score.notes.len();
            let mut last: std::collections::HashMap<u8, (f64, Option<on_hand::Hand>)> = Default::default();
            for note in &score.notes {
                if let Some((at, hand)) = last.insert(note.midi, (note.onset_seconds, note.hand)) {
                    switches += usize::from(note.onset_seconds - at < 0.6 && hand != note.hand);
                }
            }
        }
    }
    println!("{total} notes, {:.2}% in the pianist's hand, {switches} quick hand switches on a repeated key (the pianists: {theirs})", 100.0 * right as f64 / total.max(1) as f64);
    Ok(())
}
