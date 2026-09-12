//! Judging a fingering with no answer key.
//!
//! Agreement with expert annotations is the measure the literature reports, and it
//! needs a licensed corpus this repository does not ship. These two measures need
//! nothing at all: given a fingering, they ask whether a hand could perform it and how
//! much that hand would have to move. Both can therefore be run on the music actually
//! being rendered, which agreement never can.
//!
//! * **IFR**, the incapable-performing fingering rate of Zhao, Guan & Li (2021, §4.2.2)
//!   — the fraction of note transitions that no hand can make. It should be zero. It
//!   is the one number here with a right answer, which makes it a regression test
//!   rather than a benchmark.
//! * **R_pc**, the change position rate of Ramoneda, Jeong, Nakamura, Serra & Miron
//!   (2022, §5.1) — how often the hand leaves the position it was in, either by
//!   crossing the thumb or by shifting bodily. Pianists minimise this, so lower is
//!   better, but only to a point: a fingering that never moves the hand is usually one
//!   that has stopped playing the music. It is read against a reference, not alone.
//!
//! Neither replaces agreement. A fingering can be perfectly playable, barely move, and
//! still be nothing a pianist would write. What they catch is the opposite failure —
//! output that scores respectably against annotations while containing a transition
//! that cannot physically be made — and that failure is invisible to a match rate.
//!
//! # Reading a chord
//!
//! A transition is between two *events*, not two notes: notes struck within
//! [`CHORD_SECONDS`] of each other are one chord, and the fingers inside it are
//! simultaneous rather than sequential. Nothing inside a chord is a transition.
//!
//! Between two chords, both edges are measured — lowest to lowest and highest to
//! highest — which is the same convention the solver's own horizontal rules use. For
//! single notes the two edges coincide and this reduces to the plain note pair the
//! papers describe.

use on_hand::{Finger, Hand};
use on_score::Score;

use crate::rules::Placement;
use crate::solver::Solution;
use crate::spans::SpanTable;

/// How close two onsets must be to count as one chord.
///
/// Nakamura, Saito & Yoshii (2020, §5.2.3) take 30 ms from the score-following work of
/// Nakamura et al. (2014), where it is the measured spread of notes a pianist intends
/// as simultaneous.
pub const CHORD_SECONDS: f64 = 0.030;

/// One transition that cannot be made, and where in the piece it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flaw {
    /// Which hand was asked to make it.
    pub hand: Hand,
    /// When the second of the two notes sounds, so it can be found in a recording.
    pub onset_seconds: f64,
    /// The note being left.
    pub from: Placement,
    /// The note being reached for.
    pub to: Placement,
    /// How long the hand had between the two, in seconds.
    ///
    /// The measure itself ignores this, because the published definition does: PIG
    /// carries no durations, so every system built on it is timing-blind. It is
    /// recorded because it is what separates the two kinds of flag. A crossing with
    /// milliseconds between the notes is a fingering no hand can play. The same
    /// crossing with half a second between them is a hand that had time to move, and
    /// the measure simply cannot see that it did.
    pub seconds_available: f64,
}

impl std::fmt::Display for Flaw {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:>8.2}s  {:<5} {:>4} {} -> {:<4} {}   {:.0} ms to move",
            self.onset_seconds,
            format!("{:?}", self.hand).to_lowercase(),
            on_hand::keyboard::name(self.from.midi),
            self.from.finger.number(),
            on_hand::keyboard::name(self.to.midi),
            self.to.finger.number(),
            self.seconds_available * 1000.0,
        )
    }
}

/// What a fingering costs a hand, measured against nothing but itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Playability {
    /// Transitions no hand can make.
    pub impossible: usize,
    /// Each of them, in time order, for a caller that wants to go and look.
    pub flaws: Vec<Flaw>,
    /// Position changes made by passing the thumb.
    pub crossings: usize,
    /// Position changes made by moving the hand bodily.
    pub shifts: usize,
    /// Transitions examined, which is what the rates are over.
    pub transitions: usize,
}

impl Playability {
    /// The incapable-performing fingering rate: the fraction that cannot be played.
    ///
    /// Zero is the only acceptable value. Anything else is a bug in the search, not a
    /// difference of opinion about fingering.
    pub fn incapable_rate(&self) -> f32 {
        rate(self.impossible, self.transitions)
    }

    /// Position changes of both kinds.
    pub fn position_changes(&self) -> usize {
        self.crossings + self.shifts
    }

