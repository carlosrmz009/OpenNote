use super::*;
use on_fingering::biomech::BiomechWeights;
use on_hand::HandProfile;
use on_score::{Note, SourceRef, TICKS_PER_QUARTER};

fn score_of(entries: &[(u8, i64, i64, Hand)]) -> Score {
    let mut score = Score::default();
    for (i, (midi, onset, duration, hand)) in entries.iter().enumerate() {
        score.notes.push(Note {
            id: NoteId(i as u32),
            midi: *midi,
            onset: *onset,
            duration: *duration,
            hand: Some(*hand),
            velocity: 90,
            source: SourceRef::Midi { track: 0, event: i },
            ..Default::default()
        });
    }
    score.finalise();
    score
}

fn pinned(entries: &[(u32, Finger)]) -> Vec<Fingering> {
    entries
        .iter()
        .map(|(id, finger)| Fingering { note: NoteId(*id), finger: *finger, substitute: None })
        .collect()
}

fn fingered(score: &Score) -> Vec<Fingering> {
    on_fingering::finger_score(score, &Default::default()).fingerings
}

#[test]
fn a_repeated_key_knows_when_it_is_struck_again() {
    let quarter = i64::from(TICKS_PER_QUARTER);
    let score = score_of(&[
        (60, 0, quarter, Hand::Right),
        (62, quarter, quarter, Hand::Right),
        (60, 2 * quarter, quarter, Hand::Right),
        (60, 3 * quarter, quarter, Hand::Right),
    ]);
    let timeline = Timeline::build(&score, &[]);

    let at = |midi: u8, nth: usize| {
        timeline.notes.iter().filter(|n| n.midi == midi).nth(nth).expect("note is there")
    };

    let first = at(60, 0);
    let second = at(60, 1);
    let third = at(60, 2);
    assert_eq!(first.restruck, Some(second.start), "{first:?}");
    assert_eq!(second.restruck, Some(third.start), "{second:?}");
    assert_eq!(third.restruck, None, "the last one is never struck again");

    assert_eq!(at(62, 0).restruck, None);
}

#[test]
fn one_key_never_carries_two_plumes_at_once() {
    let sixteenth = i64::from(TICKS_PER_QUARTER) / 4;
    let score = score_of(&[
        (60, 0, sixteenth, Hand::Right),
        (60, sixteenth, sixteenth, Hand::Right),
        (60, 2 * sixteenth, sixteenth, Hand::Right),
        (60, 3 * sixteenth, sixteenth, Hand::Right),
    ]);
    let timeline = Timeline::build(&score, &[]);

    let tail = 0.10_f64;
    let window = |n: &TimelineNote| (n.start, (n.end + tail).min(n.restruck.unwrap_or(f64::INFINITY)));
    for pair in timeline.notes.windows(2) {
        let (_, ends) = window(&pair[0]);
        let (starts, _) = window(&pair[1]);
        assert!(ends <= starts + 1e-9, "a plume ending at {ends} overlaps the next starting at {starts}");
    }
}

#[test]
fn a_timeline_keeps_every_note_with_its_finger() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[(60, 0, q, Hand::Right), (64, q, q, Hand::Right), (48, 0, 2 * q, Hand::Left)]);
    let timeline = Timeline::build(&score, &fingered(&score));
    assert_eq!(timeline.notes.len(), 3);
    assert!(timeline.notes.iter().all(|n| n.finger.is_some()));
    assert!((timeline.duration - 1.0).abs() < 1e-6, "{}", timeline.duration);
}

#[test]
fn a_hand_lets_go_of_what_it_cannot_span() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[
        (36, 0, 8 * q, Hand::Left),
        (60, q, q, Hand::Left),
        (67, 2 * q, q, Hand::Left),
        (72, 3 * q, q, Hand::Left),
        (79, 4 * q, q, Hand::Left),
    ]);
    let fingerings = pinned(&[
        (0, Finger::Little),
        (1, Finger::Ring),
        (2, Finger::Middle),
        (3, Finger::Index),
        (4, Finger::Thumb),
    ]);
    let timeline = Timeline::build(&score, &fingerings);

    for event in timeline.hand_grips(Hand::Left) {
        if event.grip.keys.len() < 2 {
            continue;
        }
        let (low, high) =
            event.grip.keys.iter().fold((u8::MAX, u8::MIN), |(lo, hi), (m, _)| (lo.min(*m), hi.max(*m)));
        assert!(
            high - low <= 24,
            "the hand is holding {low} to {high} at {:.2}s — {} semitones",
            event.time,
            high - low
        );
    }
}

#[test]
fn a_held_note_stays_in_the_hand_until_it_is_released() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[
        (36, 0, 4 * q, Hand::Left),
        (41, 0, 4 * q, Hand::Left),
        (45, q, q, Hand::Left),
        (46, 2 * q, q, Hand::Left),
        (48, 3 * q, q, Hand::Left),
    ]);
    let fingerings = pinned(&[
        (0, Finger::Little),
        (1, Finger::Middle),
        (2, Finger::Index),
        (3, Finger::Index),
        (4, Finger::Thumb),
    ]);
    let timeline = Timeline::build(&score, &fingerings);
    let grips = timeline.hand_grips(Hand::Left);
    assert!(grips.len() >= 4, "one grip per onset: {}", grips.len());

    for grip in grips {
        let holds = |midi: u8| grip.grip.keys.iter().any(|(m, _)| *m == midi);
        assert!(holds(36) && holds(41), "let go at {:.2}s: {:?}", grip.time, grip.grip.keys);
    }
}

