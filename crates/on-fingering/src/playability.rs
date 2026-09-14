//! Judging a fingering with no answer key.
//!
//! Agreement with expert annotations is the measure the literature reports, and it
//! needs a licensed corpus this repository does not ship. These two measures need
//! nothing at all: given a fingering, they ask whether a hand could perform it and how
//! much that hand would have to move. Both can therefore be run on the music actually
//! being rendered, which agreement never can.
//!
//! * **IFR**, the incapable-performing fingering rate of Zhao, Guan & Li (2021, §4.2.2)
//!   — the fraction of note transitions where two fingers other than the thumb would
//!   have to cross. Reported exactly as they define it, so it can be read against
//!   their published figures.
//! * **unplayable** — the subset of those the hand had no time to get out of. This is
//!   the number with a right answer, and the right answer is zero.
//! * **R_pc**, the change position rate of Ramoneda, Jeong, Nakamura, Serra & Miron
//!   (2022, §5.1) — how often the hand leaves the position it was in, either by
//!   crossing the thumb or by shifting bodily. Pianists minimise this, so lower is
//!   better, but only to a point: a fingering that never moves the hand is usually one
//!   that has stopped playing the music. It is read against a reference, not alone.
//! * **travel** — how far the hand actually goes, in metres over the whole piece.
//!   R_pc counts position changes; this measures them. Gao et al. (2023) build their
//!   whole reward function on the principle of minimal motion, and it is the one
//!   ergonomic quantity that adds up over a piece rather than averaging out.
//! * **stretch** — how near its limit the hand is held, averaged over every pair of
//!   fingers in every chord. Zero is a hand at rest and one is a hand at the end of
//!   its reach.
//!
//! Neither replaces agreement. A fingering can be perfectly playable, barely move, and
//! still be nothing a pianist would write. What they catch is the opposite failure —
//! output that scores respectably against annotations while containing a transition
//! that cannot physically be made — and that failure is invisible to a match rate.
//!
//! # Why IFR alone is not the answer
//!
//! Zhao et al. define the crossing test on PIG, which records no durations at all, so
//! their measure cannot ask how long the hand had. It has to assume the worst, and
//! they say so: they count the cases in their own ground truth that the rule wrongly
//! rejects. Applied to real music the assumption bites hard, because most flagged
//! crossings are simply a hand that moved between the two notes.
//!
//! This engine knows the timings, which is the one thing every system built on PIG has
//! had to do without. So each flagged crossing is asked a second question: to uncross
//! the pair, the hand must travel far enough to bring the two fingers back inside what
//! they can span, and at [`HAND_SPEED_MM_PER_SECOND`] that takes a knowable time. If
//! the music allowed it, the crossing is a hand movement and nothing is wrong. If it
//! did not, no hand can do it.
//!
//! Both are reported. IFR for comparison with the literature, and the unplayable count
//! for knowing whether the engine is producing nonsense.
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

use on_hand::keyboard::Keyboard;
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

/// How fast a hand travels along the keyboard at full stretch, in millimetres per
/// second.
///
/// The same calibration the biomechanical model uses for its reconfiguration ceiling:
/// a pianist covers two octaves, 328 mm, in roughly an eighth of a second. That is a
/// ceiling and not a cruising speed, which is the right end to err at here — a measure
/// that calls a fingering impossible should be sure.
pub const HAND_SPEED_MM_PER_SECOND: f64 = 2.0 * 164.0 / 0.125;

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
    pub seconds_available: f64,
    /// How long it would need, to travel far enough to uncross the pair.
    pub seconds_needed: f64,
}

impl Flaw {
    /// Whether the hand had no time to get out of its own way.
    ///
    /// A crossing this is false for is not a fault at all: it is a hand that moved,
    /// which the crossing test on its own cannot see.
    pub fn unplayable(&self) -> bool {
        self.seconds_needed > self.seconds_available
    }
}

