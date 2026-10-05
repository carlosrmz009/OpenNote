use on_hand::keyboard::Keyboard;
use on_hand::{Finger, Hand};
use on_score::Score;

use crate::rules::Placement;
use crate::solver::Solution;
use crate::spans::SpanTable;

pub const CHORD_SECONDS: f64 = 0.030;

pub const HAND_SPEED_MM_PER_SECOND: f64 = 2.0 * 164.0 / 0.125;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flaw {
    pub hand: Hand,
    pub onset_seconds: f64,
    pub from: Placement,
    pub to: Placement,
    pub seconds_available: f64,
    pub seconds_needed: f64,
}

impl Flaw {
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Playability {
    pub impossible: usize,
    pub unplayable: usize,
    pub flaws: Vec<Flaw>,
    pub crossings: usize,
    pub shifts: usize,
    pub transitions: usize,
    pub travel_mm: f64,
    pub stretch_total: f64,
    pub spans_measured: usize,
    pub stretched: usize,
    pub thumb_unders: usize,
    pub finger_overs: usize,
    pub thumbless_crossings: usize,
    pub step_spread_total: f64,
    pub steps_measured: usize,
    pub chord_spread_total: f64,
    pub chord_pairs: usize,
    pub weak_on_black: usize,
    pub notes: usize,
}

impl Playability {
    pub fn incapable_rate(&self) -> f32 {
        rate(self.impossible, self.transitions)
    }

    pub fn unplayable_rate(&self) -> f32 {
        rate(self.unplayable, self.transitions)
    }

    pub fn position_changes(&self) -> usize {
        self.crossings + self.shifts
    }

    pub fn change_rate(&self) -> f32 {
        rate(self.position_changes(), self.transitions)
    }

    pub fn mean_stretch(&self) -> f32 {
        if self.spans_measured == 0 {
            0.0
        } else {
            (self.stretch_total / self.spans_measured as f64) as f32
        }
    }

    pub fn stretched_rate(&self) -> f32 {
        rate(self.stretched, self.spans_measured)
    }

    pub fn thumb_under_rate(&self) -> f32 {
        rate(self.thumb_unders, self.transitions)
    }

    pub fn finger_over_rate(&self) -> f32 {
        rate(self.finger_overs, self.transitions)
    }

    pub fn thumbless_rate(&self) -> f32 {
        rate(self.thumbless_crossings, self.transitions)
    }

    pub fn step_spread(&self) -> f32 {
        if self.steps_measured == 0 { 0.0 } else { (self.step_spread_total / self.steps_measured as f64) as f32 }
    }

    pub fn chord_spread(&self) -> f32 {
        if self.chord_pairs == 0 { 0.0 } else { (self.chord_spread_total / self.chord_pairs as f64) as f32 }
    }

    pub fn weak_on_black_rate(&self) -> f32 {
        rate(self.weak_on_black, self.notes)
    }

