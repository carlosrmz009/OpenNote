//! Turning a fingered score into something to watch.
//!
//! This module holds everything about the performance that does not need a GPU: what
//! each key is doing at time *t*, what each hand is holding, and what posture the
//! hands are in. Keeping it separate means the animation can be tested for
//! correctness rather than eyeballed, and means the offline video export and the
//! live window are driven by exactly the same state.
//!
//! # How the hands are animated
//!
//! Not by keyframes. At any moment each hand has a set of keys it must be touching,
//! and [`on_fingering::BiomechModel`] already knows the most comfortable posture that
//! touches them — it is the same calculation that chose the fingering in the first
//! place. So the animation is: solve the posture for the chord being played, solve
//! the posture for the chord coming next, and move between them.
//!
//! Thumb crossings, wrist rotation and the hand travelling up the keyboard are not
//! animated anywhere in this file. They happen because the postures either side of
//! them differ, and the hand has to get from one to the other.

use std::collections::BTreeMap;

use on_fingering::biomech::{BiomechModel, Grip};
use on_hand::keyboard::{is_black, KEY_DIP, MIDI_HIGHEST, MIDI_LOWEST};
use on_hand::skeleton::{dof, HandPose};
use on_hand::{Finger, Hand, HandProfile};
use on_score::{Fingering, NoteId, Score};

/// How long before a note sounds the hand starts moving toward it, in seconds.
///
/// Pianists arrive early; a hand that jumped into place at the instant of the note
/// would look like a machine.
pub const APPROACH_SECONDS: f64 = 0.28;

/// How far the hand rises between letting a key go and striking the next, per second
/// of silence, and the most it ever rises — both in millimetres.
///
/// This is what makes a repeated note look repeated. Two strikes of the same key have
/// the same posture, so without a lift between them the hand sits perfectly still
/// while the key goes down twice. Pianists do not play staccato from the knuckles;
/// the wrist bounces, and the whole hand comes up with it.
const LIFT_RATE_MM: f32 = 120.0;
const LIFT_MAX_MM: f32 = 24.0;

/// How long a key takes to travel down when struck, in seconds.
const KEY_ATTACK_SECONDS: f64 = 0.035;

/// How long a key takes to come back up when released.
const KEY_RELEASE_SECONDS: f64 = 0.06;

/// One note as the visualizer needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineNote {
    /// Which note in the score.
    pub id: NoteId,
    /// MIDI pitch.
    pub midi: u8,
    /// When it sounds.
    pub start: f64,
    /// When the key comes up. What the hand does, and what is drawn.
    pub end: f64,
    /// When the string is actually damped, which the pedal can put off.
    ///
    /// Separate from `end` on purpose: the hand lets the key go when it lets it go, and
    /// the sound carries on without it. Everything about the hands and the falling notes
    /// reads `end`; only the sound reads this.
    pub damped: f64,
    /// Which hand plays it.
    pub hand: Hand,
    /// Which finger plays it.
    pub finger: Option<Finger>,
    /// Loudness, 1..=127.
    pub velocity: u8,
}

impl TimelineNote {
    /// Whether the note is sounding at a given moment.
    pub fn sounds_at(&self, time: f64) -> bool {
        self.start <= time && time < self.end
    }

    /// Whether it lands on a black key.
    pub fn is_black(&self) -> bool {
        is_black(self.midi)
    }
}

/// A chord one hand plays, with the time it is played.
#[derive(Debug, Clone, PartialEq)]
pub struct GripEvent {
    /// When the hand takes this shape.
    pub time: f64,
    /// When the last of its notes is released.
    pub release: f64,
    /// The keys and the fingers on them.
    pub grip: Grip,
    /// Which fingers actually strike here, and how hard.
    ///
    /// Not the same as the grip: most of what a hand is holding at any instant it was
    /// already holding a moment ago. Only the fingers in here come down, and only they
    /// are lifted beforehand to do it.
    pub struck: Vec<(Finger, u8)>,
}

