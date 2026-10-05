use glam::{Vec2, Vec3};
use on_fingering::biomech::{BiomechModel, Grip};
use on_fingering::playability::HAND_SPEED_MM_PER_SECOND;
use on_hand::keyboard::{is_black, BLACK_KEY_DIP, BLACK_KEY_FRONT_Y, BLACK_KEY_HEIGHT, KEY_DIP};
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

const LEAST_STRIKE: f64 = 0.02;

const REACH_DAMPING: f32 = 400.0;

const REACH_STEP: f32 = 0.35;

const REACH_PASSES: usize = 4;

const REACH_GIVE_MM: (f32, f32) = (6.0, 14.0);

const AIM_PULL_MM: f32 = 12.0;

const REACH_AXES: Vec3 = Vec3::new(1.0, 0.25, 1.0);

const TOGETHER: f64 = on_fingering::playability::CHORD_SECONDS;

const GROUP_SECONDS: f64 = 0.22;

const SINK_FALL: f64 = 0.06;

const SINK_RECOVER: (f64, f64) = (0.22, 0.5);

const SINK_MM: (f32, f32) = (1.6, 6.5);

const SINK_DEG: (f32, f32) = (3.5, 13.0);

const SINK_ROOM: (f64, f64) = (0.15, 0.3);

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

const IDLE_THUMB_CLEAR_MM: f32 = 8.0;

fn five_fingers(hand: Hand) -> Grip {
    Grip::new(match hand {
        Hand::Right => vec![(60, Finger::Thumb), (62, Finger::Index), (64, Finger::Middle), (65, Finger::Ring), (67, Finger::Little)],
        Hand::Left => vec![(48, Finger::Little), (50, Finger::Ring), (52, Finger::Middle), (53, Finger::Index), (55, Finger::Thumb)],
    })
}

