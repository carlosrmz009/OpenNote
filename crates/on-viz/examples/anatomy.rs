//! Count the hand shapes a hand could not physically make.
//!
//! A diagnostic, not a test. `shapes` asks whether a fingering is *coherent* — no
//! finger in two places, no fingers wound over each other. This asks the harder
//! question: given a skeleton with real joint ranges, can a hand actually get into the
//! posture the fingering asks for, and where does it end up if it cannot.
//!
//! Three things are reported for every grip the animator will draw:
//!
//! * whether every finger reached its key, and by how far the worst one missed
//! * whether the solved posture sits inside the joint ranges, and by how many degrees
//!   it does not
//! * whether any part of the hand ends up below the keys it is supposed to be playing
//!
//! The right answer to all three is zero. A miss means the drawn hand is somewhere the
//! fingering did not ask for; a joint out of range means the drawn hand is doing
//! something no hand does; and a hand under the keyboard is the one anybody notices.
//!
//!     cargo run -p on-viz --example anatomy -- path/to/piece.mid

use on_fingering::biomech::{BiomechModel, Grip};
use on_fingering::FingeringOptions;
use on_hand::skeleton::{HandPose, DOF, LIMITS};
use on_hand::Hand;
use on_score::hands::HandAssignment;
use on_score::MidiDocument;
use on_viz::timeline::Timeline;

/// How far below the top of the keys a joint may sit before it is inside them.
///
/// A white key's surface is `z = 0` and a key travels about ten millimetres when it is
/// pressed, so a fingertip a little below zero is playing. Anything further down is not
/// on the key, it is in it.
const THROUGH_THE_KEYS_MM: f32 = 12.0;

/// How long after a key goes down a finger may still arrive and count as having played
/// it, in seconds. A rolled chord is the reason there is any window at all; it wants a
/// little more than the roll itself.
const LATE_ARRIVAL: f64 = 0.12;

fn main() -> anyhow::Result<()> {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: anatomy <midi>");
        std::process::exit(2);
    };

    let mut file = MidiDocument::read(&path)?;
    let options = FingeringOptions::default();
    on_score::assign_hands(
        file.score_mut(),
        &HandAssignment { profile: options.profile.clone(), ..Default::default() },
    );
    let score = file.score();
    let solution = on_fingering::finger_score_consensus(score, &options, None);
    let timeline = Timeline::build_for(score, &solution.fingerings, &options.profile);

    let mut grips = 0usize;
    let mut missed = 0usize;
    let mut worst_miss = 0.0f32;
    let mut out_of_range = 0usize;
    let mut worst_violation = 0.0f32;
    let mut through = 0usize;
    let mut deepest = 0.0f32;
    let mut examples: Vec<String> = Vec::new();
    let mut let_go = 0usize;
    let mut still_sounding = 0usize;
    let mut abandoned: Vec<String> = Vec::new();
    let mut other_hand_busy = 0usize;
    let mut buried: Vec<String> = Vec::new();
    let mut through_but_reached = 0usize;
    let mut struck_unplayed = 0usize;
    let mut silent_strikes: Vec<String> = Vec::new();

    for hand in Hand::ALL {
        let model = BiomechModel::new(options.profile.clone(), hand, options.biomech);
        for event in timeline.hand_grips(hand) {
            if event.grip.keys.is_empty() {
                continue;
            }
            grips += 1;
            let outcome = model.grip_outcome(&event.grip);
            let pose = model.grip_pose(&event.grip);

            if !outcome.reachable {
                missed += 1;
                worst_miss = worst_miss.max(outcome.shortfall_mm);
                if examples.len() < 6 {
                    let mut keys = event.grip.keys.clone();
                    keys.sort_by_key(|(midi, _)| *midi);
                    examples.push(format!(
                        "  {:.2}s {hand:?} misses by {:.1} mm: {}",
                        event.time,
                        outcome.shortfall_mm,
                        keys.iter()
                            .map(|(m, f)| format!("{m}={}", f.number()))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ));
                    let ways = reachable_fingerings(&model, hand, &keys);
                    let last = examples.len() - 1;
                    examples[last]
                        .push_str(&format!("   ({ways} reachable fingerings of these notes)"));
                }
            }

            let violation = worst_joint(&pose);
            if violation > 1e-3 {
                out_of_range += 1;
                worst_violation = worst_violation.max(violation);
            }

            // Notes this hand still has sounding that it is not holding: the hand was
            // given something it could not reach without letting them go, and a finger
            // cannot come off a key until the note stops.
            for note in &timeline.notes {
                if note.hand != hand || note.finger.is_none() {
                    continue;
                }
                if note.start > event.time - 1e-6 || note.end < event.time + 1e-6 {
                    continue;
                }
                still_sounding += 1;
                if !event.grip.keys.iter().any(|(midi, _)| *midi == note.midi) {
                    let_go += 1;
                    // Could the other hand have taken what this one was sent for?
                    let other = match hand {
                        Hand::Left => Hand::Right,
                        Hand::Right => Hand::Left,
                    };
                    let busy = timeline.notes.iter().any(|n| {
                        n.hand == other && n.start <= event.time + 1e-6 && n.end > event.time
                    });
                    if busy {
                        other_hand_busy += 1;
                    }
                    if abandoned.len() < 5 {
                        abandoned.push(format!(
                            "  {:.2}s {hand:?} let go of {} to reach {:?}",
                            event.time,
                            note.midi,
                            event.grip.keys.iter().map(|(m, _)| *m).collect::<Vec<_>>()
                        ));
                    }
                }
            }

            let below = deepest_point(&model, &pose);
            if below > THROUGH_THE_KEYS_MM {
                through += 1;
                deepest = deepest.max(below);
                if outcome.reachable {
                    through_but_reached += 1;
                }
                if buried.len() < 5 {
                    buried.push(format!(
                        "  {:.2}s {hand:?} {:.0} mm into the keys, reached={}: {:?}",
                        event.time, below, outcome.reachable, shape(&event.grip)
                    ));
                }
            }
        }

        // Notes struck with nothing on them.
        //
        // Asked of the note rather than of one grip, because a chord too wide to hold
        // is rolled: the hand reaches the upper notes a moment after their keys go
        // down, and the finger that arrives late is still the finger that played them.
        // Asking each grip whether it covers everything struck at its own instant
        // counts every rolled chord as a fault, and — worse — stops asking about the
        // notes it hands to the later grip at all.
        for note in &timeline.notes {
            if note.hand != hand || note.finger.is_none() {
                continue;
            }
            let played = timeline.hand_grips(hand).iter().any(|event| {
                event.time >= note.start - 1e-6
                    && event.time <= note.start + LATE_ARRIVAL
                    && event.grip.keys.iter().any(|(midi, _)| *midi == note.midi)
            });
            if !played {
                struck_unplayed += 1;
                if silent_strikes.len() < 8 {
                    let at = timeline
                        .hand_grips(hand)
                        .iter()
                        .rfind(|e| e.time <= note.start + 1e-6)
                        .map(|e| shape_of(&e.grip))
                        .unwrap_or_else(|| "nothing".into());
                    silent_strikes.push(format!(
                        "  {:.2}s {hand:?} strikes {}={} with no finger; hand is holding {at}",
                        note.start,
                        note.midi,
                        note.finger.expect("checked above").number(),
                    ));
                }
            }
        }
    }

    println!("{path}");
    println!("  grips the hands are drawn holding:      {grips}");
    println!("  ...where a finger never reaches its key: {missed}  (worst {worst_miss:.1} mm)");
    println!("  ...where a joint is outside its range:   {out_of_range}  (worst {worst_violation:.1}°)");
    println!("  ...where the hand is through the keys:   {through}  (deepest {deepest:.1} mm)");
    println!("  notes struck with no finger on them:     {struck_unplayed}");
    for line in &silent_strikes {
        println!("{line}");
    }
    println!("  notes still sounding under a hand:       {still_sounding}");
    println!("  ...that the hand had to let go of:       {let_go}");
    println!("      of those, with the other hand busy:  {other_hand_busy}");
    for line in &abandoned {
        println!("{line}");
    }
    println!("      of those, ones that did reach the key: {through_but_reached}");
    for line in &buried {
        println!("{line}");
    }
    if !examples.is_empty() {
        println!("  the first few misses:");
        for line in examples {
            println!("{line}");
        }
    }
    Ok(())
}