/// A whole performance, ready to draw.
#[derive(Debug, Clone)]
pub struct Timeline {
    /// Every note, in time order.
    pub notes: Vec<TimelineNote>,
    /// What each hand does, in time order.
    pub grips: [Vec<GripEvent>; 2],
    /// How long the piece lasts, in seconds.
    pub duration: f64,
    /// Title, if the score had one.
    pub title: Option<String>,
}

impl Timeline {
    /// The notes as the synthesiser wants them.
    ///
    /// Held notes are shortened very slightly. Two things need it: a repeated note
    /// wants a hair of silence before it is struck again, or the damper never gets to
    /// touch the string; and scores quantised onto a grid often have one note ending
    /// on the same instant the next begins, which sounds tied rather than repeated.
    pub fn audio_notes(&self) -> Vec<on_audio::Note> {
        const RELEASE_GAP: f64 = 0.012;
        self.notes
            .iter()
            .map(|note| on_audio::Note {
                midi: note.midi,
                start: note.start,
                // A note the pedal is holding does not want the gap: there is no damper
                // on the string to be given time to land.
                end: if note.damped > note.end + 1e-6 {
                    note.damped
                } else {
                    (note.end - RELEASE_GAP).max(note.start + 0.02)
                },
                velocity: note.velocity,
            })
            .collect()
    }

    /// Build a timeline from a fingered score, for a hand of the default size.
    pub fn build(score: &Score, fingerings: &[Fingering]) -> Self {
        Self::build_for(score, fingerings, &HandProfile::default())
    }