#[test]
fn a_hold_no_hand_could_make_is_let_go_of() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[(36, 0, 4 * q, Hand::Left), (43, 0, 4 * q, Hand::Left)]);
    let fingerings = pinned(&[(0, Finger::Little), (1, Finger::Ring)]);
    let timeline = Timeline::build(&score, &fingerings);

    for grip in timeline.hand_grips(Hand::Left) {
        assert!(
            grip.grip.keys.len() < 2,
            "held a fifth between the ring and little fingers: {:?}",
            grip.grip.keys
        );
    }
}

#[test]
fn a_finger_is_never_asked_to_hold_two_keys() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[
        (60, 0, 8 * q, Hand::Right),
        (62, q, q, Hand::Right),
        (64, 2 * q, q, Hand::Right),
        (65, 3 * q, q, Hand::Right),
        (67, 4 * q, q, Hand::Right),
        (69, 5 * q, q, Hand::Right),
    ]);
    let timeline = Timeline::build(&score, &fingered(&score));
    for grip in timeline.hand_grips(Hand::Right) {
        let mut fingers: Vec<u8> = grip.grip.keys.iter().map(|(_, f)| f.number()).collect();
        let before = fingers.len();
        fingers.sort_unstable();
        fingers.dedup();
        assert_eq!(fingers.len(), before, "a finger was on two keys at {:.2}s", grip.time);
        assert!(grip.grip.keys.len() <= 5, "more keys than fingers");
    }
}

#[test]
fn the_hand_bounces_between_repeated_notes() {
    let q = TICKS_PER_QUARTER as i64;
    let short = q / 3;
    let score = score_of(&[
        (60, 0, short, Hand::Right),
        (60, q, short, Hand::Right),
        (60, 2 * q, short, Hand::Right),
        (60, 3 * q, short, Hand::Right),
    ]);
    let timeline = Timeline::build(&score, &fingered(&score));
    let animator = HandAnimator::new(
        Hand::Right,
        BiomechModel::new(HandProfile::default(), Hand::Right, BiomechWeights::default()),
        timeline.hand_grips(Hand::Right).to_vec(),
    );

    let height = |t: f64| animator.pose_at(t).q[dof::WRIST_Z];
    let struck = height(0.55);
    let between = height(0.75);
    assert!(between > struck + 1.0, "the hand should rise between strikes: {struck:.2} then {between:.2}");
    let next = height(1.05);
    assert!((next - struck).abs() < 0.5, "and be back on the key: {struck:.2} then {next:.2}");
}

#[test]
fn an_unused_hand_waits_out_of_the_way() {
    let q = TICKS_PER_QUARTER as i64;
    let mut score =
        score_of(&[(60, 0, q, Hand::Right), (64, q, q, Hand::Right), (67, 2 * q, q, Hand::Right)]);
    for note in &mut score.notes {
        note.hand = Some(Hand::Right);
    }
    let timeline = Timeline::build(&score, &fingered(&score));
    let animators = timeline.animators(&HandProfile::default(), BiomechWeights::default());

    let left = animators[Hand::Left as usize].pose_at(1.0).wrist_position().x;
    let right = animators[Hand::Right as usize].pose_at(1.0).wrist_position().x;
    assert!(
        right - left > IDLE_CLEARANCE_MM,
        "the idle left hand is standing in the right one: left at {left:.0} mm, \
         right at {right:.0} mm"
    );
}

#[test]
fn a_free_hand_is_lifted_over_the_other_and_a_busy_one_is_not() {
    let q = TICKS_PER_QUARTER as i64;
    let score =
        score_of(&[(60, 0, 8 * q, Hand::Right), (64, 0, 8 * q, Hand::Right), (62, 0, q / 4, Hand::Left)]);
    let timeline = Timeline::build(&score, &fingered(&score));
    let animators = timeline.animators(&HandProfile::default(), BiomechWeights::default());

    let at = 1.0;
    let alone = [animators[Hand::Left as usize].pose_at(at), animators[Hand::Right as usize].pose_at(at)];
    let together = pose_both(&animators, at);

    assert_eq!(
        together[Hand::Right as usize].q[dof::WRIST_Z],
        alone[Hand::Right as usize].q[dof::WRIST_Z],
        "a hand holding keys was lifted off them"
    );
    assert!(
        together[Hand::Left as usize].q[dof::WRIST_Z] >= alone[Hand::Left as usize].q[dof::WRIST_Z] - 1e-4,
        "the free hand was pushed down into the other one"
    );
}

#[test]
fn getting_clear_never_drags_a_finger_off_its_key() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[
        (52, 0, 8 * q, Hand::Left),
        (59, 0, 8 * q, Hand::Left),
        (62, 0, 8 * q, Hand::Right),
        (69, 0, 8 * q, Hand::Right),
    ]);
    let timeline = Timeline::build(&score, &fingered(&score));
    let animators = timeline.animators(&HandProfile::default(), BiomechWeights::default());
    let keys = on_hand::keyboard::Keyboard::new();

    let at = 1.0;
    let posed = pose_both(&animators, at);
    for hand in Hand::ALL {
        let animator = &animators[hand as usize];
        let Some(grip) = animator.grip_at(at) else { continue };
        let swung = animator.skeleton().forward(&posed[hand as usize]);
        let alone = animator.skeleton().forward(&animator.pose_at(at));
        for (midi, finger) in &grip.keys {
            let want = keys.centre_x(*midi);
            let moved = (swung.chain[finger.index()][3].x - want).abs()
                - (alone.chain[finger.index()][3].x - want).abs();
            assert!(
                moved <= SWING_TOLERANCE_MM,
                "{hand:?} finger {} on {midi} was dragged {moved:.1} mm further off                      its key to get the hands apart",
                finger.number()
            );
        }
    }
}

