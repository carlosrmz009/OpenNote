//! Count the hand shapes a pianist could not make.
//!
//! A diagnostic, not a test. Point it at a MIDI file and it reports how often one hand
//! is asked to wind its fingers over each other, or to put one finger on two notes
//! sounding at once. Both are impossible rather than merely awkward, so the right
//! number is zero.
//!
//! It counts twice, because there are two different questions. The *fingering* is what
//! is written on the score and what a player would read. The *grip* is what the animator
//! actually draws, which is not quite the same thing: it drops a note when a later one
//! wants the finger, and lets go of anything the hand cannot span. A fault in the first
//! is a fault in the answer; a fault in the second is a fault you can see.
//!
//!     cargo run -p on-viz --example shapes -- path/to/piece.mid

use std::collections::BTreeMap;

use on_fingering::FingeringOptions;
use on_hand::Hand;
use on_score::hands::HandAssignment;
use on_score::MidiDocument;
use on_viz::timeline::Timeline;

fn main() -> anyhow::Result<()> {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: crossings <midi>");
        std::process::exit(2);
    };

    let mut file = MidiDocument::read(&path)?;
    let options = FingeringOptions::default();
    on_score::assign_hands(
        file.score_mut(),
        &HandAssignment {
            profile: options.profile.clone(),
            ..Default::default()
        },
    );
    let score = file.score();
    let solution = on_fingering::finger_score_consensus(score, &options, None);

    let mut fingers = BTreeMap::new();
    for fingering in &solution.fingerings {
        fingers.insert(fingering.note, fingering.finger);
    }

    let reach = options.span_model.table().0[on_hand::Finger::Thumb.index()]
        [on_hand::Finger::Little.index()]
    .max_prac;
    let (mut crossed, mut doubled, mut moments) = (0usize, 0usize, 0usize);
    for note in &score.notes {
        let at = note.onset_seconds + 1e-6;
        for hand in Hand::ALL {
            let mut down: Vec<(u8, u8)> = score
                .notes
                .iter()
                .filter(|other| other.hand == Some(hand))
                .filter(|other| other.onset_seconds <= at && other.offset_seconds() > at)
                .filter_map(|other| fingers.get(&other.id).map(|f| (other.midi, f.number())))
                .collect();
            down.sort();
            down.dedup();
            if down.len() < 2 {
                continue;
            }
            // Only what the hand could still be holding. Anything wider has been given
            // to the pedal, and the fingers that struck those notes are long gone —
            // counting them would be measuring a shape nobody is making.
            let (low, high) = (down[0].0, down[down.len() - 1].0);
            if i32::from(high - low) > reach || down.len() > 5 {
                continue;
            }
            moments += 1;

            let numbers: Vec<u8> = down.iter().map(|(_, f)| *f).collect();
            let ordered = match hand {
                Hand::Right => numbers.windows(2).all(|w| w[0] < w[1]),
                Hand::Left => numbers.windows(2).all(|w| w[0] > w[1]),
            };
            if !ordered {
                crossed += 1;
            }
            let mut unique = numbers.clone();
            unique.sort_unstable();
            unique.dedup();
            if unique.len() != numbers.len() {
                doubled += 1;
            }
        }
    }

    println!("{path}: {} notes", score.notes.len());
    println!("  the fingering, as written on the score");
    println!("    moments with more than one note down: {moments}");
    println!("    ...where the fingers cross:           {crossed}");
    println!("    ...where one finger holds two notes:  {doubled}");

    // And the shapes the animator will actually draw.
    let timeline = Timeline::build_for(score, &solution.fingerings, &options.profile);
    let (mut shapes, mut tangled, mut reused) = (0usize, 0usize, 0usize);
    for hand in Hand::ALL {
        for event in timeline.hand_grips(hand) {
            let mut keys = event.grip.keys.clone();
            keys.sort_by_key(|(midi, _)| *midi);
            if keys.len() < 2 {
                continue;
            }
            shapes += 1;
            let fingers: Vec<u8> = keys.iter().map(|(_, f)| f.number()).collect();
            let ordered = match hand {
                Hand::Right => fingers.windows(2).all(|w| w[0] < w[1]),
                Hand::Left => fingers.windows(2).all(|w| w[0] > w[1]),
            };
            if !ordered {
                tangled += 1;
            }
            let mut unique = fingers.clone();
            unique.sort_unstable();
            unique.dedup();
            if unique.len() != fingers.len() {
                reused += 1;
            }
        }
    }
    // How often a line is handed across, where one hand could have kept it. Two hands
    // alternating through a figure that sits under one of them is not a fingering fault
    // — the fingering is fine, note by note — but it is not what a pianist does, and on
    // screen it is the most visible thing there is.
    let mut consecutive = 0usize;
    let mut handed_over = 0usize;
    let mut previous: Option<(f64, u8, Hand)> = None;
    for note in &score.notes {
        let Some(hand) = note.hand else { continue };
        if let Some((was_at, was_midi, was_hand)) = previous {
            // Only where the two notes really are one after another, and close enough
            // that one hand could have taken both.
            if note.onset_seconds > was_at
                && note.onset_seconds - was_at < 0.5
                && i32::from(note.midi.abs_diff(was_midi)) < 15
            {
                consecutive += 1;
                if hand != was_hand {
                    handed_over += 1;
                }
            }
        }
        previous = Some((note.onset_seconds, note.midi, hand));
    }
    println!("  the split between the hands");
    println!("    notes one hand could have followed:   {consecutive}");
    println!("    ...given to the other hand instead:   {handed_over}");

    println!("  the grips, as the hands will be drawn");
    println!("    shapes with more than one key held:   {shapes}");
    println!("    ...where the fingers cross:           {tangled}");
    println!("    ...where one finger holds two keys:   {reused}");
    Ok(())
}