    /// Build a timeline from a fingered score, for a particular hand.
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
                    // Following the tie, or a note tied across a bar line stops
                    // sounding — and stops being held by a hand — halfway through.
                    end: score.release_seconds(n.id),
                    damped: score.damped_seconds(n.id),
                    hand: n.hand?,
                    finger: by_note.get(&n.id).copied(),
                    velocity: n.velocity,
                })
            })
            .collect();
        notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.midi.cmp(&b.midi)));

        let spans = on_fingering::SpanModel::for_hand_size(profile.size()).table();
        let grips = Hand::ALL.map(|hand| {
            let model = BiomechModel::new(profile.clone(), hand, Default::default());
            Self::grips_for(&notes, hand, spans, &model)
        });
        let duration = notes.iter().map(|n| n.end).fold(0.0, f64::max);

        Self { notes, grips, duration, title: score.title.clone() }
    }

    /// What one hand is holding at each moment it strikes something.
    ///
    /// A grip is everything the hand has *down*, not only what it has just played.
    /// Getting that wrong is very visible: a left hand holding two long bass notes
    /// while the right hand plays above it would let go of them and wander off after
    /// its next note, because the held notes were not part of the shape any more.
    ///
    /// So this sweeps through the onsets keeping a list of what is still sounding.
    /// A finger can only be on one key, so if a later note wants a finger that an
    /// earlier one is using, the earlier note has been let go of — whatever the score
    /// says, it is the pedal holding it now, not the hand.
    ///
    /// The same argument settles what to do when the score asks one hand to hold more
    /// than it can span. It cannot, and neither can a pianist: a bass note held while
    /// the melody climbs two octaves above it is being held by the pedal, and the hand
    /// left it behind long ago. Keeping it in the grip is what produced hands with
    /// their fingers splayed flat across half the keyboard — the animator was faithfully
    /// drawing a shape nobody could make. So when the shape runs past what this hand
    /// can hold, the notes struck longest ago are let go until it fits.
    ///
    /// What counts as fitting is asked of the hand itself rather than of a number of
    /// semitones. The two are not the same question: a thumb and a little finger span
    /// well over an octave, while a ring finger and a little finger manage a third, and
    /// a rule written in semitones cannot tell those apart. It let through shapes the
    /// hand could not make, and the model, asked to make them anyway, drove the hand
    /// down through the keyboard reaching for a key it was never going to get to.
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
            // Everything that has stopped sounding is no longer held.
            held.retain(|(note, _)| note.end > time + 1e-6);

            // Everything struck at this instant joins, displacing whatever was using
            // the same finger.
            while index < mine.len() && mine[index].0.start <= time + 1e-6 {
                let (note, finger) = mine[index];
                held.retain(|(_, other)| *other != finger);
                held.push((note, finger));
                index += 1;
            }

            // Let go of the oldest until the hand can hold what is left. Notes struck
            // at this instant are never dropped: those are what the hand is playing
            // right now.
            //
            // Asked of every pair of fingers in the shape rather than of its width.
            // The two are different questions: a thumb and a little finger span well
            // over an octave while a ring finger and a little finger manage a third, and
            // a single number of semitones cannot tell those apart. It let through
            // shapes the hand could not make, and the hand model, asked to make one
            // anyway, drove the hand down through the keyboard reaching for a key it was
            // never going to get to.
            let holdable = |held: &[(&TimelineNote, Finger)]| {
                held.iter().enumerate().all(|(i, (low, from))| {
                    held[i + 1..].iter().all(|(high, to)| {
                        let apart = i32::from(high.midi) - i32::from(low.midi);
                        spans.get(hand, *from, *to).is_practical(apart)
                    })
                })
            };

            // What the hand lets go of first: whatever it has been holding longest.
            //
            // Only ever something older. A note struck at this instant is one the hand
            // is playing right now, and taking a finger off it lights a key with
            // nothing on it — which is precisely the fault this is supposed to prevent.
            // When there is nothing older left to give up, the shape is a chord wider
            // than the hand, and the answer to that is to roll it rather than to drop
            // part of it on the floor.
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

            // The hand itself has the last word on a shape. The table is a chart of what
            // pairs of fingers can span and it does not know what the rest of the hand
            // is doing at the time; the model solves the whole posture and can say that
            // a shape every pair of which is fine is still one the hand cannot get into.
            // It is asked second because it is an inverse-kinematics solve and the table
            // has already thrown out everything obviously too wide, so by the time it is
            // asked it nearly always says yes and says it quickly.
            let makeable = |held: &[(&TimelineNote, Finger)]| {
                holdable(held) && {
                    let shape = Grip::new(
                        held.iter().map(|(note, finger)| (note.midi, *finger)).collect(),
                    );
                    model.grip_outcome(&shape).reachable
                }
            };

            // First let go of anything older the hand cannot keep. That is the cheap
            // answer and usually the right one: the key is already down, and under the
            // pedal it goes on sounding after the finger leaves.
            while held.len() > 1 && !makeable(&held) && drop_one(&mut held) {}

            // What is left is a chord struck at this instant, and if the hand still
            // cannot make the shape then the chord is simply wider than the hand. That
            // is not a fingering that went wrong — it is a chord that has to be rolled,
            // and rolling it is what a pianist does: strike the bottom, let the pedal
            // keep it, carry the hand up to the rest.
            //
            // Peeling the lowest notes off into a grip of their own says exactly that,
            // and it is the only answer that leaves every struck note with a finger on
            // it. Dropping one instead lit a key that nothing was touching, which is
            // the thing anybody watching notices first.
            let mut rolled: Vec<(&TimelineNote, Finger)> = Vec::new();
            while held.len() > 1 && !makeable(&held) {
                rolled.push(held.remove(0));
            }

            // The bottom of a rolled chord goes down first, on the beat, and the hand
            // arrives at the rest of it a moment later.
            let mut grip_time = time;
            if !rolled.is_empty() {
                events.push(grip_event(&rolled, time, time));
                grip_time = time + ROLL_SECONDS;
            }

            let struck: Vec<(Finger, u8)> = held
                .iter()
                .filter(|(note, _)| (note.start - time).abs() < 1e-6)
                .map(|(note, finger)| (*finger, note.velocity))
                .collect();
            let keys: Vec<(u8, Finger)> = held
                .iter()
                .map(|(note, finger)| (note.midi, *finger))
                .collect();
            let release = held.iter().fold(time, |latest, (note, _)| latest.max(note.end));
            let mut grip = Grip::new(keys);
            grip.keys.sort_by_key(|(midi, _)| *midi);
            events.push(GripEvent { time: grip_time, release, grip, struck });
        }
        events
    }

    /// The grips for one hand.
    pub fn hand_grips(&self, hand: Hand) -> &[GripEvent] {
        &self.grips[hand as usize]
    }

    /// How far each key is pressed at a moment, from 0 (up) to 1 (fully down).
    ///
    /// Keys fall quickly and rise a little more slowly, which is what a real action
    /// does and what stops the keyboard looking like it is flickering.
    pub fn key_depression(&self, time: f64) -> KeyStates {
        let mut states = KeyStates::default();
        for note in &self.notes {
            if time < note.start - 0.001 || time > note.end + KEY_RELEASE_SECONDS {
                continue;
            }
            let depth = if time < note.start {
                0.0
            } else if time < note.start + KEY_ATTACK_SECONDS {
                // A louder note puts the key down faster.
                let t = (time - note.start) / KEY_ATTACK_SECONDS;
                let sharpness = 0.5 + note.velocity as f64 / 127.0;
                (t * sharpness).min(1.0)
            } else if time < note.end {
                1.0
            } else {
                let t = (time - note.end) / KEY_RELEASE_SECONDS;
                (1.0 - t).max(0.0)
            };
            let slot = states.slot(note.midi);
            states.depth[slot] = states.depth[slot].max(depth as f32);
            if note.sounds_at(time) {
                states.hand[slot] = Some(note.hand);
            }
        }
        states
    }

    /// Notes visible in the falling-note lane at a moment.
    ///
    /// A note enters the lane `lookahead` seconds before it sounds and leaves it when
    /// it is released.
    pub fn visible_notes(&self, time: f64, lookahead: f64) -> impl Iterator<Item = &TimelineNote> {
        self.notes
            .iter()
            .filter(move |n| n.end > time - 0.5 && n.start < time + lookahead)
    }
}