fn flowing(score: &Score, fingers: &[(u32, Finger)]) -> (Timeline, HandAnimator) {
    let timeline = Timeline::build(score, &pinned(fingers));
    let model = BiomechModel::new(HandProfile::default(), Hand::Right, BiomechWeights::default());
    let animator = HandAnimator::moving(
        Hand::Right,
        model,
        timeline.hand_grips(Hand::Right).to_vec(),
        &timeline.notes,
        &MotionStyle::default(),
    );
    (timeline, animator)
}

#[test]
fn flowing_hands_move_and_never_step() {
    let q = TICKS_PER_QUARTER as i64;
    let pitches = [60u8, 62, 64, 65, 67, 72, 71, 69, 67, 65, 64, 62, 60];
    let fingers = [1, 2, 3, 1, 2, 5, 4, 3, 2, 1, 3, 2, 1];
    let entries: Vec<_> =
        pitches.iter().enumerate().map(|(i, m)| (*m, i as i64 * q / 4, q / 4, Hand::Right)).collect();
    let fingered: Vec<_> =
        fingers.iter().enumerate().map(|(i, f)| (i as u32, Finger::from_number(*f).unwrap())).collect();
    let (timeline, animator) = flowing(&score_of(&entries), &fingered);
    let mut before = animator.joints(&animator.pose_at(0.0));
    let mut at = 0.0005;
    while at < timeline.duration + 0.5 {
        let now = animator.joints(&animator.pose_at(at));
        let moved = before.iter().zip(&now).map(|(a, b)| a.distance(*b)).fold(0.0, f32::max);
        assert!(moved < 3.0, "a joint jumped {moved:.1} mm at {at:.4}s");
        before = now;
        at += 0.0005;
    }
}

#[test]
fn every_note_sounds_with_its_finger_on_the_key_even_between_leaps() {
    let q = TICKS_PER_QUARTER as i64;
    let pitches = [48u8, 72, 50, 74, 52, 76, 53, 77];
    let fingers = [1, 5, 1, 5, 1, 5, 1, 5];
    let entries: Vec<_> =
        pitches.iter().enumerate().map(|(i, m)| (*m, i as i64 * q / 4, q / 4, Hand::Right)).collect();
    let fingered: Vec<_> =
        fingers.iter().enumerate().map(|(i, f)| (i as u32, Finger::from_number(*f).unwrap())).collect();
    let (timeline, animator) = flowing(&score_of(&entries), &fingered);
    let keyboard = on_hand::keyboard::Keyboard::new();
    for note in &timeline.notes {
        let finger = note.finger.unwrap();
        let tip = animator.joints(&animator.pose_at(note.start))[1 + 4 * finger.index() + 3];
        let off = (tip.x - keyboard.centre_x(note.midi)).abs();
        assert!(off < 8.0, "{finger:?} sounds {} from {off:.0} mm away", note.midi);
    }
}

#[test]
fn a_pressed_key_has_a_fingertip_on_it() {
    let q = TICKS_PER_QUARTER as i64;
    let entries = [(60u8, 0, q, Hand::Right), (64, q, q, Hand::Right), (67, 2 * q, q, Hand::Right)];
    let (timeline, animator) =
        flowing(&score_of(&entries), &[(0, Finger::Thumb), (1, Finger::Middle), (2, Finger::Little)]);
    for note in &timeline.notes {
        let finger = note.finger.unwrap();
        let at = note.key_bottoms() + 0.01;
        let tip = animator.joints(&animator.pose_at(at))[1 + 4 * finger.index() + 3];
        let want = -KEY_DIP * note.key_depth(at) as f32;
        assert!(
            (tip.z - want).abs() < 2.0,
            "{:?} on {} is at {:.1} mm, the key at {want:.1}",
            finger,
            note.midi,
            tip.z
        );
    }
}

#[test]
fn notes_a_moment_apart_are_one_chord_not_two_leaps() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[(46, 0, q, Hand::Left), (58, 2, q, Hand::Left), (62, 4, q, Hand::Left)]);
    let timeline =
        Timeline::build(&score, &pinned(&[(0, Finger::Little), (1, Finger::Index), (2, Finger::Thumb)]));
    let grips = timeline.hand_grips(Hand::Left);
    let animators = timeline.animators(&HandProfile::default(), BiomechWeights::default());
    let wrist = |at: f64| animators[0].pose_at(at).q[dof::WRIST_X];
    let (start, end) = (grips[0].time, timeline.notes.iter().map(|n| n.start).fold(0.0, f64::max));
    let moved = (wrist(end + 1e-6) - wrist(start)).abs();
    let mut at = start;
    while at < end {
        let speed = (wrist(at + 0.0005) - wrist(at)).abs() / 0.0005;
        assert!(speed < 5000.0, "the hand leapt {speed:.0} mm/s at {at}");
        at += 0.0005;
    }
    assert!(moved < 200.0, "{moved}");
}

