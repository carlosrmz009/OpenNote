//! Length histogram of stepwise runs, on pitch alone.
//!
//! No gap rule and no minimum, deliberately. This is the distribution `MIN_SCALE_RUN`
//! is chosen against, and measuring it through the filter being calibrated would be
//! circular — a gradual slowing truncates runs, so runs measured that way look shorter
//! than they are, and the threshold would be fitted to its own artefact.
//!
//!     cargo run -p on-fingering --example hist -- path/to/piece.mid
use on_fingering::scales::key_of;
use on_hand::Hand;
use on_score::hands::HandAssignment;
use on_score::MidiDocument;
use std::collections::BTreeMap;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("usage: hist <midi>");
    let mut file = MidiDocument::read(&path)?;
    on_score::assign_hands(file.score_mut(), &HandAssignment::default());
    let score = file.score();
    let mut hist: BTreeMap<usize, usize> = BTreeMap::new();

    for hand in Hand::ALL {
        let pitches: Vec<u8> = score
            .notes
            .iter()
            .filter(|n| n.hand == Some(hand))
            .map(|n| n.midi)
            .collect();
        let mut i = 0;
        while i < pitches.len() {
            let mut end = i + 1;
            let mut dir = 0i32;
            let mut prev = pitches[i];
            while end < pitches.len() {
                let step = pitches[end] as i32 - prev as i32;
                if !(1..=2).contains(&step.abs()) { break; }
                if dir == 0 { dir = step.signum(); } else if step.signum() != dir { break; }
                prev = pitches[end];
                end += 1;
            }
            let len = end - i;
            if len >= 4 && key_of(&pitches[i..end]).is_some() {
                *hist.entry(len).or_default() += 1;
            }
            i = if len > 1 { end } else { i + 1 };
        }
    }
    println!("{}", path);
    for (len, count) in &hist {
        println!("  {len:>3} notes: {:<4} {}", count, "#".repeat(*count));
    }
    let six_plus: usize = hist.iter().filter(|(l, _)| **l >= 6).map(|(_, c)| c).sum();
    let seven_plus: usize = hist.iter().filter(|(l, _)| **l >= 7).map(|(_, c)| c).sum();
    println!("  runs of 6+: {six_plus}    runs of 7+: {seven_plus}");
    Ok(())
}