/// What every key on the instrument is doing.
#[derive(Debug, Clone)]
pub struct KeyStates {
    /// Depression from 0 to 1, indexed by `midi - MIDI_LOWEST`.
    pub depth: Vec<f32>,
    /// Which hand is holding the key, if any.
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

    /// How far a key is pressed, 0 to 1.
    pub fn depth_of(&self, midi: u8) -> f32 {
        self.depth[self.slot(midi)]
    }

    /// How far the key has physically travelled, in millimetres.
    pub fn travel_of(&self, midi: u8) -> f32 {
        self.depth_of(midi) * KEY_DIP
    }

    /// Which hand is on a key, if any.
    pub fn hand_on(&self, midi: u8) -> Option<Hand> {
        self.hand[self.slot(midi)]
    }

    /// Every key currently down.
    pub fn pressed(&self) -> impl Iterator<Item = u8> + '_ {
        (MIDI_LOWEST..=MIDI_HIGHEST).filter(move |m| self.depth_of(*m) > 0.05)
    }
}

/// How long after the bottom of a rolled chord the rest of it arrives, in seconds.
///
/// A roll, not an arpeggio: fast enough to read as one chord rather than as separate
/// notes, slow enough that the hand is visibly somewhere else by the time it gets
/// there. Pianists roll a wide chord in about this long.
const ROLL_SECONDS: f64 = 0.075;

/// Assemble one grip from the notes a hand has down.
fn grip_event(
    notes: &[(&TimelineNote, Finger)],
    time: f64,
    struck_at: f64,
) -> GripEvent {
    let struck: Vec<(Finger, u8)> = notes
        .iter()
        .filter(|(note, _)| (note.start - struck_at).abs() < 1e-6)
        .map(|(note, finger)| (*finger, note.velocity))
        .collect();
    let mut grip = Grip::new(notes.iter().map(|(note, finger)| (note.midi, *finger)).collect());
    grip.keys.sort_by_key(|(midi, _)| *midi);
    let release = notes.iter().fold(time, |latest, (note, _)| latest.max(note.end));
    GripEvent { time, release, grip, struck }
}