#[test]
fn deciding_ahead_of_time_changes_nothing() {
    let q = TICKS_PER_QUARTER as i64;
    let mut entries = Vec::new();
    let mut fingers = Vec::new();
    for beat in 0..12 {
        let (left, right, finger) =
            if beat % 2 == 0 { (60, 62, Finger::Thumb) } else { (59, 64, Finger::Index) };
        fingers.push((entries.len() as u32, finger));
        entries.push((left, beat * q / 2, q / 4, Hand::Left));
        fingers.push((entries.len() as u32, finger));
        entries.push((right, beat * q / 2, q / 4, Hand::Right));
    }
    let score = score_of(&entries);
    let timeline = Timeline::build(&score, &pinned(&fingers));
    let animators = timeline.animators(&HandProfile::default(), BiomechWeights::default());
    let decisions = Decisions::new(&animators, timeline.duration + 1.0);
    for n in 0..200 {
        let at = n as f64 * timeline.duration / 199.0 + 0.0013;
        assert_eq!(pose_both(&animators, at), pose_both_decided(&animators, &decisions, at), "at {at}");
    }
}

#[test]
fn hands_side_by_side_do_not_twitch() {
    let q = TICKS_PER_QUARTER as i64;
    let mut entries = Vec::new();
    let mut fingers = Vec::new();
    for beat in 0..16 {
        let (left, right, left_finger, right_finger) = match beat % 4 {
            0 => (55, 57, Finger::Thumb, Finger::Thumb),
            1 => (53, 59, Finger::Index, Finger::Index),
            2 => (52, 60, Finger::Middle, Finger::Middle),
            _ => (53, 59, Finger::Index, Finger::Index),
        };
        fingers.push((entries.len() as u32, left_finger));
        entries.push((left, beat * q / 4, q / 4, Hand::Left));
        fingers.push((entries.len() as u32, right_finger));
        entries.push((right, beat * q / 4, q / 4, Hand::Right));
    }
    let timeline = Timeline::build(&score_of(&entries), &pinned(&fingers));
    let animators = timeline.animators(&HandProfile::default(), BiomechWeights::default());
    let decisions = Decisions::new(&animators, timeline.duration + 1.0);
    let step = 1.0 / 240.0;
    let samples = (timeline.duration / step) as usize;
    let met = (0..samples).step_by(12).any(|n| {
        let at = n as f64 * step;
        (pose_both_decided(&animators, &decisions, at)[0].q[dof::WRIST_X]
            - animators[0].pose_at(at).q[dof::WRIST_X])
            .abs()
            > 1.0
    });
    assert!(met, "the hands never came close enough to be moved apart, so this showed nothing");
    for side in 0..2 {
        let x: Vec<f32> = (0..samples)
            .map(|n| pose_both_decided(&animators, &decisions, n as f64 * step)[side].q[dof::WRIST_X])
            .collect();
        let turns: Vec<usize> =
            (1..x.len() - 1).filter(|&i| (x[i] - x[i - 1]) * (x[i + 1] - x[i]) < 0.0).collect();
        for pair in turns.windows(3) {
            let (p, m, r) = (pair[0], pair[1], pair[2]);
            let quick = (r - p) as f64 * step <= 0.08;
            assert!(
                !(quick && (x[m] - x[p]).abs() > 1.0 && (x[r] - x[m]).abs() > 1.0),
                "{:?} wrist twitched at {:.3}s",
                Hand::ALL[side],
                m as f64 * step
            );
        }
    }
}

#[test]
fn the_hands_never_step_where_they_meet() {
    let q = TICKS_PER_QUARTER as i64;
    let mut entries = Vec::new();
    let mut fingers = Vec::new();
    for beat in 0..12 {
        let (left, right, finger) =
            if beat % 2 == 0 { (55, 57, Finger::Thumb) } else { (53, 59, Finger::Index) };
        let at = beat * q / 2;
        fingers.push((entries.len() as u32, finger));
        entries.push((left, at, q / 6, Hand::Left));
        fingers.push((entries.len() as u32, finger));
        entries.push((right, at, q / 6, Hand::Right));
    }
    let score = score_of(&entries);
    let timeline = Timeline::build(&score, &pinned(&fingers));
    let animators = timeline.animators(&HandProfile::default(), BiomechWeights::default());
    let drawn = |at: f64| {
        let posed = pose_both(&animators, at);
        [animators[0].joints(&posed[0]), animators[1].joints(&posed[1])]
    };

    let step = 0.0005;
    let mut met = false;
    let mut before = drawn(0.0);
    let mut at = step;
    while at < timeline.duration {
        let now = drawn(at);
        for side in 0..2 {
            let moved = before[side].iter().zip(&now[side]).map(|(a, b)| a.distance(*b)).fold(0.0, f32::max);
            assert!(moved < 3.0, "{:?} jumped {moved:.1} mm at {at:.4}s", Hand::ALL[side]);
        }
        met |= pose_both(&animators, at)[0].q != animators[0].pose_at(at).q;
        before = now;
        at += step;
    }
    assert!(met, "the hands never came near each other, so this showed nothing");
}

