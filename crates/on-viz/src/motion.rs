use glam::{Vec2, Vec3};
use on_fingering::biomech::{BiomechModel, Grip};
use on_fingering::playability::HAND_SPEED_MM_PER_SECOND;
use on_hand::keyboard::{is_black, Keyboard, BLACK_KEY_DIP, BLACK_KEY_FRONT_Y, BLACK_KEY_HEIGHT, KEY_DIP, WHITE_KEY_LENGTH};
use on_hand::skeleton::{dof, HandPose, Posture, Skeleton, DOF};
use on_hand::{Finger, Hand};

use crate::timeline::{key_seconds, GripEvent, TimelineNote};

const KEY_LEAD: f64 = 0.85;

const KEY_TRAIL: f64 = 0.15;

const HOLD_LEAST: f64 = 0.04;

const MIN_KNOT_GAP: f64 = 0.01;

const SMOOTH_SIGMA: f64 = 0.09;

const SMOOTH_HOLD_WEIGHT: f64 = 4.0;

const LEAN: [f32; DOF] = {
    let mut weights = [0.3f32; DOF];
    weights[0] = 2.0;
    weights[1] = 2.0;
    weights[2] = 2.0;
    weights[3] = 50.0;
    weights[4] = 50.0;
    weights[5] = 50.0;
    weights
};

const STRIKE_WINDOW: f64 = 0.22;

const STRIKE_LIFT_MM: (f32, f32) = (5.0, 10.0);

const FINGER_SPEED_MM: f32 = 400.0;

const LET_GO_SECONDS: f64 = 0.06;

const FADE_SECONDS: f64 = 0.08;

const TAKE_HOLD: f64 = 0.4;

const LEAST_STRIKE: f64 = 0.05;

const FINGER_TRAVEL_MM_PER_SECOND: f64 = 500.0;

const SAME_FINGER_GAP: (f64, f64) = (0.02, 0.15);

const REACH_DAMPING: f32 = 400.0;

const CURL_STIFFNESS: f32 = 4.0;

const SPREAD_STIFFNESS: f32 = 2.0;

const MCP_LEAST_DEG: f32 = -5.0;

const REACH_STEP: f32 = 0.35;

const REACH_PASSES: usize = 4;

const REACH_GIVE_MM: (f32, f32) = (6.0, 14.0);

const AIM_PULL_MM: f32 = 12.0;

const SPREAD_ROOM: f32 = 0.15;

const KEY_EDGE_MM: f32 = 6.0;

const KEY_CLEAR_MM: f32 = 3.0;

const KEY_EDGE_INWARD_MM: f32 = 2.0;

const BEND_ROOM: f32 = 0.7;

const Z_PULL_MM: (f32, f32) = (18.0, 25.0);

const REACH_AXES: Vec3 = Vec3::new(1.0, 0.25, 1.0);

const LEVER_MM: (f32, f32) = (5.0, 20.0);

const TOGETHER: f64 = on_fingering::playability::CHORD_SECONDS;

const STROKE_MM: (f32, f32) = (2.5, 8.0);

const HAND_PREPARE: f64 = 0.25;

const HAND_SHARE: (f32, f32, f32, f32) = (0.85, 0.7, 0.6, 0.15);

const HAND_SLOW: (f64, f64) = (0.25, 0.3);

const HAND_RUN: (f64, f64) = (0.1, 0.15);

const HAND_LIFT_RATE: f64 = 70.0;

const HAND_RELEASE: f64 = 0.8;

const STROKE_RECOVER: (f64, f64) = (0.12, 0.35);

const STROKE_ROOM: (f64, f64) = (0.15, 0.3);


const HOP_ROOM: (f64, f64) = (0.08, 0.2);

pub fn minimum_jerk(s: f64) -> f64 {
    let s = s.clamp(0.0, 1.0);
    s * s * s * (10.0 - 15.0 * s + 6.0 * s * s)
}

fn quintic(p0: f32, p1: f32, v0: f32, v1: f32, span: f64, s: f64) -> f32 {
    let s = s.clamp(0.0, 1.0);
    let (s3, s4, s5) = (s * s * s, s * s * s * s, s * s * s * s * s);
    let h0 = 1.0 - 10.0 * s3 + 15.0 * s4 - 6.0 * s5;
    let h1 = s - 6.0 * s3 + 8.0 * s4 - 3.0 * s5;
    let h3 = -4.0 * s3 + 7.0 * s4 - 3.0 * s5;
    let h5 = 10.0 * s3 - 15.0 * s4 + 6.0 * s5;
    (f64::from(p0) * h0 + f64::from(p1) * h5 + span * (f64::from(v0) * h1 + f64::from(v1) * h3)) as f32
}

