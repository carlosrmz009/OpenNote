use std::collections::BTreeMap;

use on_fingering::biomech::{BiomechModel, Grip};
use on_hand::keyboard::{is_black, KEY_DIP, MIDI_HIGHEST, MIDI_LOWEST};
use on_hand::skeleton::{dof, HandPose};
use on_hand::{Finger, Hand, HandProfile};
use on_score::{Fingering, NoteId, Score};

pub const APPROACH_SECONDS: f64 = 0.28;

const LIFT_RATE_MM: f32 = 120.0;
const LIFT_MAX_MM: f32 = 28.0;

const KEY_SECONDS: (f64, f64) = (0.110, 0.022);

const KEY_BEFORE_SOUND: f64 = 0.85;

const KEY_AFTER_SOUND: f64 = 0.15;

const KEY_CONTACT_SPEED: f64 = 1.5;

pub fn key_seconds(velocity: u8) -> f64 {
    let loud = ((f64::from(velocity) - 20.0) / 107.0).clamp(0.0, 1.0);
    KEY_SECONDS.0 * (KEY_SECONDS.1 / KEY_SECONDS.0).powf(loud)
}

const KEY_RELEASE_SECONDS: f64 = 0.06;

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineNote {
    pub id: NoteId,
    pub midi: u8,
    pub start: f64,
    pub end: f64,
    pub damped: f64,
    pub hand: Hand,
    pub finger: Option<Finger>,
    pub velocity: u8,
    pub restruck: Option<f64>,
    pub key_moves: f64,
    pub key_rises: f64,
}

impl TimelineNote {
    pub fn sounds_at(&self, time: f64) -> bool {
        self.start <= time && time < self.end
    }

    pub fn is_black(&self) -> bool {
        is_black(self.midi)
    }

    pub fn key_bottoms(&self) -> f64 {
        self.start + KEY_AFTER_SOUND * key_seconds(self.velocity)
    }

    pub fn key_depth(&self, time: f64) -> f64 {
        let going_down = |at: f64| {
            let (from, bottom) = (self.key_moves, self.key_bottoms());
            if at <= from {
                return 0.0;
            }
            if at >= bottom {
                return 1.0;
            }
            let span = bottom - from;
            let s = (at - from) / span;
            let push = KEY_CONTACT_SPEED * span / key_seconds(self.velocity);
            push * (s * s * s - 2.0 * s * s + s) + (3.0 * s * s - 2.0 * s * s * s)
        };
        if time < self.key_rises {
            return going_down(time);
        }
        let u = ((time - self.key_rises) / KEY_RELEASE_SECONDS).min(1.0);
        going_down(self.key_rises) * (1.0 - u * u * (3.0 - 2.0 * u))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GripEvent {
    pub time: f64,
    pub release: f64,
    pub grip: Grip,
    pub struck: Vec<(Finger, u8)>,
}

#[derive(Debug, Clone)]
pub struct Timeline {
    pub notes: Vec<TimelineNote>,
    pub grips: [Vec<GripEvent>; 2],
    pub duration: f64,
    pub title: Option<String>,
    pub style: MotionStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MotionStyle {
    pub lift: f32,
    pub weight: f32,
    pub phrase_ends: Vec<f64>,
}

impl Default for MotionStyle {
    fn default() -> Self {
        Self { lift: 1.0, weight: 1.0, phrase_ends: Vec::new() }
    }
}

const BREATH_LIFT_MM: (f32, f32) = (10.0, 15.0);

const BREATH_DEG: f32 = 8.0;

const BREATH_NEAR_SECONDS: f64 = 0.08;

const REST_LIFT_MM: f32 = 40.0;

const REST_CLEAR_MM: f32 = 140.0;

const REST_MARGIN_SECONDS: f64 = 0.5;

impl Timeline {
    pub fn audio_notes(&self) -> Vec<on_audio::Note> {
        const RELEASE_GAP: f64 = 0.012;
        self.notes
            .iter()
            .map(|note| on_audio::Note {
                midi: note.midi,
                start: note.start,
                end: if note.damped > note.end + 1e-6 {
                    note.damped
                } else {
                    (note.end - RELEASE_GAP).max(note.start + 0.02)
                },
                velocity: note.velocity,
            })
            .collect()
    }

    pub fn build(score: &Score, fingerings: &[Fingering]) -> Self {
        Self::build_for(score, fingerings, &HandProfile::default())
    }

    pub fn build_for(score: &Score, fingerings: &[Fingering], profile: &HandProfile) -> Self {
        let by_note: BTreeMap<NoteId, Finger> =
            fingerings.iter().map(|f| (f.note, f.finger)).collect();

        let mut notes: Vec<TimelineNote> = score
            .notes
            .iter()
            .filter_map(|n| {
                Some(TimelineNote {
                    id: n.id,
                    midi: n.midi,
                    start: n.onset_seconds,
                    end: score.release_seconds(n.id),
                    damped: score.damped_seconds(n.id),
                    hand: n.hand?,
                    finger: by_note.get(&n.id).copied(),
                    velocity: n.velocity,
                    restruck: None,
                    key_moves: n.onset_seconds,
                    key_rises: score.release_seconds(n.id),
                })
            })
            .collect();
        notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.midi.cmp(&b.midi)));

        let spans = on_fingering::SpanModel::for_hand_size(profile.size()).table();
        let grips = Hand::ALL.map(|hand| {
            let model = BiomechModel::new(profile.clone(), hand, Default::default());
            Self::grips_for(&notes, hand, spans, &model)
        });
        roll_in_time(&mut notes, &grips);
        notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.midi.cmp(&b.midi)));
        let mut next_on_key: BTreeMap<u8, f64> = BTreeMap::new();
        for note in notes.iter_mut().rev() {
            note.restruck = next_on_key.insert(note.midi, note.start);
        }
        let mut last_on_key: BTreeMap<u8, usize> = BTreeMap::new();
        for i in 0..notes.len() {
            let key = key_seconds(notes[i].velocity);
            let earliest = notes[i].start - KEY_BEFORE_SOUND * key;
            let mut after_last = f64::MIN;
            if let Some(&j) = last_on_key.get(&notes[i].midi) {
                let up = (earliest - 0.6 * KEY_RELEASE_SECONDS).min(notes[j].key_rises);
                notes[j].key_rises = up.max(notes[j].key_bottoms());
                after_last = notes[j].key_rises + 0.6 * KEY_RELEASE_SECONDS;
            }
            notes[i].key_moves = earliest.max(after_last).min(notes[i].start - KEY_AFTER_SOUND * key);
            last_on_key.insert(notes[i].midi, i);
        }

        let duration = notes.iter().map(|n| n.end).fold(0.0, f64::max);