#[test]
fn a_roll_is_held_and_finished_in_order() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[
        (45, 0, q, Hand::Left),
        (57, 0, q / 32, Hand::Left),
        (60, 0, q / 32, Hand::Left),
        (45, 2 * q, q, Hand::Left),
        (57, 2 * q, q, Hand::Left),
        (60, 2 * q, q, Hand::Left),
        (62, 2 * q + 6, q, Hand::Left),
    ]);
    let fingerings = pinned(&[
        (0, Finger::Little),
        (1, Finger::Index),
        (2, Finger::Thumb),
        (3, Finger::Little),
        (4, Finger::Index),
        (5, Finger::Thumb),
        (6, Finger::Thumb),
    ]);
    let timeline = Timeline::build(&score, &fingerings);
    let grips = timeline.hand_grips(Hand::Left);
    let shown: Vec<_> = grips.iter().map(|e| (e.time, e.release, e.grip.keys.clone())).collect();
    assert!(grips.len() >= 4, "expected both chords rolled: {shown:?}");
    for pair in grips.windows(2) {
        assert!(pair[1].time >= pair[0].time, "grips out of order: {shown:?}");
    }
    for event in grips {
        assert!(event.release >= event.time, "a grip let go of before it is taken: {shown:?}");
    }
    let mut starts: Vec<f64> = timeline.notes.iter().filter(|n| n.start >= 1.0).map(|n| n.start).collect();
    starts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    assert!(starts.len() >= 2, "a rolled chord sounds as it is rolled: {starts:?}");
}

#[test]
fn hands_at_opposite_ends_are_not_touched() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[(30, 0, 4 * q, Hand::Left), (95, 0, 4 * q, Hand::Right)]);
    let timeline = Timeline::build(&score, &fingered(&score));
    let animators = timeline.animators(&HandProfile::default(), BiomechWeights::default());

    for hand in Hand::ALL {
        let alone = animators[hand as usize].pose_at(0.5);
        let together = pose_both(&animators, 0.5)[hand as usize];
        assert_eq!(
            alone.q[dof::WRIST_Z],
            together.q[dof::WRIST_Z],
            "{hand:?} was moved for a collision that is not there"
        );
    }
}

#[test]
fn the_wrist_sinks_into_a_note_and_comes_back() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[(60, 0, 4 * q, Hand::Right)]);
    let timeline = Timeline::build(&score, &fingered(&score));
    let animator = HandAnimator::new(
        Hand::Right,
        BiomechModel::new(HandProfile::default(), Hand::Right, BiomechWeights::default()),
        timeline.hand_grips(Hand::Right).to_vec(),
    );
    let height = |t: f64| animator.pose_at(t).q[dof::WRIST_Z];

    let settled = height(WRIST_SETTLE_SECONDS + 0.01);
    let sinking = height(WRIST_FALL_SECONDS);
    assert!(
        sinking < settled - 0.5,
        "the wrist should drop into the note: {settled:.2} settled, {sinking:.2} at \
         the bottom"
    );
    assert!((height(WRIST_SETTLE_SECONDS + 0.4) - settled).abs() < 0.01, "the wrist should come back up");
}

#[test]
fn the_sink_moves_something_the_camera_can_see() {
    let q = TICKS_PER_QUARTER as i64;
    let mut score = score_of(&[(60, 0, 4 * q, Hand::Right)]);
    score.notes[0].velocity = 120;
    let timeline = Timeline::build(&score, &fingered(&score));
    let animator = HandAnimator::new(
        Hand::Right,
        BiomechModel::new(HandProfile::default(), Hand::Right, BiomechWeights::default()),
        timeline.hand_grips(Hand::Right).to_vec(),
    );

    let flat = |t: f64| {
        let pose = animator.pose_at(t);
        let posture = animator.skeleton().forward(&pose);
        (posture.chain[Finger::Middle.index()][3].truncate() - posture.wrist.truncate()).length()
    };

    let settled = flat(WRIST_SETTLE_SECONDS + 0.05);
    let sunk = flat(WRIST_FALL_SECONDS);
    assert!(
        (settled - sunk).abs() > 2.0,
        "the hand has to visibly foreshorten as it takes the note: {settled:.2} mm \
         settled against {sunk:.2} mm sunk, on a keyboard whose white keys are \
         23.5 mm wide"
    );
}

#[test]
fn a_louder_note_is_played_with_more_arm() {
    let q = TICKS_PER_QUARTER as i64;
    let sink = |velocity: u8| {
        let mut score = score_of(&[(60, 0, 4 * q, Hand::Right)]);
        score.notes[0].velocity = velocity;
        let timeline = Timeline::build(&score, &fingered(&score));
        let animator = HandAnimator::new(
            Hand::Right,
            BiomechModel::new(HandProfile::default(), Hand::Right, BiomechWeights::default()),
            timeline.hand_grips(Hand::Right).to_vec(),
        );
        let settled = animator.pose_at(WRIST_SETTLE_SECONDS + 0.01).q[dof::WRIST_Z];
        settled - animator.pose_at(WRIST_FALL_SECONDS).q[dof::WRIST_Z]
    };

    let quiet = sink(20);
    let loud = sink(120);
    assert!(quiet > 0.0, "even a quiet note sinks a little: {quiet:.2}");
    assert!(loud > quiet * 1.8, "a loud note should sink much further: {quiet:.2} quiet, {loud:.2} loud");
}

#[test]
fn a_waiting_hand_is_not_perfectly_still() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[(60, 0, q / 4, Hand::Right), (60, 8 * q, q, Hand::Right)]);
    let timeline = Timeline::build(&score, &fingered(&score));
    let animator = HandAnimator::new(
        Hand::Right,
        BiomechModel::new(HandProfile::default(), Hand::Right, BiomechWeights::default()),
        timeline.hand_grips(Hand::Right).to_vec(),
    );

    let release = animator.events[0].release;
    assert_eq!(animator.breath(0, release - 0.01), 0.0);

    let mut lowest = f32::INFINITY;
    let mut highest = f32::NEG_INFINITY;
    for step in 0..60 {
        let drift = animator.breath(0, release + 1.0 + step as f64 * 0.1);
        lowest = lowest.min(drift);
        highest = highest.max(drift);
    }
    assert!(
        highest - lowest > 0.2,
        "a hand waiting through a rest should drift: {lowest:.2} to {highest:.2}"
    );
    assert!(highest - lowest <= 2.0 * BREATH_MM + 1e-3, "and only just: {lowest:.2} to {highest:.2}");
}