#[derive(Debug, Clone, Copy)]
pub struct Knot {
    pub time: f64,
    pub pose: HandPose,
    pub still: bool,
    pub event: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Stay {
    pub arrive: f64,
    pub leave: f64,
}

pub struct Track {
    knots: Vec<Knot>,
    slopes: Vec<[f32; DOF]>,
    pub stays: Vec<Stay>,
}

pub fn move_seconds(from: &HandPose, to: &HandPose) -> f64 {
    let distance = f64::from(from.wrist_position().distance(to.wrist_position()));
    let fitts = 0.10 + 0.07 * (1.0 + distance / 20.0).log2();
    fitts.clamp(0.12, 0.50).max(1.875 * distance / HAND_SPEED_MM_PER_SECOND)
}

pub fn arrival(event: &GripEvent) -> f64 {
    let softest = event.struck.iter().map(|(_, v)| *v).min();
    softest.map_or(event.time, |v| event.time - KEY_LEAD * key_seconds(v))
}

fn settled(event: &GripEvent) -> f64 {
    let softest = event.struck.iter().map(|(_, v)| *v).min();
    softest.map_or(event.time, |v| event.time + KEY_TRAIL * key_seconds(v))
}

pub const REST_GAP_SECONDS: f64 = 1.5;

impl Track {
    pub fn new(events: &[GripEvent], poses: &[HandPose], rest: &dyn Fn(f64, f64, &HandPose) -> HandPose) -> Self {
        let mut points: Vec<f64> = Vec::with_capacity(events.len());
        for event in events {
            let last = points.last().map_or(f64::MIN, |t| t + MIN_KNOT_GAP);
            points.push(event.time.max(last));
        }
        let mut stays: Vec<Stay> = points.iter().map(|t| Stay { arrive: *t, leave: *t }).collect();
        if let (Some(stay), Some(event)) = (stays.first_mut(), events.first()) {
            stay.arrive = arrival(event).min(stay.arrive);
        }
        for i in 0..stays.len().saturating_sub(1) {
            let ready = arrival(&events[i + 1]).max(points[i] + MIN_KNOT_GAP);
            let need = move_seconds(&poses[i], &poses[i + 1]);
            if ready - settled(&events[i]).max(points[i]) >= need {
                stays[i].leave = ready - need;
                stays[i + 1].arrive = ready.min(points[i + 1]);
            }
        }
        if let (Some(stay), Some(event)) = (stays.last_mut(), events.last()) {
            stay.leave = stay.arrive.max(event.release);
        }
        let last = stays.len().saturating_sub(1);
        for (i, stay) in stays.iter_mut().enumerate() {
            if i != 0 && i != last && stay.leave - stay.arrive < HOLD_LEAST {
                *stay = Stay { arrive: points[i], leave: points[i] };
            }
        }
        let mut resting = vec![false; stays.len()];
        for i in 0..stays.len().saturating_sub(1) {
            let done = events[i].release.max(stays[i].arrive);
            if stays[i + 1].arrive - done >= REST_GAP_SECONDS {
                stays[i].leave = stays[i].leave.min(done);
                resting[i] = true;
            }
        }

        let mut knots = Vec::with_capacity(2 * events.len() + 2);
        if let (Some(stay), Some(pose)) = (stays.first(), poses.first()) {
            let rest = rest(f64::NEG_INFINITY, stay.arrive, pose);
            knots.push(Knot { time: stay.arrive - move_seconds(&rest, pose), pose: rest, still: true, event: 0 });
        }
        for (i, stay) in stays.iter().enumerate() {
            let held = stay.leave > stay.arrive + 1e-3 || i == 0 || i + 1 == stays.len();
            knots.push(Knot { time: stay.arrive, pose: poses[i], still: held, event: i });
            if stay.leave > stay.arrive + 1e-3 {
                knots.push(Knot { time: stay.leave, pose: poses[i], still: true, event: i });
            }
            if let Some(next) = stays.get(i + 1).filter(|_| resting[i]) {
                let resting = rest(stay.leave, next.arrive, &poses[i + 1]);
                let away = stay.leave + move_seconds(&poses[i], &resting);
                let back = next.arrive - move_seconds(&resting, &poses[i + 1]);
                if back > away + MIN_KNOT_GAP {
                    knots.push(Knot { time: away, pose: resting, still: true, event: i });
                    knots.push(Knot { time: back, pose: resting, still: true, event: i + 1 });
                }
            }
        }
        if let (Some(stay), Some(pose)) = (stays.last(), poses.last()) {
            let rest = rest(stay.leave, f64::INFINITY, pose);
            let event = stays.len() - 1;
            knots.push(Knot { time: stay.leave + move_seconds(pose, &rest), pose: rest, still: true, event });
        }

        let slopes = (0..knots.len())
            .map(|k| {
                let mut v = [0.0f32; DOF];
                if knots[k].still || k == 0 || k + 1 == knots.len() {
                    return v;
                }
                let (a, b, c) = (&knots[k - 1], &knots[k], &knots[k + 1]);
                for (i, slot) in v.iter_mut().enumerate() {
                    let d0 = (b.pose.q[i] - a.pose.q[i]) / (b.time - a.time) as f32;
                    let d1 = (c.pose.q[i] - b.pose.q[i]) / (c.time - b.time) as f32;
                    if d0 * d1 > 0.0 {
                        let harmonic = 2.0 * d0 * d1 / (d0 + d1);
                        let bound = 1.5 * d0.abs().min(d1.abs());
                        *slot = harmonic.clamp(-bound, bound);
                    }
                }
                v
            })
            .collect();
        Self { knots, slopes, stays }
    }