impl std::fmt::Display for Flaw {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:>8.2}s  {:<5} {:>4} {} -> {:<4} {}   {:>4.0} ms to move, needs {:.0}{}",
            self.onset_seconds,
            format!("{:?}", self.hand).to_lowercase(),
            on_hand::keyboard::name(self.from.midi),
            self.from.finger.number(),
            on_hand::keyboard::name(self.to.midi),
            self.to.finger.number(),
            self.seconds_available * 1000.0,
            self.seconds_needed * 1000.0,
            if self.unplayable() { "  UNPLAYABLE" } else { "" },
        )
    }
}

/// What a fingering costs a hand, measured against nothing but itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Playability {
    /// Transitions where two fingers other than the thumb would have to cross.
    pub impossible: usize,
    /// Those of them the hand had no time to move out of.
    pub unplayable: usize,
    /// Each of them, in time order, for a caller that wants to go and look.
    pub flaws: Vec<Flaw>,
    /// Position changes made by passing the thumb.
    pub crossings: usize,
    /// Position changes made by moving the hand bodily.
    pub shifts: usize,
    /// Transitions examined, which is what the rates are over.
    pub transitions: usize,
    /// How far the hand travelled in total, in millimetres.
    pub travel_mm: f64,
    /// Summed span stress, over [`Playability::spans_measured`] finger pairs.
    pub stretch_total: f64,
    /// How many finger pairs that was, so the mean can be taken.
    pub spans_measured: usize,
    /// Pairs held wider or narrower than the hand finds comfortable.
    pub stretched: usize,
}

impl Playability {
    /// The incapable-performing fingering rate, as Zhao et al. define it.
    ///
    /// Comparable with their published figures, and generous with false positives for
    /// the reason given in the module documentation. For the question of whether the
    /// output is sound, use [`Playability::unplayable_rate`].
    pub fn incapable_rate(&self) -> f32 {
        rate(self.impossible, self.transitions)
    }