#[test]
fn the_two_hands_do_not_breathe_together() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[
        (60, 0, q / 4, Hand::Right),
        (48, 0, q / 4, Hand::Left),
        (60, 8 * q, q, Hand::Right),
        (48, 8 * q, q, Hand::Left),
    ]);
    let timeline = Timeline::build(&score, &fingered(&score));
    let animator = |hand: Hand| {
        HandAnimator::new(
            hand,
            BiomechModel::new(HandProfile::default(), hand, BiomechWeights::default()),
            timeline.hand_grips(hand).to_vec(),
        )
    };
    let (right, left) = (animator(Hand::Right), animator(Hand::Left));

    let apart = (0..40).any(|step| {
        let t = 1.0 + step as f64 * 0.1;
        (right.breath(0, t) - left.breath(0, t)).abs() > 0.1
    });
    assert!(apart, "both hands drifted identically");
}

#[test]
fn a_held_note_is_never_lifted_off() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[(60, 0, 4 * q, Hand::Right), (72, 4 * q, q, Hand::Right)]);
    let timeline = Timeline::build(&score, &fingered(&score));
    let animator = HandAnimator::new(
        Hand::Right,
        BiomechModel::new(HandProfile::default(), Hand::Right, BiomechWeights::default()),
        timeline.hand_grips(Hand::Right).to_vec(),
    );
    let resting = animator.pose_at(WRIST_SETTLE_SECONDS + 0.05).q[dof::WRIST_Z];
    for step in 0..20 {
        let t = WRIST_SETTLE_SECONDS + 0.05 + step as f64 * 0.09;
        let height = animator.pose_at(t).q[dof::WRIST_Z];
        assert!(height <= resting + 0.01, "the hand rose to {height:.2} at {t:.2}s while the key was held");
    }
}

#[test]
fn a_key_goes_down_ahead_of_its_sound_and_bottoms_just_after() {
    for velocity in [10u8, 30, 63, 64, 90, 127] {
        let q = TICKS_PER_QUARTER as i64;
        let mut score = score_of(&[(60, q, q, Hand::Right)]);
        score.notes[0].velocity = velocity;
        let timeline = Timeline::build(&score, &fingered(&score));
        let note = &timeline.notes[0];
        assert!(note.key_moves < note.start, "the key moves before the hammer sounds");
        let mut last = 0.0;
        let mut at = note.key_moves;
        while at < note.key_bottoms() {
            let depth = note.key_depth(at);
            assert!(depth >= last - 1e-9 && depth <= 1.0, "velocity {velocity}: {last} then {depth}");
            last = depth;
            at += 0.0005;
        }
        assert_eq!(note.key_depth(note.key_bottoms()), 1.0, "velocity {velocity}");
        let near = note.key_bottoms() - 0.03 * (note.key_bottoms() - note.key_moves);
        assert!(note.key_depth(near) > 0.99, "no jump at the bottom, velocity {velocity}");
    }
    assert!(key_seconds(110) < key_seconds(40), "a louder note goes down faster");
}

#[test]
fn a_repeated_key_comes_up_before_it_goes_down_again() {
    let q = TICKS_PER_QUARTER as i64;
    let mut score = score_of(&[(60, 0, q / 4, Hand::Right), (60, q / 4, q / 4, Hand::Right)]);
    score.notes[1].velocity = 20;
    let timeline = Timeline::build(&score, &fingered(&score));
    let (first, second) = (&timeline.notes[0], &timeline.notes[1]);
    assert!(second.key_moves > first.key_rises, "{} {}", second.key_moves, first.key_rises);
    assert_eq!(first.end, second.start, "the sound is untouched");
    assert!(timeline.key_depression(second.key_moves).depth_of(60) < 0.8);
}

#[test]
fn keys_go_down_when_struck_and_come_back_up() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[(60, 0, q, Hand::Right)]);
    let timeline = Timeline::build(&score, &fingered(&score));

    assert_eq!(timeline.key_depression(-0.1).depth_of(60), 0.0);
    assert!(timeline.key_depression(0.2).depth_of(60) > 0.99, "should be down");
    assert!(timeline.key_depression(0.6).depth_of(60) < 0.5, "should be rising");
    assert_eq!(timeline.key_depression(2.0).depth_of(60), 0.0);
}

#[test]
fn a_pressed_key_knows_which_hand_is_on_it() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[(60, 0, q, Hand::Right), (48, 0, q, Hand::Left)]);
    let timeline = Timeline::build(&score, &fingered(&score));
    let states = timeline.key_depression(0.2);
    assert_eq!(states.hand_on(60), Some(Hand::Right));
    assert_eq!(states.hand_on(48), Some(Hand::Left));
    assert_eq!(states.hand_on(72), None);
    assert_eq!(states.pressed().count(), 2);
}

#[test]
fn a_chord_becomes_one_grip_not_several() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[(60, 0, q, Hand::Right), (64, 0, q, Hand::Right), (67, 0, q, Hand::Right)]);
    let timeline = Timeline::build(&score, &fingered(&score));
    let grips = timeline.hand_grips(Hand::Right);
    assert_eq!(grips.len(), 1);
    assert_eq!(grips[0].grip.keys.len(), 3);
    assert!(grips[0].grip.keys.windows(2).all(|w| w[0].0 < w[1].0));
}

