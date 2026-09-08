//! How many scale runs the "anchored elsewhere" guard declines to finger, in a piece.
//!
//! Scans for stepwise runs the same way `find_scale_runs` does, then reports the ones
//! it now refuses: runs that begin and end on the same note, where that note is not the
//! tonic their pitch content implies. If this is zero for a piece, the guard changed
//! nothing there.
use on_fingering::scales::{find_scale_runs, key_of, MIN_SCALE_RUN};
use on_hand::Hand;
use on_score::hands::HandAssignment;
use on_score::MidiDocument;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("usage: suppressed <midi>");
    let mut file = MidiDocument::read(&path)?;
    on_score::assign_hands(file.score_mut(), &HandAssignment::default());
    let score = file.score();

    for hand in Hand::ALL {
        let pitches: Vec<Option<u8>> = score
            .notes
            .iter()
            .filter(|n| n.hand == Some(hand))
            .map(|n| Some(n.midi))
            .collect();

        // Every stepwise run, before the guard.
        let mut all = 0usize;
        let mut suppressed = Vec::new();
        let mut index = 0;
        while index < pitches.len() {
            let mut end = index + 1;
            let mut direction = 0i32;
            let mut previous = pitches[index].unwrap();
            while end < pitches.len() {
                let pitch = pitches[end].unwrap();
                let step = pitch as i32 - previous as i32;
                if !(1..=2).contains(&step.abs()) {
                    break;
                }
                if direction == 0 {
                    direction = step.signum();
                } else if step.signum() != direction {
                    break;
                }
                previous = pitch;
                end += 1;
            }
            let length = end - index;
            if length >= MIN_SCALE_RUN {
                let notes: Vec<u8> = pitches[index..end].iter().flatten().copied().collect();
                if let Some(tonic) = key_of(&notes) {
                    all += 1;
                    let (first, last) = (notes[0] % 12, notes[notes.len() - 1] % 12);
                    if first == last && first != tonic {
                        suppressed.push((index, length, first, tonic));
                    }
                }
            }
            index = if length > 1 { end } else { index + 1 };
        }
        let kept = find_scale_runs(&pitches).len();
        println!(
            "{hand:?}: {all} scale runs by content, {kept} still fingered, {} suppressed",
            suppressed.len()
        );
        for (at, len, anchor, tonic) in suppressed.iter().take(5) {
            println!("    note {at}, {len} long, runs {anchor} to {anchor} but reads as key {tonic}");
        }
    }
    Ok(())
}