/// How long before a strike a finger starts to lift, in seconds.
///
/// Short. It is a preparation, not a wind-up, and a run of quick notes has less time
/// than this between them anyway — the window shrinks to whatever there is.
const STRIKE_LIFT_SECONDS: f64 = 0.16;

/// How far the knuckle extends to lift the finger, in degrees, for the softest note
/// and for the hardest.
///
/// Small. What has to read at a glance is *that* the finger came off the key and went
/// back down, not how far it went: a couple of degrees is already several millimetres
/// at the fingertip, and the eye is reading the tip. Taken up to the seventeen degrees
/// this first used, the knuckle straightens far enough that the finger stops looking
/// like a finger — it reads as long and loose, and the hand appears to flail at the
/// keyboard rather than play it.
const STRIKE_LIFT_DEG: (f32, f32) = (2.5, 9.0);

/// How far through the window the finger is at the top of its lift, softest to hardest.
///
/// A quiet note rises and settles in about the same time. A loud one is taken up most
/// of the window and then dropped, which is what makes it look struck rather than
/// placed — but not so late that the fall becomes a snap, which is the other half of
/// looking exaggerated.
const STRIKE_PEAK: (f32, f32) = (0.5, 0.72);

/// Poses one hand over time by interpolating between the postures the fingering
/// implies.
pub struct HandAnimator {
    hand: Hand,
    /// Kept so the renderer can place the joints; the biomechanical model itself is
    /// only needed while the postures are being solved and is dropped afterwards.
    skeleton: on_hand::Skeleton,
    events: Vec<GripEvent>,
    /// The solved posture for each event, in the same order.
    poses: Vec<HandPose>,
    /// Where the hand waits before the first note and after the last.
    resting: HandPose,
}

impl HandAnimator {
    /// Solve every posture the hand will need. Done once, up front: there are only
    /// as many as there are chords, and the octave-equivalence cache in the
    /// biomechanical model collapses most of those.
    pub fn new(hand: Hand, model: BiomechModel, events: Vec<GripEvent>) -> Self {
        let poses: Vec<HandPose> = events.iter().map(|e| model.grip_pose(&e.grip)).collect();

        // Before the music starts and after it ends the hand waits over the first
        // thing it has to play, rather than snapping in from nowhere.
        let resting = poses.first().copied().unwrap_or_else(|| {
            let centre = model.keyboard().centre_x(60);
            model
                .skeleton()
                .rest_pose(glam::Vec3::new(centre, -80.0, 60.0))
        });

        let skeleton = model.skeleton().clone();
        Self { hand, skeleton, events, poses, resting }
    }

    /// Which hand this animates.
    pub fn hand(&self) -> Hand {
        self.hand
    }

    /// The kinematics, for placing the joints of the rendered model.
    pub fn skeleton(&self) -> &on_hand::Skeleton {
        &self.skeleton
    }

    /// Index of the last grip taken at or before a moment.
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

    /// The posture of the hand at a moment.
    ///
    /// Between two chords the hand eases from one posture to the next, starting to
    /// move [`APPROACH_SECONDS`] before it is needed or as soon as the previous chord
    /// is released, whichever is later. That is the whole of the movement logic: what
    /// it looks like comes from the postures, not from here.
    pub fn pose_at(&self, time: f64) -> HandPose {
        let Some(index) = self.current_index(time) else {
            // Before the first note: hold the opening posture.
            return self.resting;
        };
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

        // Whether the hand is standing still on either side of this move. If it is, the
        // move should start and finish at rest; if it is not — because the chord was
        // released the instant the next one was due, and the one after that too — then
        // stopping at each note is exactly the mechanical look this is trying to avoid.
        let resting_before = start > current.time + 1e-4;
        let resting_after = self
            .events
            .get(index + 2)
            .is_none_or(|_| self.departure(index + 1) > next.time + 1e-4);

        let mut moved = pose.lerp(&next_pose, travel(t, resting_before, resting_after) as f32);
        moved.q[dof::WRIST_Z] += lift_between(current, next, time);
        self.raise_fingers_about_to_strike(&mut moved, index, time);
        moved
    }

