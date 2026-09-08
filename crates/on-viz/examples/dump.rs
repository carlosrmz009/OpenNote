//! Print what each hand was given, note by note, for the first few seconds.
use on_fingering::FingeringOptions;
use on_score::hands::HandAssignment;
use on_score::MidiDocument;
use on_viz::timeline::Timeline;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("usage: dump <midi> [seconds]");
    let until: f64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8.0);
    let mut file = MidiDocument::read(&path)?;
    let options = FingeringOptions::default();
    on_score::assign_hands(
        file.score_mut(),
        &HandAssignment { profile: options.profile.clone(), ..Default::default() },
    );
    let score = file.score();
    let solution = on_fingering::finger_score_consensus(score, &options, None);
    let timeline = Timeline::build_for(score, &solution.fingerings, &options.profile);
    let mut notes: Vec<_> = timeline.notes.iter().filter(|n| n.start <= until).collect();
    notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.midi.cmp(&b.midi)));
    println!("{:>7} {:>7}  {:>4} {:>5} {:>2}", "start", "end", "midi", "hand", "f");
    for n in notes {
        println!(
            "{:7.2} {:7.2}  {:4} {:>5} {:>2}",
            n.start,
            n.end,
            n.midi,
            format!("{:?}", n.hand),
            n.finger.map(|f| f.number().to_string()).unwrap_or_else(|| "-".into())
        );
    }

    // And the grips the animator will actually draw, which is not the same list: a
    // chord too wide to hold is rolled into two.
    for hand in on_hand::Hand::ALL {
        println!("
{hand:?} grips:");
        for event in timeline.hand_grips(hand) {
            if event.time < until - 4.0 || event.time > until {
                continue;
            }
            let mut keys = event.grip.keys.clone();
            keys.sort_by_key(|(midi, _)| *midi);
            println!(
                "  {:7.2}  {}",
                event.time,
                keys.iter().map(|(m, f)| format!("{m}={}", f.number())).collect::<Vec<_>>().join(" ")
            );
        }
    }
    Ok(())
}