        Self { notes, grips, duration, title: score.title.clone(), style: MotionStyle::default() }
    }

    pub fn animators(
        &self,
        profile: &HandProfile,
        weights: on_fingering::biomech::BiomechWeights,
    ) -> Vec<HandAnimator> {
        let mut animators: Vec<HandAnimator> = Hand::ALL
            .iter()
            .map(|hand| {
                let model = BiomechModel::new(profile.clone(), *hand, weights);
                if smooth_motion() {
                    HandAnimator::moving(*hand, model, self.hand_grips(*hand).to_vec(), &self.notes, &self.style)
                } else {
                    HandAnimator::new(*hand, model, self.hand_grips(*hand).to_vec())
                }
            })
            .collect();

        let keyboard = on_hand::keyboard::Keyboard::new();
        let lowest = self.notes.iter().map(|n| n.midi).min().unwrap_or(60);
        let highest = self.notes.iter().map(|n| n.midi).max().unwrap_or(60);
        let (left_edge, right_edge) = (keyboard.centre_x(lowest), keyboard.centre_x(highest));
        for animator in &mut animators {
            animator.park_at(match animator.hand() {
                Hand::Left => left_edge - IDLE_CLEARANCE_MM,
                Hand::Right => right_edge + IDLE_CLEARANCE_MM,
            });
        }
        animators
    }

    fn grips_for(
        notes: &[TimelineNote],
        hand: Hand,
        spans: &on_fingering::SpanTable,
        model: &BiomechModel,
    ) -> Vec<GripEvent> {
        let mut mine: Vec<(&TimelineNote, Finger)> = notes
            .iter()
            .filter(|note| note.hand == hand)
            .filter_map(|note| Some((note, note.finger?)))
            .collect();
        mine.sort_by(|a, b| a.0.start.total_cmp(&b.0.start));

        let mut events: Vec<GripEvent> = Vec::new();
        let mut held: Vec<(&TimelineNote, Finger)> = Vec::new();
        let mut index = 0;

        while index < mine.len() {
            let time = mine[index].0.start;
            held.retain(|(note, _)| note.end > time + 1e-6);

            while index < mine.len() && mine[index].0.start <= time + TOGETHER_SECONDS {
                let (note, finger) = mine[index];
                held.retain(|(_, other)| *other != finger);
                held.push((note, finger));
                index += 1;
            }

            let holdable = |held: &[(&TimelineNote, Finger)]| {
                held.iter().enumerate().all(|(i, (low, from))| {
                    held[i + 1..].iter().all(|(high, to)| {
                        let apart = i32::from(high.midi) - i32::from(low.midi);
                        spans.get(hand, *from, *to).is_practical(apart)
                    })
                })
            };

            let drop_one = |held: &mut Vec<(&TimelineNote, Finger)>| {
                if held.len() < 2 {
                    return false;
                }
                let Some(at) = held
                    .iter()
                    .enumerate()
                    .filter(|(_, (note, _))| note.start < time - 1e-6)
                    .min_by(|a, b| a.1 .0.start.total_cmp(&b.1 .0.start))
                    .map(|(at, _)| at)
                else {
                    return false;
                };
                held.remove(at);
                true
            };

            held.sort_by_key(|(note, _)| note.midi);

            let makeable = |held: &[(&TimelineNote, Finger)]| {
                holdable(held) && {
                    let shape = Grip::new(
                        held.iter().map(|(note, finger)| (note.midi, *finger)).collect(),
                    );
                    model.grip_outcome(&shape).reachable
                }
            };

            while held.len() > 1 && !makeable(&held) && drop_one(&mut held) {}

            let mut rolls: Vec<Vec<(&TimelineNote, Finger)>> = Vec::new();
            while held.len() > 1 && !makeable(&held) {
                let lowest = held.remove(0);
                match rolls.last_mut() {
                    Some(group) => {
                        group.push(lowest);
                        if !makeable(group) {
                            let overflow = group.pop().expect("just pushed");
                            rolls.push(vec![overflow]);
                        }
                    }
                    None => rolls.push(vec![lowest]),
                }
            }

            let step = mine.get(index).map_or(ROLL_SECONDS, |(next, _)| {
                ROLL_SECONDS.min((next.start - time) / (rolls.len() + 1) as f64)
            });
            let mut grip_time = time;
            for group in &rolls {
                events.push(grip_event(group, grip_time, time));
                grip_time += step;
            }

            let struck: Vec<(Finger, u8)> = held
                .iter()
                .filter(|(note, _)| together(note.start, time))
                .map(|(note, finger)| (*finger, note.velocity))
                .collect();
            let keys: Vec<(u8, Finger)> = held
                .iter()
                .map(|(note, finger)| (note.midi, *finger))
                .collect();
            let release = held.iter().fold(grip_time, |latest, (note, _)| latest.max(note.end));
            let mut grip = Grip::new(keys);
            grip.keys.sort_by_key(|(midi, _)| *midi);
            events.push(GripEvent { time: grip_time, release, grip, struck });
        }
        events
    }

    pub fn hand_grips(&self, hand: Hand) -> &[GripEvent] {
        &self.grips[hand as usize]
    }

    pub fn key_depression(&self, time: f64) -> KeyStates {
        let mut states = KeyStates::default();
        for note in &self.notes {
            if time < note.key_moves || time > note.key_rises + KEY_RELEASE_SECONDS {
                continue;
            }
            let depth = note.key_depth(time);
            let slot = states.slot(note.midi);
            if depth as f32 >= states.depth[slot] {
                states.depth[slot] = depth as f32;
                if depth > 0.0 {
                    states.hand[slot] = Some(note.hand);
                }
            }
        }
        states
    }

    pub fn visible_notes(&self, time: f64, lookahead: f64) -> impl Iterator<Item = &TimelineNote> {
        self.notes
            .iter()
            .filter(move |n| n.end > time - 0.5 && n.start < time + lookahead)
    }
}

#[derive(Debug, Clone)]
pub struct KeyStates {
    pub depth: Vec<f32>,
    pub hand: Vec<Option<Hand>>,
}

impl Default for KeyStates {
    fn default() -> Self {
        let count = (MIDI_HIGHEST - MIDI_LOWEST + 1) as usize;
        Self { depth: vec![0.0; count], hand: vec![None; count] }
    }
}

impl KeyStates {
    fn slot(&self, midi: u8) -> usize {
        (midi.clamp(MIDI_LOWEST, MIDI_HIGHEST) - MIDI_LOWEST) as usize
    }

    pub fn depth_of(&self, midi: u8) -> f32 {
        self.depth[self.slot(midi)]
    }

    pub fn travel_of(&self, midi: u8) -> f32 {
        self.depth_of(midi) * KEY_DIP
    }

    pub fn hand_on(&self, midi: u8) -> Option<Hand> {
        self.hand[self.slot(midi)]
    }

    pub fn pressed(&self) -> impl Iterator<Item = u8> + '_ {
        (MIDI_LOWEST..=MIDI_HIGHEST).filter(move |m| self.depth_of(*m) > 0.05)
    }
}

const ROLL_SECONDS: f64 = 0.05;

const HOLD_RAMP_SECONDS: f64 = 0.15;

const TOGETHER_SECONDS: f64 = on_fingering::playability::CHORD_SECONDS;

fn together(start: f64, onset: f64) -> bool {
    start >= onset - 1e-6 && start <= onset + TOGETHER_SECONDS
}