    /// Lift the fingers that are about to play, and drop them on the beat.
    ///
    /// Without this a repeated note does not move at all: the shape of the hand before
    /// it and after it are the same shape, so there is nothing for the interpolation to
    /// do and the finger simply stays on the key while the note sounds again underneath
    /// it. What a hand actually does is let the key come up and hit it again.
    ///
    /// Only fingers that were already down are lifted. One arriving from somewhere else
    /// is in the air already and is being carried there by the interpolation, and lifting
    /// it as well would give it a second, separate hop.
    ///
    /// How far it rises and how it falls both come from how hard the note is played. A
    /// quiet note is a small movement that settles gently; a loud one is taken from
    /// higher up and dropped, spending most of the window on the way up and very little
    /// coming down. That difference is most of what a strike looks like.
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
            // A finger that was not already on a key is on its way to one, and the
            // interpolation is already carrying it.
            if !current.grip.keys.iter().any(|(_, f)| f == finger) {
                continue;
            }
            let hardness = f32::from(*velocity) / 127.0;
            let height = STRIKE_LIFT_DEG.0 + (STRIKE_LIFT_DEG.1 - STRIKE_LIFT_DEG.0) * hardness;
            // Where in the window the finger is at the top. Later for a hard note, which
            // is what makes the fall quick.
            let peak = STRIKE_PEAK.0 + (STRIKE_PEAK.1 - STRIKE_PEAK.0) * hardness;
            let shape = if through < peak {
                through / peak
            } else {
                (1.0 - through) / (1.0 - peak)
            };
            let lift = height.to_radians() * shape.clamp(0.0, 1.0);

            // Extending the knuckle lifts the tip; the middle joint curls back by half
            // as much again, which is what keeps the finger looking like a finger. A
            // knuckle that extends on its own straightens the whole digit as it rises,
            // and a straightened finger is the thing that reads as lanky.
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

    /// When the hand leaves one chord for the next.
    ///
    /// As soon as the chord has been let go, but never later than the approach window
    /// before the next one is due — pianists arrive early.
    fn departure(&self, index: usize) -> f64 {
        let current = &self.events[index];
        let Some(next) = self.events.get(index + 1) else {
            return current.time;
        };
        let latest = next.time - APPROACH_SECONDS;
        let start = current.release.min(next.time).max(current.time);
        start.min(latest.max(current.time))
    }

    /// The grip the hand is holding at a moment, if any.
    pub fn grip_at(&self, time: f64) -> Option<&Grip> {
        let index = self.current_index(time)?;
        let event = &self.events[index];
        (time < event.release).then_some(&event.grip)
    }
}

/// How far through a move the hand is, given whether it is at rest at each end.
///
/// A cubic with its end velocities chosen to match what is on either side. Where the
/// hand is standing still before and after — holding a chord, then holding the next —
/// both are zero and this is the usual smooth start and smooth stop. Where it is not,
/// because one chord was let go exactly as the next fell due and the same again after
/// that, the velocity carries through instead of dropping to nothing and picking up
/// again. Easing every move at both ends regardless is what makes a fast passage look
/// like a series of separate lunges rather than one continuous line.
fn travel(t: f64, resting_before: bool, resting_after: bool) -> f64 {
    let t = t.clamp(0.0, 1.0);
    // Hermite tangents, in the same units as the parameter: nought to arrive at rest,
    // one to carry straight on at the speed the whole move averages.
    let m0 = if resting_before { 0.0 } else { 1.0 };
    let m1 = if resting_after { 0.0 } else { 1.0 };
    let (t2, t3) = (t * t, t * t * t);
    (-2.0 * t3 + 3.0 * t2) + m0 * (t3 - 2.0 * t2 + t) + m1 * (t3 - t2)
}