    /// The fraction of transitions no hand could make in the time the music allows.
    ///
    /// Zero is the only acceptable value. Anything else is either a bug in the search
    /// or music that cannot be played by two hands as the parts have been assigned.
    pub fn unplayable_rate(&self) -> f32 {
        rate(self.unplayable, self.transitions)
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

    /// How near its limit the hand is held, averaged over every finger pair in every
    /// chord.
    ///
    /// Zero is a hand at rest; one is a hand at the limit of what the span tables say
    /// it can be forced to. Above one means the music is asking for more than that,
    /// which the reachability model charges for separately.
    pub fn mean_stretch(&self) -> f32 {
        if self.spans_measured == 0 {
            0.0
        } else {
            (self.stretch_total / self.spans_measured as f64) as f32
        }
    }

    /// The fraction of finger pairs held outside the comfortable span.
    ///
    /// The "large-span ratio" of the ergonomic literature. A passage of tenths will
    /// have a high one honestly; it is for comparing two fingerings of the same music.
    pub fn stretched_rate(&self) -> f32 {
        rate(self.stretched, self.spans_measured)
    }

    /// Add another hand's, or another piece's, tally to this one.
    pub fn add(&mut self, other: Playability) {
        self.impossible += other.impossible;
        self.unplayable += other.unplayable;
        self.flaws.extend(other.flaws);
        self.crossings += other.crossings;
        self.shifts += other.shifts;
        self.transitions += other.transitions;
        self.travel_mm += other.travel_mm;
        self.stretch_total += other.stretch_total;
        self.spans_measured += other.spans_measured;
        self.stretched += other.stretched;
    }
}

impl std::fmt::Display for Playability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unplayable {} ({:.2}%)   IFR {:.2}%   position changes {:.1}% \
             ({} crossing, {} shift, {} transitions)",
            self.unplayable,
            self.unplayable_rate() * 100.0,
            self.incapable_rate() * 100.0,
            self.change_rate() * 100.0,
            self.crossings,
            self.shifts,
            self.transitions
        )?;
        write!(
            f,
            "\n  travel {:.1} m   stretch {:.2} (mean), {:.1}% beyond comfortable",
            self.travel_mm / 1000.0,
            self.mean_stretch(),
            self.stretched_rate() * 100.0,
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

/// Where the hand must be for a finger to reach a note, in millimetres along the
/// keyboard.
///
/// Gao et al. (2023, §3.4.2) estimate it by projecting back from the played key to
/// where the middle finger would sit, on the reasoning that the hand covers several
/// keys and the middle finger is as good a point to call its centre as any. They step
/// one white key per finger number; the span tables here say what the real offset is,
/// so the midpoint of the relaxed span from the middle finger to the playing one is
/// used instead. It is the same idea measured rather than assumed.
///
/// The table already mirrors itself for the left hand, so one formula serves both.
fn hand_position_mm(hand: Hand, spans: &SpanTable, keys: &Keyboard, at: Placement) -> f64 {
    let span = spans.get(hand, Finger::Middle, at.finger);
    let natural = f64::from(span.min_rel + span.max_rel) / 2.0;
    f64::from(keys.centre_x(at.midi)) - natural * f64::from(crate::ruler::SEMITONE_MM)
}

/// Where the hand is while playing a chord.
///
/// The average of what the lowest and the highest note each imply, which is Gao et
/// al.'s rule and the obvious one: a hand holding a shape is centred between its ends.
fn chord_position_mm(hand: Hand, spans: &SpanTable, keys: &Keyboard, chord: &Chord) -> Option<f64> {
    let low = chord.notes.first()?;
    let high = chord.notes.last()?;
    Some(
        (hand_position_mm(hand, spans, keys, *low) + hand_position_mm(hand, spans, keys, *high))
            / 2.0,
    )
}

/// How near its limit a pair of fingers is being held, as a fraction.
///
/// Zero anywhere inside the relaxed range, rising to one at the edge of what the pair
/// can be forced to, and beyond one for a span the tables say is not available at all.
/// Both directions count: a hand bunched too narrow is working as surely as one spread
/// too wide, which is why the span tables carry a minimum as well as a maximum.
fn span_stress(span: crate::spans::Span, semitones: i32) -> f64 {
    let (rel, prac) = if semitones > span.max_rel {
        (span.max_rel, span.max_prac)
    } else if semitones < span.min_rel {
        (span.min_rel, span.min_prac)
    } else {
        return 0.0;
    };
    let room = f64::from((prac - rel).abs());
    if room <= 0.0 {
        // The pair has no give at all in this direction, so any departure is total.
        return 1.0;
    }
    f64::from((semitones - rel).abs()) / room
}

/// How long the hand needs to uncross a pair, in seconds.
///
/// To play the second note with the second finger, the hand has to be somewhere that
/// puts the two inside what the pair can span. The shortfall is how far outside that
/// span the interval falls, which the tables already answer in semitones, and a
/// semitone is a known number of millimetres. Dividing by the speed a hand can travel
/// gives the time — a lower bound, since it assumes the hand is already moving at its
/// ceiling and has only to translate.
///
/// A pair the tables say can span the interval needs no time at all, and is therefore
/// never unplayable however fast it comes. That is how the two sources are reconciled
/// where they disagree: Zhao et al.'s product test rejects a thumb crossing the little
/// finger, while Parncutt's measured tables give the pair a minimum practical span of
/// -1 and so permit it. The measured tables win on what a hand can do; the product
/// test still reports it, so the IFR figure stays theirs.
fn seconds_to_uncross(hand: Hand, spans: &SpanTable, from: Placement, to: Placement) -> f64 {
    let semitones = i32::from(to.midi) - i32::from(from.midi);
    let span = spans.get(hand, from.finger, to.finger);
    let shortfall = f64::from(span.impracticality(semitones));
    shortfall * f64::from(crate::ruler::SEMITONE_MM) / HAND_SPEED_MM_PER_SECOND
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
    let keys = Keyboard::new();

    // How stretched the hand is holding each chord. Every pair of fingers in it, since
    // a chord is uncomfortable if any two of its fingers are.
    for chord in events {
        for (i, low) in chord.notes.iter().enumerate() {
            for high in &chord.notes[i + 1..] {
                let semitones = i32::from(high.midi) - i32::from(low.midi);
                let span = spans.get(hand, low.finger, high.finger);
                out.stretch_total += span_stress(span, semitones);
                out.spans_measured += 1;
                if !span.is_comfortable(semitones) {
                    out.stretched += 1;
                }
            }
        }
    }

    for pair in events.windows(2) {
        if let (Some(from), Some(to)) = (
            chord_position_mm(hand, spans, &keys, &pair[0]),
            chord_position_mm(hand, spans, &keys, &pair[1]),
        ) {
            out.travel_mm += (to - from).abs();
        }
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
                let flaw = Flaw {
                    hand,
                    onset_seconds: pair[1].onset_seconds,
                    from,
                    to,
                    seconds_available: pair[1].onset_seconds - pair[0].onset_seconds,
                    seconds_needed: seconds_to_uncross(hand, spans, from, to),
                };
                if flaw.unplayable() {
                    out.unplayable += 1;
                }
                out.flaws.push(flaw);
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



    /// A hand playing in one position barely travels; a hand thrown around the
    /// keyboard travels a long way. If the measure cannot tell those apart it is
    /// measuring nothing.
    #[test]
    fn travel_tracks_how_far_the_hand_actually_goes() {
        // Five notes under one hand position, taken with consecutive fingers.
        let settled = events(&[(60, 1), (62, 2), (64, 3), (65, 4), (67, 5)]);
        // The same five fingers, but flung across three octaves.
        let flung = events(&[(60, 1), (84, 2), (48, 3), (84, 4), (60, 5)]);

        let a = measure(Hand::Right, &PARNCUTT, &settled);
        let b = measure(Hand::Right, &PARNCUTT, &flung);
        assert!(
            b.travel_mm > 5.0 * a.travel_mm,
            "settled {:.0} mm, flung {:.0} mm",
            a.travel_mm,
            b.travel_mm
        );
        // A hand in one position still shifts a little as the fingers take their notes.
        assert!(a.travel_mm < 200.0, "{:.0} mm is not one position", a.travel_mm);
    }

    /// The hand position estimate has to be a property of the hand, not of the note.
    ///
    /// A fifth played 1-5 and the same fifth played by a hand reaching up with 1-2 put
    /// the hand in different places, and that is the whole point of projecting back
    /// from the played key rather than just using the key.
    #[test]
    fn the_hand_is_not_where_the_note_is() {
        let keys = Keyboard::new();
        let thumb = hand_position_mm(Hand::Right, &PARNCUTT, &keys, Placement::new(60, Finger::Thumb));
        let little = hand_position_mm(Hand::Right, &PARNCUTT, &keys, Placement::new(60, Finger::Little));
        assert!(
            thumb > little,
            "playing C with the thumb puts the hand above it, and with the little \
             finger below: {thumb:.0} vs {little:.0}"
        );
        // The middle finger is the reference, so it sits on its own note.
        let middle = hand_position_mm(Hand::Right, &PARNCUTT, &keys, Placement::new(60, Finger::Middle));
        assert!((middle - f64::from(keys.centre_x(60))).abs() < 1.0, "{middle}");
    }

    /// Stretch is zero for a hand at rest and rises as the span opens, in both
    /// directions: bunched is work too.
    #[test]
    fn stretch_rises_away_from_the_resting_hand() {
        let pair = |low: u8, high: u8, a: u8, b: u8| {
            let chord = vec![Chord {
                onset_seconds: 0.0,
                notes: vec![
                    Placement::new(low, Finger::from_number(a).unwrap()),
                    Placement::new(high, Finger::from_number(b).unwrap()),
                ],
            }];
            measure(Hand::Right, &PARNCUTT, &chord).mean_stretch()
        };

        // Parncutt puts the relaxed range for thumb against little finger at 7 to 10
        // semitones, which is worth knowing: an octave taken 1-5 is already a slight
        // stretch, and a ninth more so. The tables decide this, not intuition.
        let resting = pair(60, 69, 1, 5);
        let octave = pair(60, 72, 1, 5);
        let reaching = pair(60, 74, 1, 5);
        assert_eq!(resting, 0.0, "a sixth 1-5 is a hand at rest, got {resting}");
        assert!(octave > 0.0, "an octave 1-5 is past relaxed, got {octave}");
        assert!(reaching > octave, "a ninth reaches further, got {reaching}");

        // And squeezed the other way: those two fingers on neighbouring keys.
        let bunched = pair(60, 61, 1, 5);
        assert!(bunched > 0.0, "1-5 on a semitone is a bunched hand, got {bunched}");
    }

    /// The measure has to stay capable of saying no, or it is worth nothing.
    ///
    /// The same crossing, twice: once with the notes far enough apart that the hand
    /// can be somewhere else by the second one, and once with them almost together.
    /// Both are crossings and both are counted in the IFR, because that is how Zhao et
    /// al. define it. Only the second is a fingering no hand can play.
    #[test]
    fn a_crossing_is_unplayable_only_when_the_hand_had_no_time() {
        // The shape the measure found in real music: a left hand leaving G sharp with
        // the little finger and wanting C an octave-ish below with the ring finger,
        // which needs the whole hand somewhere else.
        let crossed = |seconds: f64| {
            vec![
                Chord::single(0.0, Placement::new(44, Finger::Little)),
                Chord::single(seconds, Placement::new(36, Finger::Ring)),
            ]
        };

        let hurried = measure(Hand::Left, &PARNCUTT, &crossed(0.038));
        assert_eq!(hurried.impossible, 1, "{hurried}");
        assert_eq!(hurried.unplayable, 1, "{hurried}");

        let unhurried = measure(Hand::Left, &PARNCUTT, &crossed(1.0));
        assert_eq!(unhurried.impossible, 1, "still a crossing: {unhurried}");
        assert_eq!(
            unhurried.unplayable, 0,
            "a second is long enough to carry a hand anywhere: {unhurried}"
        );
    }

    /// Where the two published sources disagree, the measured span tables decide what a
    /// hand can do and the heuristic only decides what gets reported.
    ///
    /// Zhao et al.'s product test rejects the thumb crossing the little finger, since
    /// 1 times 5 exceeds their threshold. Parncutt's Table 1 gives that pair a minimum
    /// practical span of -1, which is to say it was measured and it is possible. So it
    /// is counted in the IFR, for comparability with their figures, and never called
    /// unplayable.
    #[test]
    fn the_measured_tables_outrank_the_heuristic() {
        let fast = vec![
            Chord::single(0.0, Placement::new(64, Finger::Thumb)),
            Chord::single(0.01, Placement::new(63, Finger::Little)),
        ];
        let out = measure(Hand::Right, &PARNCUTT, &fast);
        assert_eq!(out.impossible, 1, "Zhao et al. reject it: {out}");
        assert_eq!(out.unplayable, 0, "Parncutt measured it: {out}");
    }

    /// The time a hand needs scales with how far out of position it is, so a wider
    /// crossing needs longer. Without that the measure would be a fixed threshold
    /// wearing a physical argument as a disguise.
    #[test]
    fn a_wider_crossing_needs_longer() {
        let gap = |to: u8| {
            let events = vec![
                Chord::single(0.0, Placement::new(60, Finger::Little)),
                Chord::single(0.05, Placement::new(to, Finger::Ring)),
            ];
            measure(Hand::Left, &PARNCUTT, &events)
                .flaws
                .first()
                .map(|f| f.seconds_needed)
        };
        let (near, far) = (gap(58), gap(52));
        assert!(near.is_some() && far.is_some(), "{near:?} {far:?}");
        assert!(
            far > near,
            "six semitones out of position needs no longer than two: {far:?} vs {near:?}"
        );
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
        assert_eq!(out.transitions, 0, "{out}");
        assert_eq!(out.position_changes(), 0, "{out}");
        assert_eq!(out.impossible, 0, "{out}");
        assert_eq!(out.travel_mm, 0.0, "one chord, so the hand never moves: {out}");
        // It is still a shape the hand holds, and the three pairs in it are measured.
        assert_eq!(out.spans_measured, 3, "{out}");
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