const CONTINUITY: [f32; on_hand::skeleton::DOF] = {
    let mut weights = [0.3f32; on_hand::skeleton::DOF];
    weights[dof::WRIST_X] = 0.0;
    weights[dof::WRIST_Y] = 0.0;
    weights[dof::WRIST_Z] = 0.0;
    weights[dof::WRIST_DEVIATION] = 0.8;
    weights[dof::WRIST_FLEXION] = 0.8;
    weights[dof::WRIST_PRONATION] = 0.8;
    weights
};

const ALIKE_RADIANS: f32 = 0.15;

fn alike(a: &HandPose, b: &HandPose) -> bool {
    let angles = dof::WRIST_DEVIATION..on_hand::skeleton::DOF;
    let n = angles.len() as f32;
    let sum: f32 = angles.map(|i| (a.q[i] - b.q[i]).powi(2)).sum();
    (sum / n).sqrt() < ALIKE_RADIANS
}

fn roll_in_time(notes: &mut [TimelineNote], grips: &[Vec<GripEvent>; 2]) {
    for (side, events) in grips.iter().enumerate() {
        for event in events {
            for (finger, _) in &event.struck {
                let Some((midi, _)) = event.grip.keys.iter().find(|(_, f)| f == finger) else { continue };
                let on_time = notes.iter().any(|n| {
                    n.hand as usize == side && n.midi == *midi && (n.start - event.time).abs() < 1e-6
                });
                if on_time {
                    continue;
                }
                let rolled = notes.iter_mut().filter(|n| {
                    n.hand as usize == side
                        && n.midi == *midi
                        && n.finger == Some(*finger)
                        && n.start < event.time - 1e-6
                        && n.start > event.time - 4.0 * ROLL_SECONDS - 1e-6
                });
                if let Some(note) = rolled.max_by(|a, b| a.start.total_cmp(&b.start)) {
                    note.start = event.time;
                    note.end = note.end.max(event.time + 0.02);
                    note.damped = note.damped.max(note.end);
                }
            }
        }
    }
}

fn grip_event(
    notes: &[(&TimelineNote, Finger)],
    time: f64,
    struck_at: f64,
) -> GripEvent {
    let struck: Vec<(Finger, u8)> = notes
        .iter()
        .filter(|(note, _)| together(note.start, struck_at))
        .map(|(note, finger)| (*finger, note.velocity))
        .collect();
    let mut grip = Grip::new(notes.iter().map(|(note, finger)| (note.midi, *finger)).collect());
    grip.keys.sort_by_key(|(midi, _)| *midi);
    let release = notes.iter().fold(time, |latest, (note, _)| latest.max(note.end));
    GripEvent { time, release, grip, struck }
}

const PARK_Y_MM: f32 = -80.0;
const PARK_Z_MM: f32 = 60.0;

pub const IDLE_CLEARANCE_MM: f32 = 170.0;

pub const CLEARANCE_SLACK_MM: f32 = 3.0;

pub const HAND_THICKNESS_MM: f32 = 20.0;

const SWING_LEAD_SECONDS: f64 = 0.08;

const DECIDE_EVERY_SECONDS: f64 = 1.0 / 30.0;

const EASE_SECONDS: f64 = 0.2;

const ENGAGE_MM: f32 = 30.0;

const CLEARANCE_MAX_MM: f32 = 80.0;

const SWING_MAX_DEG: f32 = 30.0;

const SWING_TOLERANCE_MM: f32 = 6.0;

const SWING_SLIDE_MM: f32 = 20.0;

const STRIKE_LIFT_SECONDS: f64 = 0.16;

const STRIKE_LIFT_DEG: (f32, f32) = (2.5, 9.0);

const WRIST_DROP_MM: (f32, f32) = (1.6, 6.5);

const WRIST_DROP_DEG: (f32, f32) = (3.5, 13.0);

const WRIST_FALL_SECONDS: f64 = 0.045;
const WRIST_SETTLE_SECONDS: f64 = 0.22;

const BREATH_MM: f32 = 1.4;
const BREATH_HZ: f64 = 0.23;

const BREATH_EASE_SECONDS: f64 = 0.35;

const APPROACH_BY_FORCE: f32 = 0.6;

const STRIKE_PEAK: (f32, f32) = (0.5, 0.72);

pub struct HandAnimator {
    hand: Hand,
    skeleton: on_hand::Skeleton,
    events: Vec<GripEvent>,
    poses: Vec<HandPose>,
    resting: HandPose,
    motion: Option<Motion>,
}

struct Motion {
    track: crate::motion::Track,
    strikes: crate::motion::Strikes,
    lag: crate::motion::Lag,
    style: MotionStyle,
}

pub fn smooth_motion() -> bool {
    std::env::var("ON_MOTION").map_or(true, |v| v != "v1")
}

impl HandAnimator {
    pub fn new(hand: Hand, model: BiomechModel, events: Vec<GripEvent>) -> Self {
        let mut poses: Vec<HandPose> = Vec::with_capacity(events.len());
        for event in &events {
            let cold = model.grip_pose(&event.grip);
            let pose = match poses.last() {
                Some(previous) if !alike(previous, &cold) => {
                    let (warm, error) = model.grip_pose_near(&event.grip, previous, CONTINUITY);
                    if error > on_hand::ik::CONTACT_TOLERANCE_MM && model.grip_outcome(&event.grip).reachable {
                        cold
                    } else {
                        warm
                    }
                }
                _ => cold,
            };
            poses.push(pose);
        }

        let resting = poses.first().copied().unwrap_or_else(|| {
            let centre = model.keyboard().centre_x(60);
            model
                .skeleton()
                .rest_pose(glam::Vec3::new(centre, PARK_Y_MM, PARK_Z_MM))
        });

        let skeleton = model.skeleton().clone();
        Self { hand, skeleton, events, poses, resting, motion: None }
    }

    pub fn moving(
        hand: Hand,
        model: BiomechModel,
        events: Vec<GripEvent>,
        notes: &[TimelineNote],
        style: &MotionStyle,
    ) -> Self {
        let twin = BiomechModel::new(model.skeleton().profile().clone(), hand, *model.weights());
        let mut animator = Self::new(hand, twin, events);
        if animator.events.is_empty() {
            return animator;
        }
        animator.poses = crate::motion::smooth_wrists(&animator.events, &animator.poses, &model);
        crate::motion::relax_thumbs(&animator.events, &mut animator.poses, &model);
        if let Some(first) = animator.poses.first() {
            animator.resting = *first;
        }
        let mine: Vec<TimelineNote> = notes.iter().filter(|n| n.hand == hand).cloned().collect();
        let keyboard = model.keyboard().clone();
        let rest = |from: f64, until: f64, pose: &HandPose| {
            let mut rest = *pose;
            rest.q[dof::WRIST_Z] += REST_LIFT_MM;
            let other = notes
                .iter()
                .filter(|n| n.hand != hand && n.end > from - REST_MARGIN_SECONDS && n.start < until + REST_MARGIN_SECONDS)
                .map(|n| keyboard.centre_x(n.midi));
            let (lo, hi) = other.fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
            if lo <= hi {
                let x = &mut rest.q[dof::WRIST_X];
                *x = match hand {
                    Hand::Right => x.max(hi + REST_CLEAR_MM).min(keyboard.width()),
                    Hand::Left => x.min(lo - REST_CLEAR_MM).max(0.0),
                };
            }
            rest
        };
        let track = crate::motion::Track::new(&animator.events, &animator.poses, &rest);
        let mut strikes =
            crate::motion::Strikes::new(&mine, &animator.events, &track.stays, &animator.poses, &animator.skeleton);
        strikes.lift = style.lift;
        let (from, until) = track.span();
        let (from, until) = (from - 1.0, until + 2.0);
        let lag = crate::motion::Lag::new(&track, from, until);
        animator.motion = Some(Motion { track, strikes, lag, style: style.clone() });
        animator
    }