    /// Position changes per transition.
    ///
    /// Ramoneda et al. report this divided again by the same figure for expert
    /// annotations, so that 1.0 means "moves the hand as much as a pianist does". That
    /// last division needs the annotations; this is the half that does not.
    pub fn change_rate(&self) -> f32 {
        rate(self.position_changes(), self.transitions)
    }

    /// Add another hand's, or another piece's, tally to this one.
    pub fn add(&mut self, other: Playability) {
        self.impossible += other.impossible;
        self.flaws.extend(other.flaws);
        self.crossings += other.crossings;
        self.shifts += other.shifts;
        self.transitions += other.transitions;
    }
}

impl std::fmt::Display for Playability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "IFR {:.2}%   position changes {:.1}% ({} crossing, {} shift, {} transitions)",
            self.incapable_rate() * 100.0,
            self.change_rate() * 100.0,
            self.crossings,
            self.shifts,
            self.transitions
        )
    }
}

fn rate(part: usize, whole: usize) -> f32 {
    if whole == 0 {
        0.0
    } else {
        part as f32 / whole as f32
    }
}

/// Which way the finger numbers run as pitch rises.
///
/// The thumb is finger 1 in both hands, and it sits at the low end of the right hand
/// and the high end of the left. So for the right hand a rising pitch and a rising
/// finger number agree, and for the left hand they disagree, and every judgement below
/// is about whether they agree in that hand's own terms.
fn outward(hand: Hand, semitones: i32) -> i32 {
    match hand {
        Hand::Right => semitones,
        Hand::Left => -semitones,
    }
}

/// Whether a hand can get from one note to the next at all.
///
/// Zhao, Guan & Li (2021, §3.4) state the constraint as: fingers other than the thumb
/// do not cross one another. If the pitch moves one way and the finger numbers move
/// the other, the fingers have crossed, and that is only something the thumb does.
///
/// Their test for "the thumb is involved" is that the product of the two finger
/// numbers exceeds 4.5, which is neater than it first looks. Every thumb crossing a
/// pianist actually uses — 1↔2, 1↔3, 1↔4 — has a product of 4 or less. Crossing the
/// thumb over the little finger, 1↔5, gives 5, and is excluded along with every
/// non-thumb pair. That matches their Table 1 cell for cell.
///
/// The octave bound is theirs too: past an octave the hand has plainly been moved, so
/// there is no claim left to make about which finger crossed which.
///
/// Inside an octave the measure cannot tell a moved hand from a still one, so it will
/// occasionally call a real fingering impossible — Zhao et al. note the case in their
/// own ground truth and count it. The bound is kept rather than papered over, because
/// a measure that excuses anything the span tables permit would excuse the bugs it
/// exists to find: the tables are part of what is being measured.
fn impossible(hand: Hand, from: Placement, to: Placement) -> bool {
    let semitones = i32::from(to.midi) - i32::from(from.midi);
    if semitones == 0 || semitones.abs() >= 12 {
        return false;
    }
    let fingers = i32::from(to.finger.number()) - i32::from(from.finger.number());
    // The same finger twice is a fault of a different kind, and the solver already
    // forbids it outright as `Rule::RepeatedFinger`. Counting it here as well would
    // report one bug as two.
    if fingers == 0 {
        return false;
    }
    if outward(hand, semitones).signum() == fingers.signum() {
        // Pitch and fingers agree: whatever else is wrong with it, nothing crossed.
        return false;
    }
    // A thumb crossing is the one crossing hands make, so the product test passing is
    // what makes this impossible rather than merely awkward.
    i32::from(from.finger.number()) * i32::from(to.finger.number()) > 4
}

/// Whether the hand left the position it was in.
///
/// Two ways, following Ramoneda et al. (2022, §5.1). The thumb passes under or over,
/// which is a crossing; or the hand moves bodily, which is a shift. A shift is read off
/// the span tables: if the two notes lie outside what those two fingers cover
/// comfortably, the hand was somewhere else for the second one.
fn position_change(
    hand: Hand,
    spans: &SpanTable,
    from: Placement,
    to: Placement,
) -> Option<Change> {
    let semitones = i32::from(to.midi) - i32::from(from.midi);
    if semitones == 0 && from.finger == to.finger {
        return None;
    }
    let fingers = i32::from(to.finger.number()) - i32::from(from.finger.number());
    let crossed = fingers != 0 && outward(hand, semitones).signum() != fingers.signum();
    if crossed && (from.finger == Finger::Thumb || to.finger == Finger::Thumb) {
        return Some(Change::Crossing);
    }
    let span = spans.get(hand, from.finger, to.finger);
    (!span.is_comfortable(semitones)).then_some(Change::Shift)
}