    pub fn span(&self) -> (f64, f64) {
        match (self.knots.first(), self.knots.last()) {
            (Some(first), Some(last)) => (first.time, last.time),
            _ => (0.0, 0.0),
        }
    }

    fn segment(&self, time: f64) -> Option<(usize, f64)> {
        if self.knots.len() < 2 || time <= self.knots[0].time {
            return None;
        }
        let k = self.knots.partition_point(|knot| knot.time <= time);
        if k >= self.knots.len() {
            return None;
        }
        let (a, b) = (&self.knots[k - 1], &self.knots[k]);
        Some((k - 1, (time - a.time) / (b.time - a.time)))
    }

    pub fn at(&self, time: f64) -> HandPose {
        let Some((k, s)) = self.segment(time) else {
            return if time <= self.knots[0].time { self.knots[0].pose } else { self.knots[self.knots.len() - 1].pose };
        };
        let (a, b) = (&self.knots[k], &self.knots[k + 1]);
        let span = b.time - a.time;
        let mut pose = a.pose;
        for i in 0..DOF {
            pose.q[i] = quintic(a.pose.q[i], b.pose.q[i], self.slopes[k][i], self.slopes[k + 1][i], span, s);
        }
        pose
    }

    pub fn travelling(&self, time: f64) -> Option<(usize, usize, f32)> {
        let (k, s) = self.segment(time)?;
        let (a, b) = (&self.knots[k], &self.knots[k + 1]);
        if a.event == b.event {
            return Some((a.event, a.event, 0.0));
        }
        Some((a.event, b.event, minimum_jerk(s) as f32))
    }
}

const LAG_RATE: f64 = 240.0;

const LAG_HZ: f64 = 7.0;

const LAG_DAMPING: f64 = 0.55;

const FIRST_DIGIT: usize = dof::THUMB_CMC_FLEX;

const DIGITS: usize = DOF - FIRST_DIGIT;

pub struct Lag {
    start: f64,
    rows: Vec<[f32; DIGITS]>,
}

impl Lag {
    pub fn new(track: &Track, from: f64, until: f64) -> Self {
        let dt = 1.0 / LAG_RATE;
        let omega = std::f64::consts::TAU * LAG_HZ;
        let samples = ((until - from).max(0.0) * LAG_RATE).ceil() as usize + 2;
        let first = track.at(from);
        let mut x: [f64; DIGITS] = std::array::from_fn(|i| f64::from(first.q[FIRST_DIGIT + i]));
        let mut v = [0.0f64; DIGITS];
        let mut rows = Vec::with_capacity(samples);
        for n in 0..samples {
            let target = track.at(from + n as f64 * dt);
            let mut row = [0.0f32; DIGITS];
            for i in 0..DIGITS {
                let goal = f64::from(target.q[FIRST_DIGIT + i]);
                v[i] += dt * (omega * omega * (goal - x[i]) - 2.0 * LAG_DAMPING * omega * v[i]);
                x[i] += dt * v[i];
                row[i] = (x[i] - goal) as f32;
            }
            rows.push(row);
        }
        Self { start: from, rows }
    }