    pub fn hand(&self) -> Hand {
        self.hand
    }

    pub fn park_at(&mut self, centre_x: f32) {
        if !self.events.is_empty() {
            return;
        }
        self.resting = self
            .skeleton
            .rest_pose(glam::Vec3::new(centre_x, PARK_Y_MM, PARK_Z_MM));
    }

    pub fn skeleton(&self) -> &on_hand::Skeleton {
        &self.skeleton
    }

    fn current_index(&self, time: f64) -> Option<usize> {
        if self.events.is_empty() || time < self.events[0].time {
            return None;
        }
        let index = self
            .events
            .partition_point(|e| e.time <= time)
            .saturating_sub(1);
        Some(index)
    }

    pub fn pose_at(&self, time: f64) -> HandPose {
        if let Some(motion) = &self.motion {
            return self.flowing(motion, time);
        }
        let Some(index) = self.current_index(time) else {
            return self.resting;
        };
        let mut pose = self.carried(index, time);
        pose.q[dof::WRIST_Z] += self.breath(index, time);
        self.sink_into_note(&mut pose, index, time);
        self.raise_fingers_about_to_strike(&mut pose, index, time);
        self.skeleton.clamp(&mut pose);
        pose
    }

    fn flowing(&self, motion: &Motion, time: f64) -> HandPose {
        if self.events.is_empty() {
            return self.resting;
        }
        let mut pose = motion.track.at(time);
        let lag = motion.lag.at(time);
        for (k, offset) in lag.iter().enumerate() {
            let index = dof::THUMB_CMC_FLEX + k;
            let idle = 1.0 - motion.strikes.engaged(crate::motion::digit_of(index), time);
            pose.q[index] += offset * idle;
        }
        let stays = &motion.track.stays;
        let index = stays.partition_point(|s| s.arrive <= time).saturating_sub(1);
        if let (Some(current), Some(next)) = (self.events.get(index), stays.get(index + 1)) {
            let gap = next.arrive - current.release;
            if gap > 1e-3 && gap < crate::motion::REST_GAP_SECONDS && time > current.release && time < next.arrive {
                let hardest = self.events[index + 1].struck.iter().map(|(_, v)| *v).max().unwrap_or(64);
                let force = 1.0 + APPROACH_BY_FORCE * (f32::from(hardest) / 127.0 - 0.5) * 2.0;
                let room = crate::motion::hop_room(gap);
                let mut height = (gap as f32 * LIFT_RATE_MM * force).min(LIFT_MAX_MM) * room;
                let shape = crate::motion::bump((time - current.release) / gap);
                let ends = &motion.style.phrase_ends;
                let at = ends.partition_point(|e| *e < current.release - BREATH_NEAR_SECONDS);
                if ends.get(at).is_some_and(|e| *e <= current.release + BREATH_NEAR_SECONDS) {
                    let breath = (BREATH_LIFT_MM.0 + BREATH_LIFT_MM.1 * (motion.style.lift - 1.0).max(0.0)) * room;
                    height = height.max(breath * motion.style.lift.min(1.5));
                    pose.q[dof::WRIST_FLEXION] += BREATH_DEG.to_radians() * motion.style.lift * room * shape;
                }
                pose.q[dof::WRIST_Z] += height * shape;
            }
            pose.q[dof::WRIST_Z] += self.breath(index, time);
        }
        let (weight, hardness) = crate::motion::sink(&self.events, time);
        if weight > 0.0 {
            let (depth, angle) = crate::motion::sink_depth(hardness);
            let weight = weight * motion.style.weight;
            pose.q[dof::WRIST_Z] -= depth * weight;
            pose.q[dof::WRIST_FLEXION] -= angle * weight;
        }
        self.skeleton.clamp(&mut pose);
        motion.strikes.press(&self.skeleton, &mut pose, time);
        pose
    }

    fn carried(&self, index: usize, time: f64) -> HandPose {
        let current = &self.events[index];
        let pose = self.poses[index];

        let Some(next) = self.events.get(index + 1) else {
            return pose;
        };
        let next_pose = self.poses[index + 1];

        let start = self.departure(index);
        if time <= start {
            return pose;
        }
        let span = (next.time - start).max(1e-4);
        let t = ((time - start) / span).clamp(0.0, 1.0);

        let resting_before = start > current.time + 1e-4;
        let resting_after = self
            .events
            .get(index + 2)
            .is_none_or(|_| self.departure(index + 1) > next.time + 1e-4);

        let mut moved = pose.lerp(&next_pose, travel(t, resting_before, resting_after) as f32);
        moved.q[dof::WRIST_Z] += lift_between(current, next, time);
        moved
    }

    fn sink_into_note(&self, pose: &mut HandPose, index: usize, time: f64) {
        let current = &self.events[index];
        let since = time - current.time;
        if since < 0.0 || since >= WRIST_SETTLE_SECONDS {
            return;
        }
        let Some(hardest) = current.struck.iter().map(|(_, v)| *v).max() else {
            return;
        };
        let hardness = f32::from(hardest) / 127.0;

        let shape = if since < WRIST_FALL_SECONDS {
            (since / WRIST_FALL_SECONDS) as f32
        } else {
            let back =
                ((since - WRIST_FALL_SECONDS) / (WRIST_SETTLE_SECONDS - WRIST_FALL_SECONDS)) as f32;
            1.0 - back * back * (3.0 - 2.0 * back)
        };
        let until = self.events.get(index + 1).map_or(f64::INFINITY, |next| next.time - time);
        let handover = ((until / WRIST_FALL_SECONDS) as f32).clamp(0.0, 1.0);
        let shape = shape.clamp(0.0, 1.0) * handover * handover * (3.0 - 2.0 * handover);

        let depth = WRIST_DROP_MM.0 + (WRIST_DROP_MM.1 - WRIST_DROP_MM.0) * hardness;
        let angle = WRIST_DROP_DEG.0 + (WRIST_DROP_DEG.1 - WRIST_DROP_DEG.0) * hardness;
        pose.q[dof::WRIST_Z] -= depth * shape;
        pose.q[dof::WRIST_FLEXION] -= angle.to_radians() * shape;
    }

    fn breath(&self, index: usize, time: f64) -> f32 {
        let current = &self.events[index];
        if time <= current.release {
            return 0.0;
        }
        let until = self.events.get(index + 1).map_or(f64::INFINITY, |next| next.time - time);
        let free = ((time - current.release).min(until) / BREATH_EASE_SECONDS).clamp(0.0, 1.0) as f32;
        let free = free * free * (3.0 - 2.0 * free);
        let offset = match self.hand {
            Hand::Right => 0.0,
            Hand::Left => 2.1,
        };
        let phase = (time * BREATH_HZ * std::f64::consts::TAU) as f32 + offset;
        free * BREATH_MM * phase.sin()
    }