enum Change {
    Crossing,
    Shift,
}

/// One chord: when it sounds, and the notes in it sorted low to high.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chord {
    /// When the chord sounds.
    pub onset_seconds: f64,
    /// Its notes, low to high, which is what lets the edges be read off the ends.
    pub notes: Vec<Placement>,
}

impl Chord {
    /// A chord of one note, for a caller building a test or a melody by hand.
    pub fn single(onset_seconds: f64, note: Placement) -> Self {
        Self { onset_seconds, notes: vec![note] }
    }
}

/// Measure one hand's part.
pub fn measure(hand: Hand, spans: &SpanTable, events: &[Chord]) -> Playability {
    let mut out = Playability::default();
    for pair in events.windows(2) {
        let (Some(from), Some(to)) = (pair[0].notes.first(), pair[1].notes.first()) else {
            continue;
        };
        // Low edge to low edge, and high edge to high edge. A single note has one edge
        // and is counted once; a chord has two, and both have to be makeable.
        let low = (*from, *to);
        let high = (
            *pair[0].notes.last().expect("checked non-empty"),
            *pair[1].notes.last().expect("checked non-empty"),
        );
        let edges = if low == high { vec![low] } else { vec![low, high] };
        for (from, to) in edges {
            out.transitions += 1;
            if impossible(hand, from, to) {
                out.impossible += 1;
                out.flaws.push(Flaw {
                    hand,
                    onset_seconds: pair[1].onset_seconds,
                    from,
                    to,
                    seconds_available: pair[1].onset_seconds - pair[0].onset_seconds,
                });
            }
            match position_change(hand, spans, from, to) {
                Some(Change::Crossing) => out.crossings += 1,
                Some(Change::Shift) => out.shifts += 1,
                None => {}
            }
        }
    }
    out
}

/// Measure a whole fingered score, both hands together.
pub fn measure_solution(score: &Score, solution: &Solution, spans: &SpanTable) -> Playability {
    let mut total = Playability::default();
    for hand in Hand::ALL {
        let events = group(score, solution, hand);
        total.add(measure(hand, spans, &events));
    }
    // Both hands' flaws in one list, in the order they happen, so the piece can be
    // read through once.
    total
        .flaws
        .sort_by(|a, b| a.onset_seconds.total_cmp(&b.onset_seconds));
    total
}