/// Smooth acceleration and deceleration, so a hand starts and stops rather than
/// snapping between positions.
/// How far off the keys the hand is, part way through a silence.
///
/// Zero while anything is still held, so this never lifts a finger off a key it is
/// supposed to be holding; a half-sine over the gap, so the hand is back down by the
/// time the next note sounds. Longer gaps get a bigger bounce, up to a limit — the
/// hand does not rise a foot in the air over a two-bar rest.
fn lift_between(current: &GripEvent, next: &GripEvent, time: f64) -> f32 {
    let gap = next.time - current.release;
    if gap <= 1e-3 || time <= current.release || time >= next.time {
        return 0.0;
    }
    let height = (gap as f32 * LIFT_RATE_MM).min(LIFT_MAX_MM);
    let through = ((time - current.release) / gap).clamp(0.0, 1.0) as f32;
    height * (through * std::f32::consts::PI).sin()
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

    /// Fingerings chosen here rather than by the solver, for tests about what the
    /// hand holds rather than about what it is told to play.
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

    /// A hand does not let go of what it is holding.
    ///
    /// The left hand holds two long bass notes while playing shorter ones above
    /// them. Every grip in between has to still contain the held notes, or the hand
    /// walks off the keys it is supposed to be holding down — which is exactly what
    /// it did when a grip was only the notes that *started* at that instant.
    #[test]
    fn a_hand_lets_go_of_what_it_cannot_span() {
        // A bass note held right through, with a melody climbing two octaves above it.
        // No hand can hold both at once, so the bass has to be released to the pedal —
        // and if it is not, the animator draws a hand splayed flat across the keyboard.
        let q = TICKS_PER_QUARTER as i64;
        let score = score_of(&[
            (36, 0, 8 * q, Hand::Left),
            (60, q, q, Hand::Left),
            (67, 2 * q, q, Hand::Left),
            (72, 3 * q, q, Hand::Left),
            (79, 4 * q, q, Hand::Left),
        ]);
        // Distinct fingers throughout, so the span rule is the only thing that can let
        // a note go. Left to the solver, the finger-conflict rule drops notes for its
        // own reasons and this would pass whether or not the span rule exists.
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
            // Two notes held for a whole bar.
            (36, 0, 4 * q, Hand::Left),
            (41, 0, 4 * q, Hand::Left),
            // And three short ones above them, inside that bar. The highest is a major
            // tenth above the bass, which a medium hand can just hold — the point of
            // this test is that a reachable hold is kept, and it used to reach a
            // twelfth, which no hand can hold and which the grip now releases.
            (45, q, q, Hand::Left),
            (46, 2 * q, q, Hand::Left),
            (48, 3 * q, q, Hand::Left),
        ]);
        // Fingers pinned rather than solved for, so this tests the grip and nothing
        // else. Left to the solver it can hand two of these notes the same finger,
        // which legitimately displaces the earlier one and has nothing to do with what
        // is being checked here.
        //
        // The two held notes are a fourth apart under the little finger and the middle,
        // which is a hold a hand comfortably has. Pinning them further apart, or onto
        // the ring finger and the little finger — which span a fourth between them and
        // no more — asks for a hold no hand has, and the grip is then quite right to let
        // it go, which is the other half of this behaviour and not what is under test
        // here.
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
        // A ring finger and a little finger span a fourth between them and no more —
        // that is what the measured tables say and what anyone can check on their own
        // hand. Asked to hold a fifth with them, the grip has to let one go.
        //
        // It used to be asked of the width of the whole shape instead, one number of
        // semitones for the whole hand, which cannot tell a thumb and a little finger
        // spanning a tenth from a ring and a little finger spanning a fifth. The shapes
        // that got through were drawn, and the hand model, asked to make one anyway,
        // drove the hand down through the keyboard reaching for a key it was never
        // going to get to.
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

    /// One finger, one key. If a later note wants a finger an earlier one is using,
    /// the hand has moved on and the pedal is holding the old note, not the finger.
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

    /// A repeated note has to look repeated.
    ///
    /// Two strikes of the same key give the same posture, so the only thing that can
    /// show the second strike is the hand coming up in between.
    #[test]
    fn the_hand_bounces_between_repeated_notes() {
        let q = TICKS_PER_QUARTER as i64;
        // Four staccato strikes of the same key: sounding for a third of each beat.
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
        // Down on the key when it sounds, and up in the silence between.
        let struck = height(0.55);
        let between = height(0.75);
        assert!(
            between > struck + 1.0,
            "the hand should rise between strikes: {struck:.2} then {between:.2}"
        );
        // And back down for the next one.
        let next = height(1.05);
        assert!(
            (next - struck).abs() < 0.5,
            "and be back on the key: {struck:.2} then {next:.2}"
        );
    }

    /// The lift must never pull a finger off a key that is still held.
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
        let resting = animator.pose_at(0.05).q[dof::WRIST_Z];
        for step in 0..20 {
            let t = 0.05 + step as f64 * 0.09;
            let height = animator.pose_at(t).q[dof::WRIST_Z];
            assert!(
                height <= resting + 0.01,
                "the hand rose to {height:.2} at {t:.2}s while the key was held"
            );
        }
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
        // Keys come out low to high, which is what the hand model expects.
        assert!(grips[0].grip.keys.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn the_hand_moves_between_chords_and_arrives_on_time() {
        let q = TICKS_PER_QUARTER as i64;
        // Two chords a bar apart, so there is plenty of time to travel.
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

        // And it should be where it needs to be exactly when the chord sounds.
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

        // Sample densely across the move and check no single step is a large jump.
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
        // Slower at the ends than in the middle, which is what "at rest" means.
        assert!(travel(0.1, true, true) < 0.1);
        assert!(travel(0.9, true, true) > 0.9);
    }

    #[test]
    fn a_move_the_hand_is_already_making_does_not_stop_for_it() {
        // Still passes through both ends...
        assert!((travel(0.0, false, false) - 0.0).abs() < 1e-9);
        assert!((travel(1.0, false, false) - 1.0).abs() < 1e-9);
        // ...but carries its speed through them rather than dropping to nothing. This
        // is the difference between a run of notes read as one line and read as a
        // series of separate lunges.
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

    /// How high the lowest fingertip is above the keys at a moment.
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
        // The same key four times over. The shape of the hand before each one and after
        // it is the same shape, so there is nothing for the interpolation to do — left
        // to itself the finger simply stays on the key while the note sounds again
        // underneath it, which is what a piano roll does and not what a hand does.
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

        // On the key when it sounds, and off it in between.
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
        // What a strike looks like is mostly the difference between placing a finger and
        // dropping one. A quiet note rises and settles in about the same time; a loud one
        // is taken up almost the whole way and then let go.
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

        // A tenth of a second before the strike: the quiet finger is already on its way
        // down, the loud one is still up.
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
        // A minor tenth in the left hand with a note in the middle of it. No fingering
        // of that is pairwise holdable — whichever pair takes the outside, the middle
        // note is a twelfth from one of them — so the shape has to come apart somehow.
        // It comes apart in time, the way a pianist takes it: the bottom, then the
        // rest. What it must not do is come apart in space, leaving a struck key with
        // no finger anywhere near it.
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

        // And it really did roll: the bottom on the beat, the rest just after.
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
        // The general form of the same fault. Whatever the shape, and whatever the hand
        // has to give up to make it, the thing it gives up is never the note it is in
        // the act of playing.
        let q = TICKS_PER_QUARTER as i64;
        let score = score_of(&[
            // A bass note held under a figure that walks away from it.
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