    fn raise_fingers_about_to_strike(&self, pose: &mut HandPose, index: usize, time: f64) {
        let Some(next) = self.events.get(index + 1) else {
            return;
        };
        let current = &self.events[index];
        let window = (next.time - current.time).min(STRIKE_LIFT_SECONDS);
        if window <= 1e-4 || time < next.time - window || time >= next.time {
            return;
        }
        let through = ((time - (next.time - window)) / window) as f32;

        for (finger, velocity) in &next.struck {
            if !current.grip.keys.iter().any(|(_, f)| f == finger) {
                continue;
            }
            let hardness = f32::from(*velocity) / 127.0;
            let height = STRIKE_LIFT_DEG.0 + (STRIKE_LIFT_DEG.1 - STRIKE_LIFT_DEG.0) * hardness;
            let peak = STRIKE_PEAK.0 + (STRIKE_PEAK.1 - STRIKE_PEAK.0) * hardness;
            let shape = if through < peak {
                through / peak
            } else {
                (1.0 - through) / (1.0 - peak)
            };
            let lift = height.to_radians() * shape.clamp(0.0, 1.0);

            match finger {
                Finger::Thumb => {
                    pose.q[dof::THUMB_MCP_FLEX] -= lift;
                    pose.q[dof::THUMB_CMC_FLEX] -= lift * 0.5;
                }
                other => {
                    let base = dof::finger(other.index() - 1);
                    pose.q[base + dof::MCP_FLEX] -= lift;
                    pose.q[base + dof::PIP_FLEX] += lift * 0.5;
                }
            }
        }
        self.skeleton.clamp(pose);
    }

    fn departure(&self, index: usize) -> f64 {
        let current = &self.events[index];
        let Some(next) = self.events.get(index + 1) else {
            return current.time;
        };
        let latest = next.time - APPROACH_SECONDS;
        let start = current.release.min(next.time).max(current.time);
        start.min(latest.max(current.time))
    }

    pub fn joints(&self, pose: &HandPose) -> Vec<glam::Vec3> {
        let posture = self.skeleton.forward(pose);
        let mut out = Vec::with_capacity(21);
        out.push(posture.wrist);
        for digit in &posture.chain {
            out.extend_from_slice(digit);
        }
        out
    }

    pub fn grips_around(&self, time: f64) -> Option<(&Grip, Option<&Grip>, f32)> {
        if let Some(motion) = &self.motion {
            return match motion.track.travelling(time) {
                Some((a, b, blend)) if a != b => Some((&self.events[a].grip, Some(&self.events[b].grip), blend)),
                Some((a, _, _)) => Some((&self.events[a].grip, None, 0.0)),
                None => {
                    let last = if time <= motion.track.stays.first().map_or(0.0, |s| s.arrive) { 0 } else { self.events.len() - 1 };
                    self.events.get(last).map(|e| (&e.grip, None, 0.0))
                }
            };
        }
        let Some(index) = self.current_index(time) else {
            return self.events.first().map(|first| (&first.grip, None, 0.0));
        };
        let current = &self.events[index].grip;
        let Some(next) = self.events.get(index + 1) else {
            return Some((current, None, 0.0));
        };
        let start = self.departure(index);
        if time <= start {
            return Some((current, Some(&next.grip), 0.0));
        }
        let span = (next.time - start).max(1e-4);
        let t = ((time - start) / span).clamp(0.0, 1.0);
        let resting_before = start > self.events[index].time + 1e-4;
        let resting_after = self
            .events
            .get(index + 2)
            .is_none_or(|_| self.departure(index + 1) > next.time + 1e-4);
        Some((current, Some(&next.grip), travel(t, resting_before, resting_after) as f32))
    }

    pub fn holding(&self, time: f64, ramp: f64) -> f32 {
        if let Some(motion) = &self.motion {
            let stays = &motion.track.stays;
            let ramp = ramp.max(HOLD_RAMP_SECONDS);
            let first = stays.partition_point(|s| s.arrive <= time - ramp).saturating_sub(1);
            let last = stays.partition_point(|s| s.arrive < time + ramp);
            let mut held = 0.0f32;
            for k in first..last {
                if self.events[k].grip.keys.is_empty() {
                    continue;
                }
                let until = stays[k].leave.max(self.events[k].release.min(stays.get(k + 1).map_or(f64::MAX, |s| s.arrive)));
                let inside = (time - stays[k].arrive).min(until - time);
                held = held.max(crate::motion::minimum_jerk((inside + ramp) / ramp) as f32);
            }
            return held;
        }
        let index = self.current_index(time).unwrap_or(0);
        let mut held = 0.0f32;
        for event in &self.events[index.saturating_sub(1)..(index + 2).min(self.events.len())] {
            if event.grip.keys.is_empty() {
                continue;
            }
            let inside = (time - event.time).min(event.release - time);
            let u = ((inside + ramp) / ramp).clamp(0.0, 1.0) as f32;
            held = held.max(u * u * (3.0 - 2.0 * u));
        }
        held
    }

    fn lands_next(&self, time: f64) -> f64 {
        let after = self.events.partition_point(|e| e.time <= time);
        self.events[after..]
            .iter()
            .find(|e| !e.grip.keys.is_empty())
            .map_or(f64::INFINITY, |e| e.time)
    }

    pub fn grip_at(&self, time: f64) -> Option<&Grip> {
        let index = self.current_index(time)?;
        let event = &self.events[index];
        (time < event.release).then_some(&event.grip)
    }
}

fn travel(t: f64, resting_before: bool, resting_after: bool) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let m0 = if resting_before { 0.0 } else { 1.0 };
    let m1 = if resting_after { 0.0 } else { 1.0 };
    let (t2, t3) = (t * t, t * t * t);
    (-2.0 * t3 + 3.0 * t2) + m0 * (t3 - 2.0 * t2 + t) + m1 * (t3 - t2)
}

fn lift_between(current: &GripEvent, next: &GripEvent, time: f64) -> f32 {
    let gap = next.time - current.release;
    if gap <= 1e-3 || time <= current.release || time >= next.time {
        return 0.0;
    }
    let hardest = next.struck.iter().map(|(_, v)| *v).max().unwrap_or(64);
    let force = 1.0 + APPROACH_BY_FORCE * (f32::from(hardest) / 127.0 - 0.5) * 2.0;
    let height = (gap as f32 * LIFT_RATE_MM * force).min(LIFT_MAX_MM);
    let through = ((time - current.release) / gap).clamp(0.0, 1.0) as f32;
    height * (through * std::f32::consts::PI).sin()
}

pub struct Decisions(Vec<Apart>);

impl Decisions {
    pub fn new(animators: &[HandAnimator], until: f64) -> Self {
        let steps = (until.max(0.0) / DECIDE_EVERY_SECONDS).ceil() as usize + 1;
        Self((0..steps).map(|step| apart_at(animators, step as f64 * DECIDE_EVERY_SECONDS)).collect())
    }