    pub fn add(&mut self, other: Playability) {
        self.thumb_unders += other.thumb_unders;
        self.finger_overs += other.finger_overs;
        self.thumbless_crossings += other.thumbless_crossings;
        self.step_spread_total += other.step_spread_total;
        self.steps_measured += other.steps_measured;
        self.chord_spread_total += other.chord_spread_total;
        self.chord_pairs += other.chord_pairs;
        self.weak_on_black += other.weak_on_black;
        self.notes += other.notes;
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

fn outward(hand: Hand, semitones: i32) -> i32 {
    match hand {
        Hand::Right => semitones,
        Hand::Left => -semitones,
    }
}

fn impossible(hand: Hand, from: Placement, to: Placement) -> bool {
    let semitones = i32::from(to.midi) - i32::from(from.midi);
    if semitones == 0 || semitones.abs() >= 12 {
        return false;
    }
    let fingers = i32::from(to.finger.number()) - i32::from(from.finger.number());
    if fingers == 0 {
        return false;
    }
    if outward(hand, semitones).signum() == fingers.signum() {
        return false;
    }
    i32::from(from.finger.number()) * i32::from(to.finger.number()) > 4
}

fn hand_position_mm(hand: Hand, spans: &SpanTable, keys: &Keyboard, at: Placement) -> f64 {
    let span = spans.get(hand, Finger::Middle, at.finger);
    let natural = f64::from(span.min_rel + span.max_rel) / 2.0;
    f64::from(keys.centre_x(at.midi)) - natural * f64::from(crate::ruler::SEMITONE_MM)
}

fn chord_position_mm(hand: Hand, spans: &SpanTable, keys: &Keyboard, chord: &Chord) -> Option<f64> {
    let low = chord.notes.first()?;
    let high = chord.notes.last()?;
    Some(
        (hand_position_mm(hand, spans, keys, *low) + hand_position_mm(hand, spans, keys, *high))
            / 2.0,
    )
}

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
        return 1.0;
    }
    f64::from((semitones - rel).abs()) / room
}

fn seconds_to_uncross(hand: Hand, spans: &SpanTable, from: Placement, to: Placement) -> f64 {
    let semitones = i32::from(to.midi) - i32::from(from.midi);
    let span = spans.get(hand, from.finger, to.finger);
    let shortfall = f64::from(span.impracticality(semitones));
    shortfall * f64::from(crate::ruler::SEMITONE_MM) / HAND_SPEED_MM_PER_SECOND
}

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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Chord {
    pub onset_seconds: f64,
    pub notes: Vec<Placement>,
}

impl Chord {
    pub fn single(onset_seconds: f64, note: Placement) -> Self {
        Self { onset_seconds, notes: vec![note] }
    }
}

pub fn measure(hand: Hand, spans: &SpanTable, events: &[Chord]) -> Playability {
    let mut out = Playability::default();
    let keys = Keyboard::new();

    for chord in events {
        out.notes += chord.notes.len();
        out.weak_on_black += chord
            .notes
            .iter()
            .filter(|n| n.finger.number() >= 4 && on_hand::keyboard::is_black(n.midi))
            .count();
        for pair in chord.notes.windows(2) {
            let fingers = (i32::from(pair[1].finger.number()) - i32::from(pair[0].finger.number())).abs();
            if fingers > 0 {
                out.chord_spread_total += f64::from(i32::from(pair[1].midi) - i32::from(pair[0].midi)).abs() / f64::from(fingers);
                out.chord_pairs += 1;
            }
        }
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
        let low = (*from, *to);
        let high = (
            *pair[0].notes.last().expect("checked non-empty"),
            *pair[1].notes.last().expect("checked non-empty"),
        );
        let edges = if low == high { vec![low] } else { vec![low, high] };
        for (from, to) in edges {
            out.transitions += 1;
            tally_crossing(&mut out, hand, from, to);
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

fn tally_crossing(out: &mut Playability, hand: Hand, from: Placement, to: Placement) {
    let up = i32::from(to.midi) - i32::from(from.midi);
    let (a, b) = (from.finger.number(), to.finger.number());
    if up == 0 || a == b {
        return;
    }
    out.step_spread_total += f64::from(up.abs()) / f64::from((i32::from(b) - i32::from(a)).abs());
    out.steps_measured += 1;
    let rising = (b > a) == (up > 0);
    let natural = match hand {
        Hand::Right => rising,
        Hand::Left => !rising,
    };
    if natural || up.abs() >= 12 {
        return;
    }
    if b == 1 {
        out.thumb_unders += 1;
    } else if a == 1 {
        out.finger_overs += 1;
    } else {
        out.thumbless_crossings += 1;
    }
}

pub fn measure_solution(score: &Score, solution: &Solution, spans: &SpanTable) -> Playability {
    let mut total = Playability::default();
    for hand in Hand::ALL {
        let events = group(score, solution, hand);
        total.add(measure(hand, spans, &events));
    }
    total
        .flaws
        .sort_by(|a, b| a.onset_seconds.total_cmp(&b.onset_seconds));
    total
}

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

    #[test]
    fn travel_tracks_how_far_the_hand_actually_goes() {
        let settled = events(&[(60, 1), (62, 2), (64, 3), (65, 4), (67, 5)]);
        let flung = events(&[(60, 1), (84, 2), (48, 3), (84, 4), (60, 5)]);

        let a = measure(Hand::Right, &PARNCUTT, &settled);
        let b = measure(Hand::Right, &PARNCUTT, &flung);
        assert!(
            b.travel_mm > 5.0 * a.travel_mm,
            "settled {:.0} mm, flung {:.0} mm",
            a.travel_mm,
            b.travel_mm
        );
        assert!(a.travel_mm < 200.0, "{:.0} mm is not one position", a.travel_mm);
    }

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
        let middle = hand_position_mm(Hand::Right, &PARNCUTT, &keys, Placement::new(60, Finger::Middle));
        assert!((middle - f64::from(keys.centre_x(60))).abs() < 1.0, "{middle}");
    }

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

        let resting = pair(60, 69, 1, 5);
        let octave = pair(60, 72, 1, 5);
        let reaching = pair(60, 74, 1, 5);
        assert_eq!(resting, 0.0, "a sixth 1-5 is a hand at rest, got {resting}");
        assert!(octave > 0.0, "an octave 1-5 is past relaxed, got {octave}");
        assert!(reaching > octave, "a ninth reaches further, got {reaching}");

        let bunched = pair(60, 61, 1, 5);
        assert!(bunched > 0.0, "1-5 on a semitone is a bunched hand, got {bunched}");
    }

    #[test]
    fn a_crossing_is_unplayable_only_when_the_hand_had_no_time() {
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
        assert_eq!(out.crossings, 1, "{out}");
    }

    #[test]
    fn the_thumb_passes_under_going_one_way_and_a_finger_over_coming_back() {
        let up = events(&[(60, 1), (62, 2), (64, 3), (65, 1), (67, 2)]);
        let right = measure(Hand::Right, &PARNCUTT, &up);
        assert_eq!((right.thumb_unders, right.finger_overs, right.thumbless_crossings), (1, 0, 0));
        let down = events(&[(67, 2), (65, 1), (64, 3), (62, 2), (60, 1)]);
        let right = measure(Hand::Right, &PARNCUTT, &down);
        assert_eq!((right.thumb_unders, right.finger_overs), (0, 1));
        let left_up = events(&[(48, 5), (50, 4), (52, 3), (53, 2), (55, 1), (57, 3), (59, 2)]);
        let left = measure(Hand::Left, &PARNCUTT, &left_up);
        assert_eq!((left.thumb_unders, left.finger_overs), (0, 1));
        let left_down = events(&[(59, 2), (57, 3), (55, 1), (53, 2)]);
        assert_eq!(measure(Hand::Left, &PARNCUTT, &left_down).thumb_unders, 1);
    }

    #[test]
    fn a_crossing_without_the_thumb_is_counted_and_a_leap_is_not() {
        let crossed = events(&[(64, 2), (62, 3)]);
        assert_eq!(measure(Hand::Right, &PARNCUTT, &crossed).thumbless_crossings, 1);
        let leap = events(&[(76, 2), (62, 3)]);
        assert_eq!(measure(Hand::Right, &PARNCUTT, &leap).thumbless_crossings, 0, "fourteen semitones is a jump");
    }

    #[test]
    fn spread_and_weak_fingers_on_black_keys() {
        let line = events(&[(60, 1), (67, 2), (61, 4)]);
        let out = measure(Hand::Right, &PARNCUTT, &line);
        assert!((out.step_spread_total - (7.0 + 6.0 / 2.0)).abs() < 1e-9, "{}", out.step_spread_total);
        assert_eq!(out.weak_on_black, 1, "the fourth finger on C sharp");
        assert_eq!(out.notes, 3);
    }

    #[test]
    fn non_thumb_fingers_cannot_cross_each_other() {
        let crossed = events(&[(64, 2), (62, 3)]);
        assert_eq!(measure(Hand::Right, &PARNCUTT, &crossed).impossible, 1);

        let thumbed = events(&[(64, 2), (62, 1)]);
        let out = measure(Hand::Right, &PARNCUTT, &thumbed);
        assert_eq!(out.impossible, 0, "{out}");
        assert_eq!(out.crossings, 0, "pitch and fingers agree, so nothing crossed");
    }

    #[test]
    fn the_thumb_does_not_cross_the_little_finger() {
        let out = measure(Hand::Right, &PARNCUTT, &events(&[(64, 1), (62, 5)]));
        assert_eq!(out.impossible, 1, "{out}");
    }

    #[test]
    fn the_left_hand_mirrors_the_right() {
        let right = events(&[(60, 1), (62, 2), (64, 3), (65, 1), (67, 2)]);
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

    #[test]
    fn a_leap_beyond_an_octave_is_never_called_impossible() {
        let leap = events(&[(72, 2), (48, 3)]);
        assert_eq!(measure(Hand::Right, &PARNCUTT, &leap).impossible, 0);
    }

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
        assert_eq!(out.spans_measured, 3, "{out}");
    }

    #[test]
    fn a_leap_counts_as_a_shift() {
        let out = measure(Hand::Right, &PARNCUTT, &events(&[(60, 3), (84, 3)]));
        assert_eq!(out.shifts, 1, "{out}");
        assert_eq!(out.crossings, 0, "{out}");
    }

    #[test]
    fn a_repeated_note_does_not_move_the_hand() {
        let out = measure(Hand::Right, &PARNCUTT, &events(&[(60, 2), (60, 2), (60, 2)]));
        assert_eq!(out.position_changes(), 0, "{out}");
        assert_eq!(out.transitions, 2, "{out}");
    }

    #[test]
    fn an_empty_part_reports_zero() {
        let out = measure(Hand::Right, &PARNCUTT, &[]);
        assert_eq!(out.incapable_rate(), 0.0);
        assert_eq!(out.change_rate(), 0.0);
    }
}