pub fn relax_thumbs(events: &[GripEvent], poses: &mut [HandPose], model: &BiomechModel) {
    let skeleton = model.skeleton();
    let mut relaxed = model.grip_pose(&five_fingers(skeleton.hand()));
    let target = skeleton.forward(&relaxed).tip(Finger::Thumb) + IDLE_THUMB_LIFT;
    for _ in 0..12 {
        let posture = skeleton.forward(&relaxed);
        reach_step(skeleton, &posture, &mut relaxed, Finger::Thumb, target);
        skeleton.clamp(&mut relaxed);
    }
    let thumb = digit_dofs(Finger::Thumb);
    let clear = |pose: &HandPose| {
        let tip = skeleton.forward(pose).tip(Finger::Thumb);
        let surface = if tip.y > BLACK_KEY_FRONT_Y { BLACK_KEY_HEIGHT } else { 0.0 };
        tip.z >= surface + IDLE_THUMB_CLEAR_MM
    };
    for (event, pose) in events.iter().zip(poses.iter_mut()) {
        if event.grip.keys.iter().any(|(_, f)| *f == Finger::Thumb) {
            continue;
        }
        let original = *pose;
        let blended = |share: f32| {
            let mut out = original;
            for j in thumb {
                out.q[*j] += share * (relaxed.q[*j] - original.q[*j]);
            }
            out
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

pub fn sink(events: &[GripEvent], time: f64) -> (f32, f32) {
    let index = events.partition_point(|e| e.time <= time);
    if index == 0 {
        return (0.0, 0.0);
    }
    let mut start = index - 1;
    while start > 0 && events[start].time - events[start - 1].time < GROUP_SECONDS {
        start -= 1;
    }
    let mut end = start;
    while end + 1 < events.len() && events[end + 1].time - events[end].time < GROUP_SECONDS {
        end += 1;
    }
    let first = &events[start];
    let Some(hardest) = events[start..=end].iter().flat_map(|e| e.struck.iter().map(|(_, v)| *v)).max() else {
        return (0.0, 0.0);
    };
    let length = events[end].time - first.time;
    let recover = length.clamp(SINK_RECOVER.0, SINK_RECOVER.1);
    let next = events.get(end + 1).map_or(f64::INFINITY, |e| e.time);
    let scale = minimum_jerk((next - first.time - SINK_ROOM.0) / SINK_ROOM.1) as f32;
    let since = time - first.time;
    let shape = if since < SINK_FALL {
        minimum_jerk(since / SINK_FALL)
    } else {
        1.0 - minimum_jerk((since - SINK_FALL) / recover)
    };
    let until_next = ((next - time) / SINK_FALL).clamp(0.0, 1.0);
    (scale * shape as f32 * minimum_jerk(until_next) as f32, f32::from(hardest) / 127.0)
}

pub fn sink_depth(hardness: f32) -> (f32, f32) {
    (
        SINK_MM.0 + (SINK_MM.1 - SINK_MM.0) * hardness,
        (SINK_DEG.0 + (SINK_DEG.1 - SINK_DEG.0) * hardness).to_radians(),
    )
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
            by_finger[finger.index()].push(Strike { note, aim });
        }
        for list in &mut by_finger {
            list.sort_by(|a, b| a.note.key_moves.total_cmp(&b.note.key_moves));
            for i in 1..list.len() {
                let next = list[i].note.key_moves;
                let note = &mut list[i - 1].note;
                note.key_rises = note.key_rises.min(next - LET_GO_SECONDS - 0.02).max(note.key_moves);
            }
        }
        Self { by_finger, lift: 1.0 }
    }

    fn plan(&self, finger: Finger, time: f64) -> Option<Plan> {
        let list = &self.by_finger[finger.index()];
        let after = list.partition_point(|s| s.note.key_moves <= time);
        let mut going = None;
        if after > 0 {
            let Strike { note, aim } = &list[after - 1];
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
        let alone = going.map(|w| Plan { going: Some(w), striking: None, taking: 0.0 });
        let Some(Strike { note: next, aim }) = list.get(after) else { return alone };
        let free_from = if after > 0 { list[after - 1].note.key_rises + LET_GO_SECONDS } else { f64::MIN };
        let from = (next.key_moves - STRIKE_WINDOW).max(free_from).min(next.key_moves - LEAST_STRIKE);
        if time < from {
            return alone;
        }
        let window = (next.key_moves - from) as f32;
        let loud = f32::from(next.velocity) / 127.0;
        let height = ((STRIKE_LIFT_MM.0 + STRIKE_LIFT_MM.1 * loud) * self.lift).min(0.5 * window * FINGER_SPEED_MM);
        let contact = (1.5 * if is_black(next.midi) { BLACK_KEY_DIP } else { KEY_DIP }) as f64 / key_seconds(next.velocity);
        let peak = from + 0.55 * (next.key_moves - from);
        let (top, _) = surface(next.midi);
        let taking = minimum_jerk((time - from) / (TAKE_HOLD * (next.key_moves - from))) as f32;
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
        Some(Plan { going, striking: Some(striking), taking })
    }

    pub fn engaged(&self, finger: Finger, time: f64) -> f32 {
        self.plan(finger, time).map_or(0.0, |p| p.weight())
    }

    pub fn press(&self, skeleton: &Skeleton, pose: &mut HandPose, time: f64) {
        let plans: Vec<(Finger, Plan)> =
            Finger::ALL.iter().filter_map(|f| Some((*f, self.plan(*f, time)?))).collect();
        if plans.is_empty() {
            return;
        }
        let natural = skeleton.forward(pose);
        let targets: Vec<(Finger, Vec3)> = plans.iter().map(|(f, plan)| (*f, plan.target(natural.tip(*f)))).collect();
        let before = *pose;
        for _ in 0..REACH_PASSES {
            let posture = skeleton.forward(pose);
            for (finger, target) in &targets {
                reach_step(skeleton, &posture, pose, *finger, *target);
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
        let z = free.z + self.weight * (z - free.z);
        let (aim, weight) = self.aim;
        let xy = free.truncate() + weight * (aim - free.truncate()).clamp_length_max(AIM_PULL_MM);
        xy.extend(z)
    }
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
    let columns: Vec<Vec3> = joints.iter().map(|j| skeleton.tip_jacobian(posture, finger, *j)).collect();
    let mut a = [[0.0f32; 5]; 4];
    for r in 0..n {
        for c in 0..n {
            a[r][c] = (columns[r] * REACH_AXES).dot(columns[c]) + if r == c { REACH_DAMPING } else { 0.0 };
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
        pose.q[*j] += step[k].clamp(-REACH_STEP, REACH_STEP);
    }
}