    fn at(&self, animators: &[HandAnimator], time: f64) -> Apart {
        apart_around(time, |step| match self.0.get(step as usize) {
            Some(decided) => *decided,
            None => apart_at(animators, step as f64 * DECIDE_EVERY_SECONDS),
        })
    }
}

pub fn pose_both(animators: &[HandAnimator], time: f64) -> [HandPose; 2] {
    let apart = apart_around(time, |step| apart_at(animators, step as f64 * DECIDE_EVERY_SECONDS));
    posed_apart(animators, time, apart)
}

pub fn pose_both_decided(animators: &[HandAnimator], decisions: &Decisions, time: f64) -> [HandPose; 2] {
    posed_apart(animators, time, decisions.at(animators, time))
}

fn posed_apart(animators: &[HandAnimator], time: f64, apart: Apart) -> [HandPose; 2] {
    let mut poses = [
        animators[Hand::Left as usize].pose_at(time),
        animators[Hand::Right as usize].pose_at(time),
    ];
    for (side, animator) in animators.iter().enumerate().take(2) {
        let (swing, lift) = (apart.swing[side], apart.lift[side]);
        if swing == 0.0 && lift == 0.0 {
            continue;
        }
        let hold = animator.holding(time, SWING_LEAD_SECONDS);
        let over = (lift - apart.lift[1 - side]).max(0.0);
        let adjust = |pose: &mut HandPose, grip: &Grip| {
            swing_by(animator, pose, grip, swing);
            raise_by(animator, pose, grip, over * hold);
        };
        if let Some((current, next, blend)) = animator.grips_around(time) {
            let mut a = poses[side];
            adjust(&mut a, current);
            poses[side] = match next {
                Some(next) if blend > 0.0 => {
                    let mut b = poses[side];
                    adjust(&mut b, next);
                    a.lerp(&b, blend)
                }
                _ => a,
            };
        }
        poses[side].q[dof::WRIST_Z] += lift * (1.0 - hold);
    }
    poses
}

#[derive(Clone, Copy, Default)]
struct Apart {
    lift: [f32; 2],
    swing: [f32; 2],
}

fn apart_around(time: f64, decide: impl Fn(i64) -> Apart) -> Apart {
    let first = ((time - EASE_SECONDS) / DECIDE_EVERY_SECONDS).ceil().max(0.0) as i64;
    let last = ((time + EASE_SECONDS) / DECIDE_EVERY_SECONDS).floor() as i64;
    let mut lift = [0.0f32; 2];
    let mut swing = [[0.0f32; 2]; 2];
    for step in first..=last {
        let at = step as f64 * DECIDE_EVERY_SECONDS;
        let reach = reach((time - at).abs());
        if reach <= 0.0 {
            continue;
        }
        let decided = decide(step);
        for side in 0..2 {
            lift[side] = lift[side].max(reach * decided.lift[side]);
            let turn = decided.swing[side];
            swing[side][0] = swing[side][0].max(reach * turn.max(0.0));
            swing[side][1] = swing[side][1].max(reach * (-turn).max(0.0));
        }
    }
    Apart { lift, swing: [swing[0][0] - swing[0][1], swing[1][0] - swing[1][1]] }
}

fn reach(gap: f64) -> f32 {
    let u = ((EASE_SECONDS - gap) / (EASE_SECONDS - DECIDE_EVERY_SECONDS)).clamp(0.0, 1.0) as f32;
    u * u * (3.0 - 2.0 * u)
}

fn apart_at(animators: &[HandAnimator], time: f64) -> Apart {
    let mut apart = Apart::default();
    let poses = [
        animators[Hand::Left as usize].pose_at(time),
        animators[Hand::Right as usize].pose_at(time),
    ];
    let joints = [
        animators[Hand::Left as usize].joints(&poses[0]),
        animators[Hand::Right as usize].joints(&poses[1]),
    ];
    let near = ((ENGAGE_MM - nearest(&joints[0], &joints[1])) / (ENGAGE_MM - HAND_THICKNESS_MM))
        .clamp(0.0, 1.0);
    if near <= 0.0 {
        return apart;
    }

    let free = [
        animators[Hand::Left as usize].grip_at(time).is_none(),
        animators[Hand::Right as usize].grip_at(time).is_none(),
    ];
    let lands = [
        animators[Hand::Left as usize].lands_next(time),
        animators[Hand::Right as usize].lands_next(time),
    ];
    let up = match (free[0], free[1]) {
        (true, true) => Some(usize::from(lands[1] > lands[0])),
        (true, false) => Some(0),
        (false, true) => Some(1),
        (false, false) => None,
    };
    if let Some(up) = up {
        apart.lift[up] = lift_to_clear(&joints[up], &joints[1 - up]).min(CLEARANCE_MAX_MM);
    }

    let hold = [
        animators[Hand::Left as usize].holding(time, SWING_LEAD_SECONDS),
        animators[Hand::Right as usize].holding(time, SWING_LEAD_SECONDS),
    ];
    let strength = near * hold[0] * hold[1];
    if strength <= 0.0 {
        return apart;
    }
    let centre = |side: usize| {
        let (lo, hi) = joints[side]
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), j| (a.min(j.x), b.max(j.x)));
        (lo + hi) / 2.0
    };
    for side in 0..2 {
        let animator = &animators[side];
        let Some((current, next, blend)) = animator.grips_around(time) else {
            continue;
        };
        let grip = match next {
            Some(next) if blend >= 0.5 => next,
            _ => current,
        };
        apart.swing[side] = swing_angle(animator, &poses[side], grip, centre(1 - side), strength);
    }
    apart
}

pub fn nearest(a: &[glam::Vec3], b: &[glam::Vec3]) -> f32 {
    a.iter()
        .flat_map(|p| b.iter().map(move |q| p.distance(*q)))
        .fold(f32::MAX, f32::min)
}

pub fn overlapping(a: &[glam::Vec3], b: &[glam::Vec3]) -> bool {
    a.iter()
        .any(|p| b.iter().any(|q| p.distance_squared(*q) < HAND_THICKNESS_MM * HAND_THICKNESS_MM))
}

pub fn overlap_depth(a: &[glam::Vec3], b: &[glam::Vec3]) -> f32 {
    (HAND_THICKNESS_MM - nearest(a, b)).max(0.0)
}

fn lift_to_clear(up: &[glam::Vec3], down: &[glam::Vec3]) -> f32 {
    let want_apart = HAND_THICKNESS_MM + CLEARANCE_SLACK_MM;
    let mut needed = 0.0f32;
    for p in up {
        for q in down {
            let flat = (p.x - q.x).hypot(p.y - q.y);
            if flat >= want_apart {
                continue;
            }
            let want = (want_apart * want_apart - flat * flat).sqrt();
            needed = needed.max(want - (p.z - q.z));
        }
    }
    needed
}