/// Pull one hand's fingered notes out of a score, grouped into chords.
fn group(score: &Score, solution: &Solution, hand: Hand) -> Vec<Chord> {
    let mut notes: Vec<(f64, Placement)> = score
        .notes
        .iter()
        .filter(|n| n.hand == Some(hand))
        .filter_map(|n| {
            solution
                .finger_of(n.id)
                .map(|f| (n.onset_seconds, Placement::new(n.midi, f)))
        })
        .collect();
    notes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.midi.cmp(&b.1.midi)));

    let mut events: Vec<Chord> = Vec::new();
    let mut started = f64::NEG_INFINITY;
    for (onset, placement) in notes {
        match events.last_mut() {
            // Against the onset that opened the chord, not the one before it, so a
            // long roll cannot creep into a chord 30 ms at a time.
            Some(last) if onset - started <= CHORD_SECONDS => last.notes.push(placement),
            _ => {
                started = onset;
                events.push(Chord::single(onset, placement));
            }
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spans::PARNCUTT;

    fn events(notes: &[(u8, u8)]) -> Vec<Chord> {
        notes
            .iter()
            .enumerate()
            .map(|(i, (midi, finger))| {
                Chord::single(
                    i as f64 * 0.25,
                    Placement::new(*midi, Finger::from_number(*finger).expect("1..=5")),
                )
            })
            .collect()
    }

    /// The fingering every pianist is taught for a C major scale has to come out
    /// playable, or the measure is measuring the wrong thing.
    #[test]
    fn a_taught_scale_is_playable() {
        let scale = events(&[
            (60, 1),
            (62, 2),
            (64, 3),
            (65, 1),
            (67, 2),
            (69, 3),
            (71, 4),
            (72, 5),
        ]);
        let out = measure(Hand::Right, &PARNCUTT, &scale);
        assert_eq!(out.impossible, 0, "{out}");
        // One thumb crossing, at the fourth note, and nothing else moves the hand.
        assert_eq!(out.crossings, 1, "{out}");
    }

    /// The case Zhao et al. draw out of their Table 1: with the pitch falling, the
    /// right hand cannot go from its index finger to its middle finger, because the
    /// two would have to cross and neither is the thumb.
    #[test]
    fn non_thumb_fingers_cannot_cross_each_other() {
        let crossed = events(&[(64, 2), (62, 3)]);
        assert_eq!(measure(Hand::Right, &PARNCUTT, &crossed).impossible, 1);

        // The same two notes with the thumb involved is the ordinary crossing, and is
        // counted as a position change rather than as impossible.
        let thumbed = events(&[(64, 2), (62, 1)]);
        let out = measure(Hand::Right, &PARNCUTT, &thumbed);
        assert_eq!(out.impossible, 0, "{out}");
        assert_eq!(out.crossings, 0, "pitch and fingers agree, so nothing crossed");
    }

    /// 1↔5 is the cell that makes the product test worth using rather than a plain
    /// "is either of them the thumb". The thumb does not cross the little finger.
    #[test]
    fn the_thumb_does_not_cross_the_little_finger() {
        let out = measure(Hand::Right, &PARNCUTT, &events(&[(64, 1), (62, 5)]));
        assert_eq!(out.impossible, 1, "{out}");
    }

    /// The left hand is the right hand read backwards, and the measure has to agree:
    /// the same shape mirrored about the keyboard must score identically.
    #[test]
    fn the_left_hand_mirrors_the_right() {
        let right = events(&[(60, 1), (62, 2), (64, 3), (65, 1), (67, 2)]);
        // Reflecting about D4 maps white keys to white and black to black, so the two
        // passages are the same passage to a hand.
        let mirrored: Vec<(u8, u8)> = [(60, 1), (62, 2), (64, 3), (65, 1), (67, 2)]
            .iter()
            .map(|(midi, finger)| (124 - *midi, *finger))
            .collect();
        let left = events(&mirrored);
        assert_eq!(
            measure(Hand::Right, &PARNCUTT, &right),
            measure(Hand::Left, &PARNCUTT, &left)
        );
    }

    /// Past an octave there is no claim to make: the hand has obviously moved, so no
    /// arrangement of fingers is impossible on crossing grounds.
    #[test]
    fn a_leap_beyond_an_octave_is_never_called_impossible() {
        let leap = events(&[(72, 2), (48, 3)]);
        assert_eq!(measure(Hand::Right, &PARNCUTT, &leap).impossible, 0);
    }

    /// Notes struck together are one chord, and the fingers inside it are simultaneous.
    /// Reading them as a sequence would call an ordinary triad a string of position
    /// changes.
    #[test]
    fn a_chord_is_not_a_sequence_of_transitions() {
        let triad = vec![Chord {
            onset_seconds: 0.0,
            notes: vec![
                Placement::new(60, Finger::Thumb),
                Placement::new(64, Finger::Middle),
                Placement::new(67, Finger::Little),
            ],
        }];
        let out = measure(Hand::Right, &PARNCUTT, &triad);
        assert_eq!(out, Playability::default(), "{out}");
    }

    /// A hand that has to jump two octaves has moved, and the measure should say so.
    #[test]
    fn a_leap_counts_as_a_shift() {
        let out = measure(Hand::Right, &PARNCUTT, &events(&[(60, 3), (84, 3)]));
        assert_eq!(out.shifts, 1, "{out}");
        assert_eq!(out.crossings, 0, "{out}");
    }

    /// Repeating one note with one finger is the cheapest thing a hand can do, and
    /// must not be charged as a position change.
    #[test]
    fn a_repeated_note_does_not_move_the_hand() {
        let out = measure(Hand::Right, &PARNCUTT, &events(&[(60, 2), (60, 2), (60, 2)]));
        assert_eq!(out.position_changes(), 0, "{out}");
        assert_eq!(out.transitions, 2, "{out}");
    }

    /// Nothing to measure is not a division by zero.
    #[test]
    fn an_empty_part_reports_zero() {
        let out = measure(Hand::Right, &PARNCUTT, &[]);
        assert_eq!(out.incapable_rate(), 0.0);
        assert_eq!(out.change_rate(), 0.0);
    }
}
