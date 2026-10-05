use on_fingering::biomech::{BiomechModel, Grip};
use on_fingering::FingeringOptions;
use on_hand::skeleton::{HandPose, DOF, LIMITS};
use on_hand::keyboard::KEY_DIP;
use on_hand::Hand;
use on_score::hands::HandAssignment;
use on_score::MidiDocument;
use on_viz::timeline::Timeline;

const THROUGH_THE_KEYS_MM: f32 = 12.0;

const LATE_ARRIVAL: f64 = 0.12;

const COLLISION_STEP: f64 = 1.0 / 90.0;

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
    let mut free_handed: Vec<String> = Vec::new();
    let mut other_hand_busy = 0usize;
    let mut buried: Vec<String> = Vec::new();
    let mut through_but_reached = 0usize;
    let mut struck_unplayed = 0usize;
    let mut silent_strikes: Vec<String> = Vec::new();
    let mut colliding = 0usize;
    let mut sunk = 0usize;
    let mut deepest_moving = 0.0f32;
    let mut unhandled = 0usize;
    let mut sampled = 0usize;
    let mut worst_overlap = 0.0f32;
    let mut collisions: Vec<String> = Vec::new();
    let mut both_busy = 0usize;
    let mut one_free = 0usize;
    let mut worst_depth_overlap = 0.0f32;
    let mut worst_off_key = 0.0f32;
    let mut worst_off_key_alone = 0.0f32;
    let keys_geom = on_hand::keyboard::Keyboard::new();

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

            let violation = worst_joint(model.skeleton(), &pose);
            if violation > 1e-3 {
                out_of_range += 1;
                worst_violation = worst_violation.max(violation);
            }

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
                    if !busy && free_handed.len() < 10 {
                        free_handed.push(format!(
                            "  {:.2}s {hand:?} let go of {} to reach {:?} — other hand idle",
                            event.time,
                            note.midi,
                            event.grip.keys.iter().map(|(m, _)| *m).collect::<Vec<_>>()
                        ));
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

    let animators = timeline.animators(&options.profile, options.biomech);
    let mut at = 0.0;
    while at <= timeline.duration {
        sampled += 1;
        let posed = on_viz::timeline::pose_both(&animators, at);

        for hand in Hand::ALL {
            let posture = animators[hand as usize]
                .skeleton()
                .forward(&posed[hand as usize]);
            let below = posture
                .chain
                .iter()
                .flatten()
                .map(|joint| -joint.z)
                .fold(-posture.wrist.z, f32::max);
            if below > KEY_DIP {
                sunk += 1;
                deepest_moving = deepest_moving.max(below - KEY_DIP);
            }
        }
        let left = animators[Hand::Left as usize].joints(&posed[0]);
        let right = animators[Hand::Right as usize].joints(&posed[1]);
        let overlap = on_viz::timeline::overlap_depth(&left, &right);
        let bare = [
            animators[Hand::Left as usize].pose_at(at),
            animators[Hand::Right as usize].pose_at(at),
        ];
        if on_viz::timeline::overlap_depth(
            &animators[Hand::Left as usize].joints(&bare[0]),
            &animators[Hand::Right as usize].joints(&bare[1]),
        ) > 0.0
        {
            unhandled += 1;
        }
        let span = |j: &[glam::Vec3]| {
            j.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)))
        };
        let ((left_low, left_high), (right_low, right_high)) = (span(&left), span(&right));
        if overlap > 0.0 {
            colliding += 1;
            let free = [
                animators[Hand::Left as usize].grip_at(at).is_none(),
                animators[Hand::Right as usize].grip_at(at).is_none(),
            ];
            if free[0] || free[1] {
                one_free += 1;
            } else {
                both_busy += 1;
            }
            worst_depth_overlap =
                worst_depth_overlap.max(left_high.min(right_high) - left_low.max(right_low));
            worst_overlap = worst_overlap.max(overlap);
            if collisions.len() < 6 && collisions.last().is_none_or(|last: &String| {
                !last.starts_with(&format!("  {:.1}", at.floor()))
            }) {
                collisions.push(format!(
                    "  {at:.2}s the hands overlap by {overlap:.0} mm  \
                     (left {left_low:.0}..{left_high:.0}, right {right_low:.0}..{right_high:.0})"
                ));
            }
        }
        at += COLLISION_STEP;
    }

    for hand in Hand::ALL {
        let animator = &animators[hand as usize];
        for event in timeline.hand_grips(hand) {
            if event.grip.keys.is_empty() {
                continue;
            }
            let posed = on_viz::timeline::pose_both(&animators, event.time);
            let posture = animator.skeleton().forward(&posed[hand as usize]);
            let alone = animator.skeleton().forward(&animator.pose_at(event.time));
            for (midi, finger) in &event.grip.keys {
                let want = keys_geom.centre_x(*midi);
                worst_off_key =
                    worst_off_key.max((posture.chain[finger.index()][3].x - want).abs());
                worst_off_key_alone =
                    worst_off_key_alone.max((alone.chain[finger.index()][3].x - want).abs());
            }
        }
    }

    println!("{path}");
    println!("  grips the hands are drawn holding:      {grips}");
    println!("  ...where a finger never reaches its key: {missed}  (worst {worst_miss:.1} mm)");
    println!("  ...where a joint is outside its range:   {out_of_range}  (worst {worst_violation:.1}°)");
    println!("  ...where the hand is through the keys:   {through}  (deepest {deepest:.1} mm)");
    println!(
        "  ...and while they are moving:            {sunk} of {}  (deepest {deepest_moving:.1} mm past the key bottom)",
        sampled * 2
    );
    println!(
        "  moments the two hands share a space:     {colliding} of {sampled}  \
         (worst {worst_overlap:.0} mm)"
    );
    println!("      ...before any of them are moved apart: {unhandled}");
    println!("      of those, with a hand free to lift: {one_free}");
    println!("      of those, with BOTH hands holding:   {both_busy}");
    println!("      worst overlap along the keys:        {worst_depth_overlap:.0} mm");
    println!(
        "  furthest a held finger sits from its key: {worst_off_key:.1} mm posed for          collisions, {worst_off_key_alone:.1} mm without  (a white key is 23.5 mm wide)"
    );
    for line in &collisions {
        println!("{line}");
    }
    println!("  notes struck with no finger on them:     {struck_unplayed}");
    for line in &silent_strikes {
        println!("{line}");
    }
    println!("  notes still sounding under a hand:       {still_sounding}");
    println!("  ...that the hand had to let go of:       {let_go}");
    println!("      of those, with the other hand busy:  {other_hand_busy}");
    println!("      of those, with the other hand FREE: {}", let_go - other_hand_busy);
    for line in &free_handed {
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

fn shape_of(grip: &Grip) -> String {
    let mut keys = grip.keys.clone();
    keys.sort_by_key(|(midi, _)| *midi);
    keys.iter()
        .map(|(m, f)| format!("{m}={}", f.number()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shape(grip: &Grip) -> Vec<u8> {
    let mut keys = grip.keys.clone();
    keys.sort_by_key(|(midi, _)| *midi);
    keys.iter().map(|(_, f)| f.number()).collect()
}

fn worst_joint(skeleton: &on_hand::Skeleton, pose: &HandPose) -> f32 {
    let neutral = skeleton.wrist_neutral(pose);
    (0..DOF)
        .map(|i| {
            let q = if i == on_hand::skeleton::dof::WRIST_DEVIATION {
                pose.q[i] - neutral
            } else {
                pose.q[i]
            };
            LIMITS[i].violation(q).to_degrees()
        })
        .fold(0.0, f32::max)
}

fn deepest_point(model: &BiomechModel, pose: &HandPose) -> f32 {
    let posture = model.skeleton().forward(pose);
    posture
        .chain
        .iter()
        .flatten()
        .map(|point| -point.z)
        .fold(-posture.wrist.z, f32::max)
}

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