fn raise_by(animator: &HandAnimator, pose: &mut HandPose, grip: &Grip, height: f32) {
    if height <= 0.0 {
        return;
    }
    if grip.keys.is_empty() {
        pose.q[dof::WRIST_Z] += height;
        return;
    }
    let raised = |height: f32| {
        let mut trial = *pose;
        let middle = |p: &HandPose| -> glam::Vec3 {
            let posture = animator.skeleton.forward(p);
            grip.keys
                .iter()
                .map(|(_, finger)| posture.chain[finger.index()][3])
                .sum::<glam::Vec3>()
                / grip.keys.len() as f32
        };
        let held = middle(&trial);
        trial.q[dof::WRIST_Z] += height;
        const PROBE: f32 = 0.01;
        const RAISE_LEAST_SLOPE: f32 = 40.0;
        const RAISE_STEP: f32 = 0.12;
        for _ in 0..3 {
            let now = middle(&trial);
            let high = now.z - held.z;
            let mut probe = trial;
            probe.q[dof::WRIST_FLEXION] += PROBE;
            let slope = (middle(&probe).z - now.z) / PROBE;
            let usable = slope.signum() * slope.abs().max(RAISE_LEAST_SLOPE);
            let flexion = &mut trial.q[dof::WRIST_FLEXION];
            let step = (-high / usable).clamp(-RAISE_STEP, RAISE_STEP);
            *flexion = on_hand::skeleton::LIMITS[dof::WRIST_FLEXION].clamp(*flexion + step);
        }
        let now = middle(&trial);
        trial.q[dof::WRIST_X] += held.x - now.x;
        trial.q[dof::WRIST_Y] += held.y - now.y;
        trial
    };
    const RAISE_SLACK_MM: f32 = 0.5;
    let before = animator.joints(pose);
    let trial = raised(height);
    let after = animator.joints(&trial);
    let allowed = before
        .iter()
        .zip(&after)
        .filter(|(was, is)| is.z < was.z)
        .map(|(was, is)| (was.z.max(0.0) + RAISE_SLACK_MM) / (was.z - is.z))
        .fold(1.0f32, f32::min);
    *pose = if allowed >= 1.0 { trial } else { raised(height * allowed) };
}

fn swing_angle(
    animator: &HandAnimator,
    pose: &HandPose,
    grip: &Grip,
    away_from: f32,
    urgency: f32,
) -> f32 {
    if grip.keys.is_empty() {
        return 0.0;
    }
    let mut best = (0.0f32, 0.0f32);
    for sign in [1.0f32, -1.0] {
        let mut trial = *pose;
        let (angle, strayed) =
            swing_by(animator, &mut trial, grip, sign * SWING_MAX_DEG.to_radians() * urgency);
        if strayed > 1.1 {
            continue;
        }
        let gained = (trial.wrist_position().x - away_from).abs()
            - (pose.wrist_position().x - away_from).abs();
        if gained > best.0 {
            best = (gained, angle);
        }
    }
    best.1
}