/// A grip written out as `note=finger` pairs, low note first.
fn shape_of(grip: &Grip) -> String {
    let mut keys = grip.keys.clone();
    keys.sort_by_key(|(midi, _)| *midi);
    keys.iter()
        .map(|(m, f)| format!("{m}={}", f.number()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The finger numbers of a grip, low note first.
fn shape(grip: &Grip) -> Vec<u8> {
    let mut keys = grip.keys.clone();
    keys.sort_by_key(|(midi, _)| *midi);
    keys.iter().map(|(_, f)| f.number()).collect()
}

/// How far outside its range the worst joint of a pose is, in degrees.
fn worst_joint(pose: &HandPose) -> f32 {
    (0..DOF)
        .map(|i| LIMITS[i].violation(pose.q[i]).to_degrees())
        .fold(0.0, f32::max)
}

/// How far the deepest part of the hand sits below the surface of the keys.
///
/// Height is `z`, with a white key's top at zero. `y` is the distal axis, along the
/// keys towards the player, and measuring that instead says every hand is buried,
/// which is how this metric read the first time it was written.
fn deepest_point(model: &BiomechModel, pose: &HandPose) -> f32 {
    let posture = model.skeleton().forward(pose);
    posture
        .chain
        .iter()
        .flatten()
        .map(|point| -point.z)
        .fold(-posture.wrist.z, f32::max)
}

/// How many orderly fingerings of these notes the hand could actually hold.
///
/// Orderly meaning the finger numbers run the same way as the pitches, which is what
/// the search itself enumerates. If this is zero the notes cannot be played by one hand
/// at all and the fault is further back, in which hand was given them; if it is more
/// than zero, a reachable fingering existed and was not taken.
fn reachable_fingerings(model: &BiomechModel, hand: Hand, keys: &[(u8, on_hand::Finger)]) -> usize {
    use on_hand::Finger;
    let n = keys.len();
    let mut found = 0;
    let mut chosen: Vec<Finger> = Vec::with_capacity(n);
    fn walk(
        model: &BiomechModel,
        hand: Hand,
        keys: &[(u8, on_hand::Finger)],
        at: usize,
        chosen: &mut Vec<Finger>,
        found: &mut usize,
    ) {
        use on_hand::Finger;
        if at == keys.len() {
            let grip = Grip::new(
                keys.iter().zip(chosen.iter()).map(|((m, _), f)| (*m, *f)).collect(),
            );
            if model.grip_outcome(&grip).reachable {
                *found += 1;
            }
            return;
        }
        for f in Finger::ALL {
            if let Some(previous) = chosen.last() {
                let ordered = match hand {
                    Hand::Right => f.index() > previous.index(),
                    Hand::Left => f.index() < previous.index(),
                };
                if !ordered {
                    continue;
                }
            }
            chosen.push(f);
            walk(model, hand, keys, at + 1, chosen, found);
            chosen.pop();
        }
    }
    walk(model, hand, keys, 0, &mut chosen, &mut found);
    let _ = n;
    found
}