#[test]
fn the_hand_moves_between_chords_and_arrives_on_time() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[
        (60, 0, q, Hand::Right),
        (64, 0, q, Hand::Right),
        (79, 4 * q, q, Hand::Right),
        (84, 4 * q, q, Hand::Right),
    ]);
    let timeline = Timeline::build(&score, &fingered(&score));
    let model = BiomechModel::new(HandProfile::default(), Hand::Right, BiomechWeights::default());
    let animator = HandAnimator::new(Hand::Right, model, timeline.hand_grips(Hand::Right).to_vec());

    let first = animator.pose_at(0.1);
    let arrival = animator.pose_at(2.0);
    let wrist_start = first.wrist_position().x;
    let wrist_end = arrival.wrist_position().x;
    assert!(
        wrist_end > wrist_start + 100.0,
        "the hand should have travelled up the keyboard: {wrist_start} to {wrist_end}"
    );

    let target = animator.pose_at(2.0).wrist_position().x;
    let just_after = animator.pose_at(2.05).wrist_position().x;
    assert!((target - just_after).abs() < 1.0, "the hand should have settled");
}

#[test]
fn the_hand_holds_still_while_a_chord_is_held() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[(60, 0, 4 * q, Hand::Right), (67, 0, 4 * q, Hand::Right)]);
    let timeline = Timeline::build(&score, &fingered(&score));
    let model = BiomechModel::new(HandProfile::default(), Hand::Right, BiomechWeights::default());
    let animator = HandAnimator::new(Hand::Right, model, timeline.hand_grips(Hand::Right).to_vec());
    let a = animator.pose_at(0.3);
    let b = animator.pose_at(1.5);
    assert!((a.wrist_position() - b.wrist_position()).length() < 1e-3);
}

#[test]
fn movement_is_smooth_rather_than_a_jump() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[(60, 0, q, Hand::Right), (84, 2 * q, q, Hand::Right)]);
    let timeline = Timeline::build(&score, &fingered(&score));
    let model = BiomechModel::new(HandProfile::default(), Hand::Right, BiomechWeights::default());
    let animator = HandAnimator::new(Hand::Right, model, timeline.hand_grips(Hand::Right).to_vec());

    let mut previous = animator.pose_at(0.0).wrist_position().x;
    let mut biggest: f32 = 0.0;
    for step in 1..=200 {
        let t = step as f64 * 0.01;
        let x = animator.pose_at(t).wrist_position().x;
        biggest = biggest.max((x - previous).abs());
        previous = x;
    }
    assert!(biggest < 20.0, "the hand jumped {biggest} mm in one hundredth of a second");
}

#[test]
fn the_lane_shows_notes_before_they_sound() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[(60, 0, q, Hand::Right), (72, 8 * q, q, Hand::Right)]);
    let timeline = Timeline::build(&score, &fingered(&score));
    let visible: Vec<_> = timeline.visible_notes(0.0, 3.0).map(|n| n.midi).collect();
    assert_eq!(visible, vec![60], "the distant note should not be in the lane yet");
    let visible: Vec<_> = timeline.visible_notes(2.0, 3.0).map(|n| n.midi).collect();
    assert!(visible.contains(&72), "the note should have come into view");
}

#[test]
fn a_move_between_two_held_chords_starts_and_ends_still() {
    assert_eq!(travel(0.0, true, true), 0.0);
    assert_eq!(travel(1.0, true, true), 1.0);
    assert!((travel(0.5, true, true) - 0.5).abs() < 1e-9);
    assert!(travel(0.1, true, true) < 0.1);
    assert!(travel(0.9, true, true) > 0.9);
}

#[test]
fn a_move_the_hand_is_already_making_does_not_stop_for_it() {
    assert!((travel(0.0, false, false) - 0.0).abs() < 1e-9);
    assert!((travel(1.0, false, false) - 1.0).abs() < 1e-9);
    assert!(travel(0.05, false, false) > travel(0.05, true, true) * 2.0);
    assert!(1.0 - travel(0.95, false, false) > (1.0 - travel(0.95, true, true)) * 2.0);
}

#[test]
fn travel_never_runs_backwards() {
    for (before, after) in [(true, true), (true, false), (false, true), (false, false)] {
        let mut last = -1.0;
        for step in 0..=100 {
            let here = travel(step as f64 / 100.0, before, after);
            assert!(
                here >= last - 1e-9,
                "went backwards at {step} with {before}/{after}: {here} after {last}"
            );
            last = here;
        }
    }
}

fn lowest_tip(animator: &HandAnimator, time: f64) -> f32 {
    let posture = animator.skeleton().forward(&animator.pose_at(time));
    Finger::ALL
        .into_iter()
        .map(|f| posture.joint(on_hand::skeleton::Joint::Tip(f)).z)
        .fold(f32::INFINITY, f32::min)
}

fn animator_for(score: &Score, fingerings: &[Fingering], hand: Hand) -> HandAnimator {
    let options = on_fingering::FingeringOptions::default();
    let timeline = Timeline::build(score, fingerings);
    let model = BiomechModel::new(options.profile.clone(), hand, options.biomech);
    HandAnimator::new(hand, model, timeline.hand_grips(hand).to_vec())
}