    pub fn at(&self, time: f64) -> [f32; DIGITS] {
        let u = (time - self.start) * LAG_RATE;
        if u <= 0.0 || self.rows.len() < 4 {
            return [0.0; DIGITS];
        }
        let i = u.floor() as usize;
        if i + 2 >= self.rows.len() {
            return [0.0; DIGITS];
        }
        let f = (u - u.floor()) as f32;
        let p = |k: usize| self.rows[k.min(self.rows.len() - 1)];
        let (a, b, c, d) = (p(i.saturating_sub(1)), p(i), p(i + 1), p(i + 2));
        std::array::from_fn(|j| {
            let (a, b, c, d) = (a[j], b[j], c[j], d[j]);
            b + 0.5 * f * (c - a + f * (2.0 * a - 5.0 * b + 4.0 * c - d + f * (3.0 * (b - c) + d - a)))
        })
    }
}

pub fn digit_of(index: usize) -> Finger {
    match index {
        i if i < dof::FINGER_BASE => Finger::Thumb,
        i => Finger::ALL[1 + (i - dof::FINGER_BASE) / 3],
    }
}

pub fn smooth_wrists(events: &[GripEvent], poses: &[HandPose], model: &BiomechModel) -> Vec<HandPose> {
    let reach = 3.0 * SMOOTH_SIGMA;
    let times: Vec<f64> = events.iter().map(arrival).collect();
    let weights: Vec<f64> = events
        .iter()
        .map(|e| 1.0 + SMOOTH_HOLD_WEIGHT * (e.release - e.time).clamp(0.0, 0.5))
        .collect();
    let mut out = poses.to_vec();
    for i in 0..events.len() {
        if events[i].grip.keys.is_empty() {
            continue;
        }
        let mut sum = [0.0f64; 6];
        let mut total = 0.0;
        for j in 0..events.len() {
            let gap = times[j] - times[i];
            if gap.abs() > reach || events[j].grip.keys.is_empty() {
                continue;
            }
            let g = (-0.5 * (gap / SMOOTH_SIGMA).powi(2)).exp() * weights[j];
            for (d, slot) in sum.iter_mut().enumerate() {
                *slot += g * f64::from(poses[j].q[d]);
            }
            total += g;
        }
        let smoothed: [f32; 6] = std::array::from_fn(|d| (sum[d] / total) as f32);
        let moved = (0..3).any(|d| (smoothed[d] - poses[i].q[d]).abs() > 1.0)
            || (3..6).any(|d| (smoothed[d] - poses[i].q[d]).abs() > 0.01);
        if !moved {
            continue;
        }
        let mut near = poses[i];
        near.q[..6].copy_from_slice(&smoothed);
        let (pose, error) = model.grip_pose_near(&events[i].grip, &near, LEAN);
        if error <= on_hand::ik::CONTACT_TOLERANCE_MM {
            out[i] = pose;
        }
    }
    out
}

const IDLE_THUMB_LIFT: Vec3 = Vec3::new(0.0, -6.0, 12.0);

const IDLE_CLEAR_MM: f32 = 5.0;

const IDLE_CURL_DEG: (f32, f32) = (25.0, 50.0);

fn five_fingers(hand: Hand) -> Grip {
    Grip::new(match hand {
        Hand::Right => vec![(60, Finger::Thumb), (62, Finger::Index), (64, Finger::Middle), (65, Finger::Ring), (67, Finger::Little)],
        Hand::Left => vec![(48, Finger::Little), (50, Finger::Ring), (52, Finger::Middle), (53, Finger::Index), (55, Finger::Thumb)],
    })
}

pub fn relax_idle_fingers(events: &[GripEvent], poses: &mut [HandPose], model: &BiomechModel) {
    let skeleton = model.skeleton();
    let mut relaxed = model.grip_pose(&five_fingers(skeleton.hand()));
    let target = skeleton.forward(&relaxed).tip(Finger::Thumb) + IDLE_THUMB_LIFT;
    for _ in 0..12 {
        let posture = skeleton.forward(&relaxed);
        reach_step(skeleton, &posture, &mut relaxed, Finger::Thumb, target);
        skeleton.clamp(&mut relaxed);
    }
    for slot in 0..4 {
        let base = dof::finger(slot);
        relaxed.q[base + dof::MCP_FLEX] = IDLE_CURL_DEG.0.to_radians();
        relaxed.q[base + dof::PIP_FLEX] = IDLE_CURL_DEG.1.to_radians();
    }
    for (event, pose) in events.iter().zip(poses.iter_mut()) {
        for finger in Finger::ALL {
            if event.grip.keys.iter().any(|(_, f)| *f == finger) {
                continue;
            }
            let joints = digit_dofs(finger);
            let original = *pose;
            let blended = |share: f32| {
                let mut out = original;
                for j in joints {
                    out.q[*j] += share * (relaxed.q[*j] - original.q[*j]);
                }
                out
            };
            let clear = |pose: &HandPose| {
                let tip = skeleton.forward(pose).tip(finger);
                let surface = if tip.y > BLACK_KEY_FRONT_Y - KEY_EDGE_MM { BLACK_KEY_HEIGHT } else { 0.0 };
                tip.y < 0.0 || tip.z >= surface + IDLE_CLEAR_MM
            };
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            if clear(&blended(1.0)) {
                lo = 1.0;
            } else {
                for _ in 0..8 {
                    let mid = 0.5 * (lo + hi);
                    if clear(&blended(mid)) {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
            }
            *pose = blended(lo);
        }
    }
}

fn struck_keys(event: &GripEvent) -> Vec<u8> {
    let mut keys: Vec<u8> = event
        .struck
        .iter()
        .filter_map(|(finger, _)| event.grip.keys.iter().find(|(_, f)| f == finger).map(|(m, _)| *m))
        .collect();
    keys.sort_unstable();
    keys
}

pub fn hand_share(events: &[GripEvent], i: usize) -> f32 {
    let event = &events[i];
    let keys = struck_keys(event);
    let before = if i > 0 { event.time - events[i - 1].time } else { f64::INFINITY };
    let repeated = i > 0 && !keys.is_empty() && struck_keys(&events[i - 1]) == keys;
    let base = if repeated {
        HAND_SHARE.0
    } else if keys.len() >= 2 {
        HAND_SHARE.1
    } else {
        HAND_SHARE.3 * minimum_jerk((before - HAND_RUN.0) / HAND_RUN.1) as f32
    };
    base.max(HAND_SHARE.2 * minimum_jerk((before - HAND_SLOW.0) / HAND_SLOW.1) as f32)
}

pub fn strike_lift(velocity: u8) -> f32 {
    STRIKE_LIFT_MM.0 + STRIKE_LIFT_MM.1 * f32::from(velocity) / 127.0
}

pub fn stroke(events: &[GripEvent], time: f64) -> (f32, f32) {
    let first = events.partition_point(|e| e.time < time - STROKE_RECOVER.1 - 0.05);
    let (mut down, mut up) = (0.0f32, 0.0f32);
    for (i, event) in events.iter().enumerate().skip(first) {
        if event.time > time + HAND_PREPARE + 0.12 {
            break;
        }
        let Some(hardest) = event.struck.iter().map(|(_, v)| *v).max() else { continue };
        let before = if i > 0 { event.time - events[i - 1].time } else { f64::INFINITY };
        let after = events.get(i + 1).map_or(f64::INFINITY, |e| e.time - event.time);
        let loud = f32::from(hardest) / 127.0;
        let room = minimum_jerk((before - STROKE_ROOM.0) / STROKE_ROOM.1) as f32;
        let depth = (STROKE_MM.0 + STROKE_MM.1 * loud.powf(1.5)) * room;
        let key = key_seconds(hardest);
        let moves = event.time - KEY_LEAD * key;
        let bottom = event.time + KEY_TRAIL * key;
        let prepare = HAND_PREPARE.min(0.6 * before);
        let lift = (hand_share(events, i) * strike_lift(hardest)).min((before * HAND_LIFT_RATE) as f32);
        let recover = (0.8 * after).clamp(STROKE_RECOVER.0, STROKE_RECOVER.1);
        if time >= moves - prepare && time < moves {
            up += lift * bump((time - moves + prepare) / prepare);
        } else if time >= moves && time < bottom {
            down = down.max(depth * minimum_jerk((time - moves) / (bottom - moves)) as f32);
        } else if time >= bottom {
            down = down.max(depth * (1.0 - minimum_jerk((time - bottom) / recover)) as f32);
        }
    }
    (down, up)
}

pub fn hop_room(gap: f64) -> f32 {
    minimum_jerk((gap - HOP_ROOM.0) / HOP_ROOM.1) as f32
}

pub fn bump(s: f64) -> f32 {
    let s = s.clamp(0.0, 1.0);
    (64.0 * s.powi(3) * (1.0 - s).powi(3)) as f32
}

fn surface(midi: u8) -> (f32, f32) {
    if is_black(midi) {
        (BLACK_KEY_HEIGHT, BLACK_KEY_DIP)
    } else {
        (0.0, KEY_DIP)
    }
}

struct Strike {
    note: TimelineNote,
    aim: Vec2,
    reach: Vec2,
    share: f32,
}

#[derive(Debug, Clone, Copy)]
struct Want {
    z: f32,
    weight: f32,
    floor: f32,
    aim: (Vec2, f32),
}

pub struct Strikes {
    by_finger: [Vec<Strike>; 5],
    pub lift: f32,
}

impl Strikes {
    pub fn new(
        notes: &[TimelineNote],
        events: &[GripEvent],
        stays: &[Stay],
        poses: &[HandPose],
        skeleton: &Skeleton,
    ) -> Self {
        let mut by_finger: [Vec<Strike>; 5] = Default::default();
        for note in notes {
            let Some(finger) = note.finger else { continue };
            let mut note = note.clone();
            let first = events.partition_point(|e| e.time < note.start - TOGETHER);
            let holding = |e: &GripEvent| e.grip.keys.contains(&(note.midi, finger));
            let mut near = events[first..].iter().take_while(|e| e.time <= note.start + TOGETHER);
            let Some(struck) = near.position(holding).map(|k| first + k) else { continue };
            let held = struck + events[struck..].iter().take_while(|e| holding(e)).count() - 1;
            let stay = stays[held];
            let release_by = if stay.leave > stay.arrive {
                stay.leave - 0.5 * LET_GO_SECONDS
            } else {
                stays.get(held + 1).map_or(f64::INFINITY, |s| s.arrive)
            };
            note.key_rises = note.key_rises.min(release_by).max(note.key_bottoms());
            let aim = skeleton.forward(&poses[struck]).tip(finger).truncate();
            let reach = aim - poses[struck].wrist_position().truncate();
            let share = hand_share(events, struck);
            by_finger[finger.index()].push(Strike { note, aim, reach, share });
        }
        for list in &mut by_finger {
            list.sort_by(|a, b| a.note.key_moves.total_cmp(&b.note.key_moves));
            for i in 1..list.len() {
                let next = list[i].note.key_moves;
                let travel = f64::from(list[i].reach.distance(list[i - 1].reach)) / FINGER_TRAVEL_MM_PER_SECOND;
                let between = list[i].note.start - list[i - 1].note.start;
                let lifted = f64::from(list[i].share) * HAND_RELEASE * HAND_PREPARE.min(0.6 * between);
                let gap = travel.clamp(SAME_FINGER_GAP.0, SAME_FINGER_GAP.1).max(lifted - LET_GO_SECONDS);
                let note = &mut list[i - 1].note;
                note.key_rises = note.key_rises.min(next - LET_GO_SECONDS - gap).max(note.key_bottoms());
            }
        }
        Self { by_finger, lift: 1.0 }
    }

    fn plan(&self, finger: Finger, time: f64) -> Option<Plan> {
        let list = &self.by_finger[finger.index()];
        let after = list.partition_point(|s| s.note.key_moves <= time);
        let mut going = None;
        if after > 0 {
            let Strike { note, aim, .. } = &list[after - 1];
            let (top, dip) = surface(note.midi);
            let up = note.key_rises + LET_GO_SECONDS;
            if time <= up {
                let z = top - note.key_depth(time) as f32 * dip;
                going = Some(Want { z, weight: 1.0, floor: 0.0, aim: (*aim, 1.0) });
            } else if time < up + FADE_SECONDS {
                let u = minimum_jerk((time - up) / FADE_SECONDS) as f32;
                going = Some(Want { z: top, weight: 1.0 - u, floor: u, aim: (*aim, 1.0 - u) });
            }
        }
        let held = going.and_then(|w| list.get(after.wrapping_sub(1)).map(|s| (s.note.midi, w.weight)));
        let alone = going.map(|w| Plan { going: Some(w), striking: None, taking: 0.0, own: [held, None] });
        let Some(Strike { note: next, aim, share, .. }) = list.get(after) else { return alone };
        let free_from = if after > 0 { list[after - 1].note.key_rises + LET_GO_SECONDS } else { f64::MIN };
        let from = (next.key_moves - STRIKE_WINDOW).max(free_from).min(next.key_moves - LEAST_STRIKE);
        if time < from {
            return alone;
        }
        let window = (next.key_moves - from) as f32;
        let height = (strike_lift(next.velocity) * (1.0 - *share) * self.lift).min(0.5 * window * FINGER_SPEED_MM);
        let contact = (1.5 * if is_black(next.midi) { BLACK_KEY_DIP } else { KEY_DIP }) as f64 / key_seconds(next.velocity);
        let peak = from + 0.55 * (next.key_moves - from);
        let (top, _) = surface(next.midi);
        let span = next.key_moves - from;
        let taking = minimum_jerk((time - from) / (TAKE_HOLD * span).max(span.min(LEAST_STRIKE))) as f32;
        let raised = if time < peak {
            f64::from(height) * minimum_jerk((time - from) / (peak - from))
        } else {
            let span = next.key_moves - peak;
            let s = ((time - peak) / span).clamp(0.0, 1.0);
            let (s2, s3) = (s * s, s * s * s);
            f64::from(height) * (2.0 * s3 - 3.0 * s2 + 1.0) - contact * span * (s3 - s2)
        };
        let floor = if time < peak { 1.0 } else { 1.0 - minimum_jerk((time - peak) / (next.key_moves - peak)) as f32 };
        let striking = Want { z: top + raised as f32, weight: 1.0, floor, aim: (*aim, 1.0 - floor) };
        Some(Plan { going, striking: Some(striking), taking, own: [held, Some((next.midi, taking))] })
    }

    pub fn engaged(&self, finger: Finger, time: f64) -> f32 {
        self.plan(finger, time).map_or(0.0, |p| p.weight())
    }

    pub fn press(&self, skeleton: &Skeleton, pose: &mut HandPose, time: f64) {
        let natural = skeleton.forward(pose);
        let targets: Vec<(Finger, Vec3)> = Finger::ALL
            .iter()
            .filter_map(|f| {
                let plan = self.plan(*f, time);
                let free = above_the_keys(natural.tip(*f), plan.map_or(0.0, |p| p.weight()));
                let mut target = plan.map_or(free, |p| p.target(free));
                target.z = target.z.max(clear_of_keys(target, plan.map_or([None; 2], |p| p.own)));
                (target != natural.tip(*f)).then_some((*f, target))
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        let before = *pose;
        for _ in 0..REACH_PASSES {
            let posture = skeleton.forward(pose);
            for (finger, target) in &targets {
                reach_step(skeleton, &posture, pose, *finger, *target);
                for j in digit_dofs(*finger) {
                    let room = if is_spread(*j) { SPREAD_ROOM } else { BEND_ROOM };
                    pose.q[*j] = pose.q[*j].clamp(before.q[*j] - room, before.q[*j] + room);
                    if is_knuckle(*j) {
                        pose.q[*j] = pose.q[*j].max(MCP_LEAST_DEG.to_radians().min(before.q[*j]));
                    }
                }
            }
            skeleton.clamp(pose);
        }
        let reached = skeleton.forward(pose);
        for (finger, target) in &targets {
            let miss = ((reached.tip(*finger) - *target) * REACH_AXES).length();
            let give = minimum_jerk(f64::from((miss - REACH_GIVE_MM.0) / REACH_GIVE_MM.1)) as f32;
            if give > 0.0 {
                for j in digit_dofs(*finger) {
                    pose.q[*j] += give * (before.q[*j] - pose.q[*j]);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Plan {
    going: Option<Want>,
    striking: Option<Want>,
    taking: f32,
    own: [Option<(u8, f32)>; 2],
}

impl Plan {
    fn weight(&self) -> f32 {
        let going = self.going.map_or(0.0, |w| w.weight);
        if self.striking.is_some() { going.max(self.taking) } else { going }
    }

    fn target(&self, free: Vec3) -> Vec3 {
        let from = self.going.map_or(free, |w| w.target(free));
        match self.striking {
            Some(striking) => from.lerp(striking.target(free), self.taking),
            None => from,
        }
    }
}

impl Want {
    fn target(&self, free: Vec3) -> Vec3 {
        let z = self.z + self.floor * (free.z - self.z).max(0.0);
        let z = free.z + (self.weight * (z - free.z)).clamp(-Z_PULL_MM.0, Z_PULL_MM.1);
        let (aim, weight) = self.aim;
        let xy = free.truncate() + weight * (aim - free.truncate()).clamp_length_max(AIM_PULL_MM);
        xy.extend(z)
    }
}

fn clear_of_keys(tip: Vec3, own: [Option<(u8, f32)>; 2]) -> f32 {
    static KEYBOARD: std::sync::OnceLock<Keyboard> = std::sync::OnceLock::new();
    let keyboard = KEYBOARD.get_or_init(Keyboard::new);
    if tip.y < 0.0 || tip.y > WHITE_KEY_LENGTH {
        return f32::MIN;
    }
    let behind = minimum_jerk(f64::from((tip.y - BLACK_KEY_FRONT_Y + KEY_EDGE_INWARD_MM) / KEY_EDGE_INWARD_MM)) as f32;
    let floor = KEY_CLEAR_MM + behind * BLACK_KEY_HEIGHT;
    let inside_own = own
        .iter()
        .flatten()
        .map(|(midi, weight)| {
            let (x0, x1, y0, y1) = keyboard.footprint(*midi);
            let inside = (tip.x - x0).min(x1 - tip.x).min(tip.y - y0).min(y1 - tip.y);
            weight * minimum_jerk(f64::from(inside / KEY_EDGE_INWARD_MM)) as f32
        })
        .fold(0.0f32, f32::max);
    floor - inside_own * (floor + 20.0)
}

fn above_the_keys(tip: Vec3, engaged: f32) -> Vec3 {
    let over = minimum_jerk(f64::from((tip.y + KEY_EDGE_MM) / KEY_EDGE_MM)) as f32;
    let black = minimum_jerk(f64::from((tip.y - BLACK_KEY_FRONT_Y + 2.0 * KEY_EDGE_MM) / (2.0 * KEY_EDGE_MM))) as f32;
    let clear = black * BLACK_KEY_HEIGHT + KEY_CLEAR_MM;
    let below = (clear - tip.z).max(0.0) * over * (1.0 - engaged);
    if below > 1e-3 { tip + Vec3::Z * below } else { tip }
}

fn stiffness(j: usize) -> f32 {
    match j {
        dof::THUMB_CMC_FLEX | dof::THUMB_CMC_ABD => 1.0,
        dof::THUMB_MCP_FLEX => 2.0,
        dof::THUMB_IP_FLEX => CURL_STIFFNESS,
        j if is_spread(j) => SPREAD_STIFFNESS,
        j if j >= dof::FINGER_BASE && (j - dof::FINGER_BASE) % 3 == dof::PIP_FLEX => CURL_STIFFNESS,
        _ => 1.0,
    }
}

fn is_knuckle(j: usize) -> bool {
    j >= dof::FINGER_BASE && (j - dof::FINGER_BASE) % 3 == dof::MCP_FLEX
}

fn is_spread(j: usize) -> bool {
    j == dof::THUMB_CMC_ABD || (j >= dof::FINGER_BASE && (j - dof::FINGER_BASE) % 3 == dof::MCP_SPREAD)
}

fn digit_dofs(finger: Finger) -> &'static [usize] {
    const THUMB: [usize; 4] = [dof::THUMB_CMC_FLEX, dof::THUMB_CMC_ABD, dof::THUMB_MCP_FLEX, dof::THUMB_IP_FLEX];
    const FINGERS: [[usize; 3]; 4] = {
        let mut out = [[0; 3]; 4];
        let mut slot = 0;
        while slot < 4 {
            let base = dof::finger(slot);
            out[slot] = [base + dof::MCP_FLEX, base + dof::MCP_SPREAD, base + dof::PIP_FLEX];
            slot += 1;
        }
        out
    };
    match finger {
        Finger::Thumb => &THUMB,
        other => &FINGERS[other.index() - 1],
    }
}

fn reach_step(skeleton: &Skeleton, posture: &Posture, pose: &mut HandPose, finger: Finger, target: Vec3) {
    let error = (target - posture.tip(finger)) * REACH_AXES;
    if error.length() < 0.1 {
        return;
    }
    let joints = digit_dofs(finger);
    let n = joints.len();
    let levers: Vec<f32> = joints
        .iter()
        .map(|j| {
            let column = skeleton.tip_jacobian(posture, finger, *j) * REACH_AXES;
            minimum_jerk(f64::from((column.length() - LEVER_MM.0) / LEVER_MM.1)) as f32
        })
        .collect();
    let columns: Vec<Vec3> =
        joints.iter().zip(&levers).map(|(j, lever)| skeleton.tip_jacobian(posture, finger, *j) * *lever).collect();
    let mut a = [[0.0f32; 5]; 4];
    for r in 0..n {
        for c in 0..n {
            a[r][c] = (columns[r] * REACH_AXES).dot(columns[c]) + if r == c { REACH_DAMPING * stiffness(joints[r]) } else { 0.0 };
        }
        a[r][n] = columns[r].dot(error);
    }
    for k in 0..n {
        let pivot = a[k][k];
        for r in k + 1..n {
            let f = a[r][k] / pivot;
            for c in k..=n {
                a[r][c] -= f * a[k][c];
            }
        }
    }
    let mut step = [0.0f32; 4];
    for k in (0..n).rev() {
        let known: f32 = (k + 1..n).map(|c| a[k][c] * step[c]).sum();
        step[k] = (a[k][n] - known) / a[k][k];
    }
    for (k, j) in joints.iter().enumerate() {
        pose.q[*j] += (levers[k] * step[k]).clamp(-REACH_STEP, REACH_STEP);
    }
}
