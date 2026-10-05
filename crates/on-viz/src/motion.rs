use on_fingering::biomech::BiomechModel;
use on_fingering::playability::HAND_SPEED_MM_PER_SECOND;
use on_hand::keyboard::{is_black, BLACK_KEY_DIP, BLACK_KEY_HEIGHT, KEY_DIP};
use on_hand::skeleton::{dof, HandPose, Posture, Skeleton, DOF};
use on_hand::Finger;

use crate::timeline::{key_seconds, GripEvent, TimelineNote};

const KEY_LEAD: f64 = 0.85;

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

const BEND_DAMPING: f32 = 400.0;

const BEND_STEP: f32 = 0.35;

const BEND_LEAST_RATE: f32 = -10.0;

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

impl Track {
    pub fn new(events: &[GripEvent], poses: &[HandPose]) -> Self {
        let mut stays: Vec<Stay> = Vec::with_capacity(events.len());
        let mut last = f64::MIN;
        for event in events {
            let arrive = arrival(event).max(last + MIN_KNOT_GAP);
            stays.push(Stay { arrive, leave: arrive });
            last = arrive;
        }
        for i in 0..stays.len().saturating_sub(1) {
            let free = stays[i + 1].arrive - move_seconds(&poses[i], &poses[i + 1]);
            stays[i].leave = free.min(stays[i + 1].arrive - MIN_KNOT_GAP).max(stays[i].arrive);
        }
        if let (Some(stay), Some(event)) = (stays.last_mut(), events.last()) {
            stay.leave = stay.arrive.max(event.release);
        }

        let mut knots = Vec::with_capacity(2 * events.len());
        for (i, stay) in stays.iter().enumerate() {
            let held = stay.leave > stay.arrive + 1e-3 || i == 0 || i + 1 == stays.len();
            knots.push(Knot { time: stay.arrive, pose: poses[i], still: held, event: i });
            if stay.leave > stay.arrive + 1e-3 {
                knots.push(Knot { time: stay.leave, pose: poses[i], still: true, event: i });
            }
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

pub struct Strikes {
    by_finger: [Vec<TimelineNote>; 5],
    lift: f32,
}

impl Strikes {
    pub fn new(notes: &[TimelineNote], events: &[GripEvent], stays: &[Stay]) -> Self {
        let mut by_finger: [Vec<TimelineNote>; 5] = Default::default();
        for note in notes {
            let Some(finger) = note.finger else { continue };
            let mut note = note.clone();
            let first = events.partition_point(|e| e.time < note.start - TOGETHER);
            let held = events[first..]
                .iter()
                .enumerate()
                .take_while(|(_, e)| e.grip.keys.contains(&(note.midi, finger)))
                .last()
                .map(|(k, _)| first + k);
            if let Some(stay) = held.and_then(|j| stays.get(j)) {
                note.key_rises = note.key_rises.min(stay.leave).max(note.key_moves);
            }
            by_finger[finger.index()].push(note);
        }
        for list in &mut by_finger {
            list.sort_by(|a, b| a.key_moves.total_cmp(&b.key_moves));
            for i in 1..list.len() {
                let next = list[i].key_moves;
                let note = &mut list[i - 1];
                note.key_rises = note.key_rises.min(next - LET_GO_SECONDS - 0.02).max(note.key_moves);
            }
        }
        Self { by_finger, lift: 1.0 }
    }

    fn height(&self, finger: Finger, time: f64) -> Option<(f32, f32, f32)> {
        let list = &self.by_finger[finger.index()];
        let after = list.partition_point(|n| n.key_moves <= time);
        let mut letting_go = None;
        if after > 0 {
            let note = &list[after - 1];
            let (top, dip) = surface(note.midi);
            let up = note.key_rises + LET_GO_SECONDS;
            if time <= up {
                return Some((top - note.key_depth(time) as f32 * dip, 1.0, 0.0));
            }
            if time < up + FADE_SECONDS {
                let u = (time - up) / FADE_SECONDS;
                letting_go = Some((top, 1.0 - minimum_jerk(u) as f32, minimum_jerk(u) as f32));
            }
        }
        let Some(next) = list.get(after) else { return letting_go };
        let free_from = if after > 0 { list[after - 1].key_rises + LET_GO_SECONDS } else { f64::MIN };
        let from = (next.key_moves - STRIKE_WINDOW).max(free_from).min(next.key_moves - 0.005);
        if time < from {
            return letting_go;
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
        let striking = (top + raised as f32, taking, floor);
        Some(match letting_go {
            Some((z, w, f)) => (z + taking * (striking.0 - z), w.max(taking), f + taking * (floor - f)),
            _ => striking,
        })
    }

    pub fn press(&self, skeleton: &Skeleton, pose: &mut HandPose, time: f64) {
        let wanted: Vec<(Finger, (f32, f32, f32))> =
            Finger::ALL.iter().filter_map(|f| Some((*f, self.height(*f, time)?))).collect();
        if wanted.is_empty() {
            return;
        }
        let natural = skeleton.forward(pose);
        let wanted: Vec<(Finger, f32)> = wanted
            .into_iter()
            .map(|(f, (z, w, floor))| {
                let free = natural.tip(f).z;
                let z = z + floor * (free - z).max(0.0);
                (f, free + w * (z - free))
            })
            .collect();
        for _ in 0..3 {
            let posture = skeleton.forward(pose);
            for (finger, z) in &wanted {
                bend_to(skeleton, &posture, pose, *finger, *z);
            }
            skeleton.clamp(pose);
        }
    }
}

fn bend_to(skeleton: &Skeleton, posture: &Posture, pose: &mut HandPose, finger: Finger, z: f32) {
    let (main, follow) = match finger {
        Finger::Thumb => (dof::THUMB_MCP_FLEX, dof::THUMB_CMC_FLEX),
        other => {
            let base = dof::finger(other.index() - 1);
            (base + dof::MCP_FLEX, base + dof::PIP_FLEX)
        }
    };
    let rate = skeleton.tip_jacobian(posture, finger, main).z + 0.5 * skeleton.tip_jacobian(posture, finger, follow).z;
    let rate = if finger == Finger::Thumb { rate } else { rate.min(BEND_LEAST_RATE) };
    let error = z - posture.tip(finger).z;
    let change = (error * rate / (rate * rate + BEND_DAMPING)).clamp(-BEND_STEP, BEND_STEP);
    pose.q[main] += change;
    pose.q[follow] += 0.5 * change;
}