#[test]
fn a_repeated_note_lifts_the_finger_and_puts_it_back() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[
        (64, 0, q, Hand::Right),
        (64, q, q, Hand::Right),
        (64, 2 * q, q, Hand::Right),
        (64, 3 * q, q, Hand::Right),
    ]);
    let fingerings =
        pinned(&[(0, Finger::Middle), (1, Finger::Middle), (2, Finger::Middle), (3, Finger::Middle)]);
    let animator = animator_for(&score, &fingerings, Hand::Right);

    let down = lowest_tip(&animator, 1.0);
    let between = (0..12).map(|i| lowest_tip(&animator, 0.84 + f64::from(i) * 0.01)).fold(0.0f32, f32::max);
    assert!(down < 1.0, "the finger should be on the key on the beat: {down}");
    assert!(between > 3.0, "and off it beforehand, or the note repeats without the hand moving: {between}");
}

#[test]
fn a_repeated_note_is_played_from_the_wrist_more_than_the_finger() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[
        (64, 0, q, Hand::Right),
        (64, q, q, Hand::Right),
        (64, 2 * q, q, Hand::Right),
        (64, 3 * q, q, Hand::Right),
    ]);
    let fingerings =
        pinned(&[(0, Finger::Middle), (1, Finger::Middle), (2, Finger::Middle), (3, Finger::Middle)]);
    let animator = animator_for(&score, &fingerings, Hand::Right);
    let middle = dof::finger(1);
    let struck = animator.pose_at(1.0);
    let raised = (0..30)
        .map(|i| animator.pose_at(0.6 + f64::from(i) * 0.01))
        .min_by(|a, b| a.q[dof::WRIST_FLEXION].total_cmp(&b.q[dof::WRIST_FLEXION]))
        .expect("sampled");
    let wrist = (struck.q[dof::WRIST_FLEXION] - raised.q[dof::WRIST_FLEXION]).to_degrees();
    let finger = (struck.q[middle + dof::MCP_FLEX] - raised.q[middle + dof::MCP_FLEX]).abs().to_degrees();
    assert!(wrist > 6.0, "the hand should pivot up from the wrist between notes: {wrist:.1} degrees");
    assert!(finger < wrist, "more than the knuckle bends: knuckle {finger:.1}, wrist {wrist:.1} degrees");
}

#[test]
fn a_louder_note_is_dropped_from_higher_and_faster() {
    let q = TICKS_PER_QUARTER as i64;
    let quiet = {
        let mut s = score_of(&[(64, 0, q, Hand::Right), (64, q, q, Hand::Right)]);
        for n in &mut s.notes {
            n.velocity = 30;
        }
        s
    };
    let loud = {
        let mut s = score_of(&[(64, 0, q, Hand::Right), (64, q, q, Hand::Right)]);
        for n in &mut s.notes {
            n.velocity = 125;
        }
        s
    };
    let fingerings = pinned(&[(0, Finger::Middle), (1, Finger::Middle)]);

    let at = 0.40;
    let quiet_height = lowest_tip(&animator_for(&quiet, &fingerings, Hand::Right), at);
    let loud_height = lowest_tip(&animator_for(&loud, &fingerings, Hand::Right), at);
    assert!(
        loud_height > quiet_height + 1.0,
        "the loud note should still be up at {at}: loud {loud_height}, quiet {quiet_height}"
    );
}

#[test]
fn a_chord_wider_than_the_hand_is_rolled_rather_than_dropped() {
    let q = TICKS_PER_QUARTER as i64;
    let score = score_of(&[(45, 0, 4 * q, Hand::Left), (57, 0, q, Hand::Left), (60, 0, q, Hand::Left)]);
    let fingerings = pinned(&[(0, Finger::Little), (1, Finger::Index), (2, Finger::Thumb)]);
    let timeline = Timeline::build(&score, &fingerings);
    let grips = timeline.hand_grips(Hand::Left);

    for note in timeline.notes.iter().filter(|n| n.hand == Hand::Left) {
        assert!(
            grips.iter().any(|event| {
                event.time >= note.start - 1e-6
                    && event.time <= note.start + 0.12
                    && event.grip.keys.iter().any(|(midi, _)| *midi == note.midi)
            }),
            "{} was struck with no finger on it; grips were {:?}",
            note.midi,
            grips.iter().map(|e| (e.time, e.grip.keys.clone())).collect::<Vec<_>>()
        );
    }

    assert!(grips.len() >= 2, "expected a roll, got {} grip(s)", grips.len());
    assert!(
        grips[0].grip.keys.iter().any(|(midi, _)| *midi == 45),
        "the bottom of the chord goes down first"
    );
    assert!(grips[1].time > grips[0].time, "the rest of it arrives afterwards");
}

#[test]
fn a_hand_never_takes_a_finger_off_a_note_it_is_striking() {
    let q = TICKS_PER_QUARTER as i64;
    let score =
        score_of(&[(28, 0, 8 * q, Hand::Left), (40, 0, 8 * q, Hand::Left), (52, 4 * q, q, Hand::Left)]);
    let fingerings = pinned(&[(0, Finger::Little), (1, Finger::Thumb), (2, Finger::Thumb)]);
    let timeline = Timeline::build(&score, &fingerings);
    let grips = timeline.hand_grips(Hand::Left);
    for note in timeline.notes.iter().filter(|n| n.hand == Hand::Left) {
        assert!(
            grips.iter().any(|event| {
                event.time >= note.start - 1e-6
                    && event.time <= note.start + 0.12
                    && event.grip.keys.iter().any(|(midi, _)| *midi == note.midi)
            }),
            "{} was struck with no finger on it",
            note.midi
        );
    }
}
