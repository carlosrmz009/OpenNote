use on_fingering::rules::Placement;
use on_fingering::scales::{scale_fingerings, Taught};
use on_fingering::{NgramPrior, Symmetries};
use on_hand::{Finger, Hand};

const MAJOR: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];

fn scale_pitches(tonic: u8, base: u8) -> Vec<u8> {
    let mut up = Vec::new();
    for octave in 0..2u8 {
        for step in MAJOR {
            up.push(base + tonic + step + 12 * octave);
        }
    }
    up.push(base + tonic + 24);
    let mut out = up.clone();
    out.extend(up.iter().rev().skip(1).copied());
    out
}

fn taught(hand: Hand, tonic: u8) -> Vec<Placement> {
    let base = match hand {
        Hand::Right => 60,
        Hand::Left => 36,
    };
    let pitches = scale_pitches(tonic, base);
    let optional: Vec<Option<u8>> = pitches.iter().map(|p| Some(*p)).collect();
    let onsets: Vec<f64> = (0..pitches.len()).map(|i| i as f64 * 0.2).collect();
    let table: std::collections::HashMap<usize, Taught> =
        scale_fingerings(hand, &optional, &onsets);
    let mut out = Vec::new();
    for (index, pitch) in pitches.iter().enumerate() {
        if let Some(t) = table.get(&index) {
            out.push(Placement::new(*pitch, t.finger));
        }
    }
    out
}

fn halves(run: &[Placement]) -> (Vec<Placement>, Vec<Placement>) {
    let peak = run
        .iter()
        .enumerate()
        .max_by_key(|(_, p)| p.midi)
        .map(|(i, _)| i)
        .unwrap_or(0);
    (run[..=peak].to_vec(), run[peak..].to_vec())
}

fn agreement(prior: &NgramPrior, hand: Hand, runs: &[Vec<Placement>]) -> f32 {
    use on_fingering::FingeringPrior;
    let mut right = 0usize;
    let mut total = 0usize;
    for run in runs {
        for index in 2..run.len() {
            let want = run[index];
            let best = Finger::ALL
                .iter()
                .copied()
                .max_by(|a, b| {
                    let score = |f: Finger| {
                        prior.log_probability(
                            hand,
                            Some(run[index - 2]),
                            Some(run[index - 1]),
                            Placement::new(want.midi, f),
                        )
                    };
                    score(*a).total_cmp(&score(*b))
                })
                .expect("five fingers");
            total += 1;
            if best == want.finger {
                right += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        right as f32 / total as f32
    }
}

fn trial(
    symmetries: Symmetries,
    train: &[(Hand, Vec<Placement>)],
    test_hand: Hand,
    test: &[Vec<Placement>],
) -> f32 {
    let mut prior = NgramPrior::with_symmetries(symmetries);
    for (hand, run) in train {
        prior.observe(*hand, run);
    }
    agreement(&prior, test_hand, test)
}

fn line(label: &str, none: f32, time: f32, full: f32) {
    let change = |x: f32| {
        let d = (x - none) * 100.0;
        if d.abs() < 0.05 {
            "     -".to_string()
        } else {
            format!("{d:+6.1}")
        }
    };
    println!(
        "  {label:<24} {:>6.1}% {:>6.1}% {} {:>6.1}% {}",
        none * 100.0,
        time * 100.0,
        change(time),
        full * 100.0,
        change(full),
    );
}

fn compare(
    train: &[(Hand, Vec<Placement>)],
    test_hand: Hand,
    test: &[Vec<Placement>],
) -> (f32, f32, f32) {
    (
        trial(Symmetries::None, train, test_hand, test),
        trial(Symmetries::Time, train, test_hand, test),
        trial(Symmetries::Full, train, test_hand, test),
    )
}

fn main() {
    let right: Vec<Vec<Placement>> = (0..12).map(|t| taught(Hand::Right, t)).collect();
    let left: Vec<Vec<Placement>> = (0..12).map(|t| taught(Hand::Left, t)).collect();

    println!("Agreement of the prior alone with the taught scale fingerings.\n");
    println!(
        "  {:<24} {:>6} {:>6} {:>6} {:>6} {:>6}",
        "", "none", "time", "", "full", ""
    );

    println!("\n  Trained on some keys, measured on the rest");
    for count in [1usize, 2, 3, 4, 6] {
        let mut totals = (0.0, 0.0, 0.0);
        for start in 0..12 {
            let chosen: Vec<usize> = (0..count).map(|i| (start + i * 5) % 12).collect();
            let mut train = Vec::new();
            for key in &chosen {
                train.push((Hand::Right, right[*key].clone()));
                train.push((Hand::Left, left[*key].clone()));
            }
            let held: Vec<Vec<Placement>> = (0..12)
                .filter(|k| !chosen.contains(k))
                .map(|k| right[k].clone())
                .collect();
            let got = compare(&train, Hand::Right, &held);
            totals = (totals.0 + got.0, totals.1 + got.1, totals.2 + got.2);
        }
        let label = format!("{count} key{}", if count == 1 { "" } else { "s" });
        line(&label, totals.0 / 12.0, totals.1 / 12.0, totals.2 / 12.0);
    }

    println!("\n  Trained on the right hand, measured on the left");
    for count in [1usize, 3, 12] {
        let train: Vec<(Hand, Vec<Placement>)> = (0..count)
            .map(|k| (Hand::Right, right[k % 12].clone()))
            .collect();
        let got = compare(&train, Hand::Left, &left);
        let label = format!("{count} right-hand scale{}", if count == 1 { "" } else { "s" });
        line(&label, got.0, got.1, got.2);
    }

    println!("\n  Trained on ascents, measured on descents");
    for count in [1usize, 3, 12] {
        let train: Vec<(Hand, Vec<Placement>)> = (0..count)
            .map(|k| (Hand::Right, halves(&right[k % 12]).0))
            .collect();
        let down: Vec<Vec<Placement>> = (0..12).map(|k| halves(&right[k]).1).collect();
        let got = compare(&train, Hand::Right, &down);
        let label = format!("{count} ascent{}", if count == 1 { "" } else { "s" });
        line(&label, got.0, got.1, got.2);
    }

    println!(
        "\n  Scales are the easy case, and the prior is shown here without the rules or\n  \
         the hand model that normally sit in front of it. What the three panels are for\n  \
         is the shape of the curve, not the height of it."
    );
}