fn swing_by(animator: &HandAnimator, pose: &mut HandPose, grip: &Grip, angle: f32) -> (f32, f32) {
    if grip.keys.is_empty() || angle == 0.0 {
        return (0.0, 0.0);
    }
    let tips = |p: &HandPose| -> Vec<glam::Vec3> {
        let posture = animator.skeleton.forward(p);
        grip.keys
            .iter()
            .map(|(_, finger)| posture.chain[finger.index()][3])
            .collect()
    };
    let middle = |t: &[glam::Vec3]| t.iter().copied().sum::<glam::Vec3>() / t.len() as f32;
    let before = tips(pose);
    let held = middle(&before);

    let swung = |angle: f32| {
        let mut trial = *pose;
        trial.q[dof::WRIST_DEVIATION] += angle;
        animator.skeleton.clamp(&mut trial);
        let moved = middle(&tips(&trial));
        trial.q[dof::WRIST_X] += held.x - moved.x;
        trial.q[dof::WRIST_Y] += held.y - moved.y;
        let strayed = tips(&trial)
            .iter()
            .zip(&before)
            .map(|(now, was)| {
                ((now.x - was.x).abs() / SWING_TOLERANCE_MM)
                    .max((now.y - was.y).abs() / SWING_SLIDE_MM)
            })
            .fold(0.0f32, f32::max);
        (trial, strayed)
    };

    let mut angle = angle;
    let (mut trial, mut strayed) = swung(angle);
    for _ in 0..2 {
        if strayed <= 1.0 {
            break;
        }
        angle /= strayed;
        (trial, strayed) = swung(angle);
    }
    let turned = trial.q[dof::WRIST_DEVIATION] - pose.q[dof::WRIST_DEVIATION];
    *pose = trial;
    (turned, strayed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use on_fingering::biomech::BiomechWeights;
    use on_hand::HandProfile;
    use on_score::{Note, SourceRef, TieState, TICKS_PER_QUARTER};

    fn score_of(entries: &[(u8, i64, i64, Hand)]) -> Score {
        let mut score = Score::default();
        for (i, (midi, onset, duration, hand)) in entries.iter().enumerate() {
            score.notes.push(Note {
                id: NoteId(i as u32),
                midi: *midi,
                onset: *onset,
                duration: *duration,
                onset_seconds: 0.0,
                duration_seconds: 0.0,
                staff: None,
                voice: None,
                hand: Some(*hand),
                tie: TieState::default(),
                grace: false,
                chord: false,
                velocity: 90,
                given_finger: None,
                source: SourceRef::Midi { track: 0, event: i },
            });
        }
        score.finalise();
        score
    }

    fn pinned(entries: &[(u32, Finger)]) -> Vec<Fingering> {
        entries
            .iter()
            .map(|(id, finger)| Fingering {
                note: NoteId(*id),
                finger: *finger,
                substitute: None,
            })
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
            timeline
                .notes
                .iter()
                .filter(|n| n.midi == midi)
                .nth(nth)
                .expect("note is there")
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
        let window = |n: &TimelineNote| {
            (n.start, (n.end + tail).min(n.restruck.unwrap_or(f64::INFINITY)))
        };
        for pair in timeline.notes.windows(2) {
            let (_, ends) = window(&pair[0]);
            let (starts, _) = window(&pair[1]);
            assert!(
                ends <= starts + 1e-9,
                "a plume ending at {ends} overlaps the next starting at {starts}"
            );
        }
    }

    #[test]
    fn a_timeline_keeps_every_note_with_its_finger() {
        let q = TICKS_PER_QUARTER as i64;
        let score = score_of(&[
            (60, 0, q, Hand::Right),
            (64, q, q, Hand::Right),
            (48, 0, 2 * q, Hand::Left),
        ]);
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
            let (low, high) = event
                .grip
                .keys
                .iter()
                .fold((u8::MAX, u8::MIN), |(lo, hi), (m, _)| (lo.min(*m), hi.max(*m)));
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
        let score = score_of(&[
            (36, 0, 4 * q, Hand::Left),
            (43, 0, 4 * q, Hand::Left),
        ]);
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
    fn a_finger_is_never_asked_to_hold_two_keys(
    ) {
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
        assert!(
            between > struck + 1.0,
            "the hand should rise between strikes: {struck:.2} then {between:.2}"
        );
        let next = height(1.05);
        assert!(
            (next - struck).abs() < 0.5,
            "and be back on the key: {struck:.2} then {next:.2}"
        );
    }

    #[test]
    fn an_unused_hand_waits_out_of_the_way() {
        let q = TICKS_PER_QUARTER as i64;
        let mut score = score_of(&[
            (60, 0, q, Hand::Right),
            (64, q, q, Hand::Right),
            (67, 2 * q, q, Hand::Right),
        ]);
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
        let score = score_of(&[
            (60, 0, 8 * q, Hand::Right),
            (64, 0, 8 * q, Hand::Right),
            (62, 0, q / 4, Hand::Left),
        ]);
        let timeline = Timeline::build(&score, &fingered(&score));
        let animators = timeline.animators(&HandProfile::default(), BiomechWeights::default());

        let at = 1.0;
        let alone = [
            animators[Hand::Left as usize].pose_at(at),
            animators[Hand::Right as usize].pose_at(at),
        ];
        let together = pose_both(&animators, at);

        assert_eq!(
            together[Hand::Right as usize].q[dof::WRIST_Z],
            alone[Hand::Right as usize].q[dof::WRIST_Z],
            "a hand holding keys was lifted off them"
        );
        assert!(
            together[Hand::Left as usize].q[dof::WRIST_Z]
                >= alone[Hand::Left as usize].q[dof::WRIST_Z] - 1e-4,
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
        let animator = HandAnimator::moving(Hand::Right, model, timeline.hand_grips(Hand::Right).to_vec(), &timeline.notes, &MotionStyle::default());
        (timeline, animator)
    }

    #[test]
    fn flowing_hands_move_and_never_step() {
        let q = TICKS_PER_QUARTER as i64;
        let pitches = [60u8, 62, 64, 65, 67, 72, 71, 69, 67, 65, 64, 62, 60];
        let fingers = [1, 2, 3, 1, 2, 5, 4, 3, 2, 1, 3, 2, 1];
        let entries: Vec<_> = pitches.iter().enumerate().map(|(i, m)| (*m, i as i64 * q / 4, q / 4, Hand::Right)).collect();
        let fingered: Vec<_> = fingers.iter().enumerate().map(|(i, f)| (i as u32, Finger::from_number(*f).unwrap())).collect();
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
        let entries: Vec<_> = pitches.iter().enumerate().map(|(i, m)| (*m, i as i64 * q / 4, q / 4, Hand::Right)).collect();
        let fingered: Vec<_> = fingers.iter().enumerate().map(|(i, f)| (i as u32, Finger::from_number(*f).unwrap())).collect();
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
        let (timeline, animator) = flowing(&score_of(&entries), &[(0, Finger::Thumb), (1, Finger::Middle), (2, Finger::Little)]);
        for note in &timeline.notes {
            let finger = note.finger.unwrap();
            let at = note.key_bottoms() + 0.01;
            let tip = animator.joints(&animator.pose_at(at))[1 + 4 * finger.index() + 3];
            let want = -KEY_DIP * note.key_depth(at) as f32;
            assert!((tip.z - want).abs() < 2.0, "{:?} on {} is at {:.1} mm, the key at {want:.1}", finger, note.midi, tip.z);
        }
    }

    #[test]
    fn notes_a_moment_apart_are_one_chord_not_two_leaps() {
        let q = TICKS_PER_QUARTER as i64;
        let score = score_of(&[(46, 0, q, Hand::Left), (58, 2, q, Hand::Left), (62, 4, q, Hand::Left)]);
        let timeline = Timeline::build(&score, &pinned(&[(0, Finger::Little), (1, Finger::Index), (2, Finger::Thumb)]));
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
                let moved =
                    before[side].iter().zip(&now[side]).map(|(a, b)| a.distance(*b)).fold(0.0, f32::max);
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
                alone.q[dof::WRIST_Z], together.q[dof::WRIST_Z],
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
        assert!(
            (height(WRIST_SETTLE_SECONDS + 0.4) - settled).abs() < 0.01,
            "the wrist should come back up"
        );
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
            (posture.chain[Finger::Middle.index()][3].truncate() - posture.wrist.truncate())
                .length()
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
        assert!(
            loud > quiet * 1.8,
            "a loud note should sink much further: {quiet:.2} quiet, {loud:.2} loud"
        );
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
        assert!(
            highest - lowest <= 2.0 * BREATH_MM + 1e-3,
            "and only just: {lowest:.2} to {highest:.2}"
        );
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
        let score = score_of(&[
            (60, 0, 4 * q, Hand::Right),
            (72, 4 * q, q, Hand::Right),
        ]);
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
            assert!(
                height <= resting + 0.01,
                "the hand rose to {height:.2} at {t:.2}s while the key was held"
            );
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
        let score = score_of(&[
            (60, 0, q, Hand::Right),
            (64, 0, q, Hand::Right),
            (67, 0, q, Hand::Right),
        ]);
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
        let animator = HandAnimator::new(
            Hand::Right,
            model,
            timeline.hand_grips(Hand::Right).to_vec(),
        );

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
        let animator = HandAnimator::new(
            Hand::Right,
            model,
            timeline.hand_grips(Hand::Right).to_vec(),
        );
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
        let animator = HandAnimator::new(
            Hand::Right,
            model,
            timeline.hand_grips(Hand::Right).to_vec(),
        );

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
        let fingerings = pinned(&[
            (0, Finger::Middle),
            (1, Finger::Middle),
            (2, Finger::Middle),
            (3, Finger::Middle),
        ]);
        let animator = animator_for(&score, &fingerings, Hand::Right);

        let down = lowest_tip(&animator, 1.0);
        let between = (0..12)
            .map(|i| lowest_tip(&animator, 0.84 + f64::from(i) * 0.01))
            .fold(0.0f32, f32::max);
        assert!(down < 1.0, "the finger should be on the key on the beat: {down}");
        assert!(
            between > 3.0,
            "and off it beforehand, or the note repeats without the hand moving: {between}"
        );
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
        let score = score_of(&[
            (45, 0, 4 * q, Hand::Left),
            (57, 0, q, Hand::Left),
            (60, 0, q, Hand::Left),
        ]);
        let fingerings = pinned(&[
            (0, Finger::Little),
            (1, Finger::Index),
            (2, Finger::Thumb),
        ]);
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
                grips
                    .iter()
                    .map(|e| (e.time, e.grip.keys.clone()))
                    .collect::<Vec<_>>()
            );
        }

        assert!(grips.len() >= 2, "expected a roll, got {} grip(s)", grips.len());
        assert!(
            grips[0].grip.keys.iter().any(|(midi, _)| *midi == 45),
            "the bottom of the chord goes down first"
        );
        assert!(
            grips[1].time > grips[0].time,
            "the rest of it arrives afterwards"
        );
    }

    #[test]
    fn a_hand_never_takes_a_finger_off_a_note_it_is_striking() {
        let q = TICKS_PER_QUARTER as i64;
        let score = score_of(&[
            (28, 0, 8 * q, Hand::Left),
            (40, 0, 8 * q, Hand::Left),
            (52, 4 * q, q, Hand::Left),
        ]);
        let fingerings = pinned(&[
            (0, Finger::Little),
            (1, Finger::Thumb),
            (2, Finger::Thumb),
        ]);
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
}
