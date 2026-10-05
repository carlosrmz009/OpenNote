use on_hand::keyboard::Keyboard;
use on_hand::profile::HandProfile;
use on_hand::Hand;

use crate::{NoteId, Score, Ticks};

#[derive(Debug, Clone)]
pub struct HandAssignment {
    pub profile: HandProfile,
    pub overspan_weight: f32,
    pub travel_weight: f32,
    pub register_weight: f32,
    pub abandon_weight: f32,
    pub pivot_window_seconds: f64,
    pub one_hand_fraction: f32,
}

impl Default for HandAssignment {
    fn default() -> Self {
        Self {
            profile: HandProfile::default(),
            overspan_weight: 0.76,
            travel_weight: 0.00004,
            register_weight: 26.0,
            abandon_weight: 0.004,
            pivot_window_seconds: 1.5,
            one_hand_fraction: 0.5,
        }
    }
}

pub fn assign_hands(score: &mut Score, options: &HandAssignment) {
    if assign_from_staves(score) {
        return;
    }
    assign_by_search(score, options);
}

pub fn truth_hands(score: &Score) -> Option<Vec<Option<Hand>>> {
    if score.notes.iter().all(|n| n.staff.is_some()) {
        let mut staves: Vec<u8> = score.notes.iter().filter_map(|n| n.staff).collect();
        staves.sort_unstable();
        staves.dedup();
        if staves.len() == 2 {
            let upper = staves[0];
            return Some(
                score
                    .notes
                    .iter()
                    .map(|n| Some(if n.staff == Some(upper) { Hand::Right } else { Hand::Left }))
                    .collect(),
            );
        }
    }

    let track_of = |note: &crate::Note| match note.source {
        crate::SourceRef::Midi { track, .. } => Some(track),
        _ => None,
    };
    let all: Vec<usize> = score.notes.iter().filter_map(track_of).collect();
    if all.is_empty() || all.len() != score.notes.len() {
        return None;
    }
    let mut seen = all.clone();
    seen.sort_unstable();
    seen.dedup();
    let tracks: Vec<usize> = seen
        .into_iter()
        .filter(|track| all.iter().filter(|t| *t == track).count() * 100 >= all.len())
        .collect();
    if tracks.len() != 2 {
        return None;
    }
    let mean = |track: usize| {
        let pitches: Vec<f32> = score
            .notes
            .iter()
            .filter(|n| track_of(n) == Some(track))
            .map(|n| f32::from(n.midi))
            .collect();
        pitches.iter().sum::<f32>() / pitches.len() as f32
    };
    let right = if mean(tracks[0]) > mean(tracks[1]) { tracks[0] } else { tracks[1] };
    Some(
        score
            .notes
            .iter()
            .map(|n| match track_of(n) {
                Some(t) if t == right => Some(Hand::Right),
                Some(t) if tracks.contains(&t) => Some(Hand::Left),
                _ => None,
            })
            .collect(),
    )
}

pub fn agreement(score: &Score, options: &HandAssignment) -> Option<(usize, usize)> {
    let truth = truth_hands(score)?;
    let mut blind = score.clone();
    for note in &mut blind.notes {
        note.staff = None;
        note.hand = None;
    }
    assign_by_search(&mut blind, options);
    let (mut right, mut judged) = (0, 0);
    for (note, want) in blind.notes.iter().zip(&truth) {
        let Some(want) = want else { continue };
        judged += 1;
        right += usize::from(note.hand == Some(*want));
    }
    Some((right, judged))
}

fn assign_from_staves(score: &mut Score) -> bool {
    let mut staves: Vec<u8> = score.notes.iter().filter_map(|n| n.staff).collect();
    if staves.len() != score.notes.len() {
        return false;
    }
    staves.sort_unstable();
    staves.dedup();
    if staves.len() != 2 {
        return false;
    }

    let (upper, _lower) = (staves[0], staves[1]);
    for note in &mut score.notes {
        note.hand = Some(if note.staff == Some(upper) {
            Hand::Right
        } else {
            Hand::Left
        });
    }
    true
}

struct Simultaneity {
    onset: Ticks,
    onset_seconds: f64,
    notes: Vec<NoteId>,
    pitches: Vec<u8>,
    until: Vec<f64>,
    pivot: f32,
}

fn assign_by_search(score: &mut Score, options: &HandAssignment) {
    let mut events = collect_simultaneities(score);
    if events.is_empty() {
        return;
    }
    hold_sustained(&mut events);
    set_pivots(&mut events, options);
    let pedal: Vec<(f64, f64)> = score
        .pedal
        .iter()
        .map(|(down, up)| (score.tempo.seconds_at(*down), score.tempo.seconds_at(*up)))
        .collect();
    let footed = |at: f64| pedal.iter().any(|(down, up)| *down <= at && at < *up);

    let keyboard = Keyboard::new();
    let max_span = options.profile.comfortable_span_mm();

    let mut costs: Vec<Vec<f32>> = Vec::with_capacity(events.len());
    let mut back: Vec<Vec<usize>> = Vec::with_capacity(events.len());

    for (i, event) in events.iter().enumerate() {
        let splits = event.pitches.len() + 1;
        let mut row = vec![f32::INFINITY; splits];
        let mut from = vec![0usize; splits];

        for split in 0..splits {
            let local = split_cost(event, split, &keyboard, max_span, options);
            if i == 0 {
                row[split] = local;
                continue;
            }
            let previous = &events[i - 1];
            let gap = (event.onset_seconds - previous.onset_seconds).max(0.02);
            for (prev_split, prev_cost) in costs[i - 1].iter().enumerate() {
                if !prev_cost.is_finite() {
                    continue;
                }
                let move_cost = travel_cost(
                    previous,
                    prev_split,
                    event,
                    split,
                    &keyboard,
                    gap,
                    max_span,
                    options,
                    footed(event.onset_seconds),
                );
                let total = prev_cost + local + move_cost;
                if total < row[split] {
                    row[split] = total;
                    from[split] = prev_split;
                }
            }
        }
        costs.push(row);
        back.push(from);
    }

    let mut split = costs
        .last()
        .unwrap()
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
        .unwrap_or(0);

    for i in (0..events.len()).rev() {
        let event = &events[i];
        for (j, id) in event.notes.iter().enumerate() {
            let hand = if j < split { Hand::Left } else { Hand::Right };
            score.note_mut(*id).hand = Some(hand);
        }
        if i > 0 {
            split = back[i][split];
        }
    }
}

fn hold_sustained(events: &mut [Simultaneity]) {
    let mut sounding: Vec<(NoteId, u8, f64)> = Vec::new();
    for index in 0..events.len() {
        let now = events[index].onset_seconds;
        sounding.retain(|(_, _, until)| *until > now + 1e-6);

        let struck: Vec<(NoteId, u8, f64)> = events[index]
            .notes
            .iter()
            .zip(&events[index].pitches)
            .zip(&events[index].until)
            .map(|((id, midi), until)| (*id, *midi, *until))
            .collect();

        for (id, midi, until) in &sounding {
            if struck.iter().any(|(other, _, _)| other == id) {
                continue;
            }
            events[index].notes.push(*id);
            events[index].pitches.push(*midi);
            events[index].until.push(*until);
        }
        sort_by_pitch(&mut events[index]);

        sounding.extend(struck);
    }
}

fn sort_by_pitch(event: &mut Simultaneity) {
    let mut rows: Vec<(NoteId, u8, f64)> = event
        .notes
        .iter()
        .zip(&event.pitches)
        .zip(&event.until)
        .map(|((id, midi), until)| (*id, *midi, *until))
        .collect();
    rows.sort_by_key(|(_, midi, _)| *midi);
    event.notes = rows.iter().map(|(id, _, _)| *id).collect();
    event.pitches = rows.iter().map(|(_, midi, _)| *midi).collect();
    event.until = rows.iter().map(|(_, _, until)| *until).collect();
}

fn collect_simultaneities(score: &Score) -> Vec<Simultaneity> {
    let mut out: Vec<Simultaneity> = Vec::new();
    for note in &score.notes {
        match out.last_mut() {
            Some(last) if last.onset == note.onset => {
                last.notes.push(note.id);
                last.pitches.push(note.midi);
                last.until.push(note.offset_seconds());
            }
            _ => out.push(Simultaneity {
                onset: note.onset,
                onset_seconds: note.onset_seconds,
                notes: vec![note.id],
                pitches: vec![note.midi],
                until: vec![note.offset_seconds()],
                pivot: HAND_DIVIDER_MIDI,
            }),
        }
    }
    for event in &mut out {
        let pairs: Vec<_> = event
            .notes
            .iter()
            .copied()
            .zip(event.pitches.iter().copied())
            .collect();
        let mut pairs: Vec<_> = pairs
            .into_iter()
            .zip(event.until.iter().copied())
            .map(|((n, p), u)| (n, p, u))
            .collect();
        pairs.sort_by_key(|(_, p, _)| *p);
        event.notes = pairs.iter().map(|(n, _, _)| *n).collect();
        event.pitches = pairs.iter().map(|(_, p, _)| *p).collect();
        event.until = pairs.iter().map(|(_, _, u)| *u).collect();
    }
    out
}

fn split_cost(
    event: &Simultaneity,
    split: usize,
    keyboard: &Keyboard,
    max_span: f32,
    options: &HandAssignment,
) -> f32 {
    let mut cost = 0.0;
    for range in [&event.pitches[..split], &event.pitches[split..]] {
        let (Some(low), Some(high)) = (range.first(), range.last()) else {
            continue;
        };
        let span = keyboard.interval_mm(*low, *high);
        if span > max_span {
            cost += options.overspan_weight * (span - max_span) / 23.5;
        }
        if range.len() > 5 {
            cost += options.overspan_weight * (range.len() - 5) as f32;
        }
    }

    for (i, pitch) in event.pitches.iter().enumerate() {
        let hand = if i < split { Hand::Left } else { Hand::Right };
        cost += register_cost(*pitch, hand, event.pivot, options.register_weight);
    }
    cost
}

const HAND_DIVIDER_MIDI: f32 = 60.0;

fn set_pivots(events: &mut [Simultaneity], options: &HandAssignment) {
    let window = options.pivot_window_seconds;
    let semitone = 7.0 * on_hand::keyboard::WHITE_KEY_WIDTH / 12.0;
    let span = options.one_hand_fraction * options.profile.comfortable_span_mm() / semitone;

    let middles: Vec<(f64, f32, f32, f32)> = events
        .iter()
        .map(|event| {
            let sum: f32 = event.pitches.iter().map(|p| f32::from(*p)).sum();
            (
                event.onset_seconds,
                sum / event.pitches.len() as f32,
                f32::from(event.pitches[0]),
                f32::from(*event.pitches.last().unwrap()),
            )
        })
        .collect();

    let (mut lo, mut hi) = (0usize, 0usize);
    let mut sum = 0.0f32;
    for event in events.iter_mut() {
        let at = event.onset_seconds;
        while hi < middles.len() && middles[hi].0 <= at + window {
            sum += middles[hi].1;
            hi += 1;
        }
        while lo < hi && middles[lo].0 < at - window {
            sum -= middles[lo].1;
            lo += 1;
        }
        let mean = sum / (hi - lo) as f32;

        let low = middles[lo..hi].iter().fold(f32::MAX, |a, m| a.min(m.2));
        let high = middles[lo..hi].iter().fold(f32::MIN, |a, m| a.max(m.3));
        let widest = middles[lo..hi].iter().fold(0.0f32, |a, m| a.max(m.3 - m.2));
        event.pivot = if widest <= span {
            if mean < HAND_DIVIDER_MIDI {
                high + span
            } else {
                low - span
            }
        } else {
            mean
        };
    }
}

fn register_cost(midi: u8, hand: Hand, pivot: f32, weight: f32) -> f32 {
    let octaves = (midi as f32 - pivot) / 12.0;
    let wrong_way = match hand {
        Hand::Right => -octaves,
        Hand::Left => octaves,
    };
    weight * wrong_way.max(0.0)
}

fn travel_cost(
    previous: &Simultaneity,
    prev_split: usize,
    event: &Simultaneity,
    split: usize,
    keyboard: &Keyboard,
    gap_seconds: f64,
    reach_mm: f32,
    options: &HandAssignment,
    footed: bool,
) -> f32 {
    let mut cost = 0.0;
    for hand in [Hand::Left, Hand::Right] {
        let (was_low, was_high) = hand_reach(previous, prev_split, hand, keyboard);
        let (low, high) = hand_reach(event, split, hand, keyboard);
        let together = was_high.max(high) - was_low.min(low);
        let moved = (together - reach_mm).max(0.0);
        cost += options.travel_weight * moved / gap_seconds as f32;

        if let Some((held_low, held_high)) =
            still_held(previous, prev_split, hand, keyboard, event.onset_seconds)
                .filter(|_| !footed)
        {
            let with_hold = held_high.max(high) - held_low.min(low);
            cost += options.abandon_weight * (with_hold - reach_mm).max(0.0);
        }
    }
    cost
}

fn still_held(
    previous: &Simultaneity,
    split: usize,
    hand: Hand,
    keyboard: &Keyboard,
    now: f64,
) -> Option<(f32, f32)> {
    let range = match hand {
        Hand::Left => 0..split,
        Hand::Right => split..previous.pitches.len(),
    };
    let mut span: Option<(f32, f32)> = None;
    for i in range {
        if previous.until[i] <= now + 1e-6 {
            continue;
        }
        let x = keyboard.centre_x(previous.pitches[i]);
        span = Some(match span {
            Some((lo, hi)) => (lo.min(x), hi.max(x)),
            None => (x, x),
        });
    }
    span
}

const IDLE_PARK_MM: f32 = 150.0;

fn hand_reach(
    event: &Simultaneity,
    split: usize,
    hand: Hand,
    keyboard: &Keyboard,
) -> (f32, f32) {
    let range = match hand {
        Hand::Left => &event.pitches[..split],
        Hand::Right => &event.pitches[split..],
    };
    if let (Some(low), Some(high)) = (range.first(), range.last()) {
        return (keyboard.centre_x(*low), keyboard.centre_x(*high));
    }
    let parked = match hand {
        Hand::Left => keyboard.centre_x(event.pitches[0]) - IDLE_PARK_MM,
        Hand::Right => keyboard.centre_x(*event.pitches.last().unwrap()) + IDLE_PARK_MM,
    };
    (parked, parked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Note, SourceRef, TieState, TICKS_PER_QUARTER};

    fn note(id: u32, midi: u8, onset: Ticks, staff: Option<u8>) -> Note {
        Note {
            id: NoteId(id),
            midi,
            onset,
            duration: TICKS_PER_QUARTER as Ticks,
            onset_seconds: 0.0,
            duration_seconds: 0.0,
            staff,
            voice: None,
            hand: None,
            tie: TieState::default(),
            grace: false,
            chord: false,
            velocity: 64,
            given_finger: None,
            source: SourceRef::Midi { track: 0, event: id as usize },
        }
    }

    fn score_of(notes: Vec<Note>) -> Score {
        let mut s = Score { notes, ..Default::default() };
        s.finalise();
        s
    }

    #[test]
    fn staff_numbers_win_when_the_source_has_them() {
        let mut s = score_of(vec![
            note(0, 72, 0, Some(1)),
            note(1, 48, 0, Some(2)),
            note(2, 76, 960, Some(1)),
        ]);
        assign_hands(&mut s, &HandAssignment::default());
        assert_eq!(s.notes.iter().find(|n| n.midi == 72).unwrap().hand, Some(Hand::Right));
        assert_eq!(s.notes.iter().find(|n| n.midi == 48).unwrap().hand, Some(Hand::Left));
    }

    #[test]
    fn a_two_hand_texture_separates_without_staff_information() {
        let mut notes = Vec::new();
        let mut id = 0;
        for step in 0..8u8 {
            let t = step as Ticks * 480;
            notes.push(note(id, 43 + step, t, None));
            id += 1;
            notes.push(note(id, 72 + step, t, None));
            id += 1;
        }
        let mut s = score_of(notes);
        assign_hands(&mut s, &HandAssignment::default());
        for n in &s.notes {
            let expected = if n.midi < 60 { Hand::Left } else { Hand::Right };
            assert_eq!(n.hand, Some(expected), "midi {} went to the wrong hand", n.midi);
        }
    }

    #[test]
    fn a_single_line_stays_in_one_hand_rather_than_alternating() {
        let notes: Vec<_> = (0..12u8)
            .map(|i| note(i as u32, 60 + i, i as Ticks * 240, None))
            .collect();
        let mut s = score_of(notes);
        assign_hands(&mut s, &HandAssignment::default());
        let hands: Vec<_> = s.notes.iter().map(|n| n.hand.unwrap()).collect();
        let switches = hands.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(switches <= 1, "a single melodic line switched hands {switches} times");
    }

    #[test]
    fn a_line_through_the_middle_is_not_shared_out_between_the_hands() {
        let scale = [55u8, 57, 59, 60, 62, 64, 66, 67, 69, 71, 72, 74, 76, 78, 79];
        let notes: Vec<_> = scale
            .iter()
            .enumerate()
            .map(|(i, midi)| note(i as u32, *midi, i as Ticks * TICKS_PER_QUARTER as Ticks, None))
            .collect();
        let mut score = score_of(notes);
        assign_hands(&mut score, &HandAssignment::default());

        let hands: Vec<Hand> = score.notes.iter().map(|n| n.hand.unwrap()).collect();
        let swaps = hands.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(
            swaps <= 1,
            "the line changed hands {swaps} times: {hands:?}"
        );
    }

    #[test]
    fn the_hands_still_follow_the_music_up_the_keyboard() {
        let mut notes = Vec::new();
        let mut id = 0;
        for (step, (bass, tune)) in [(43u8, 67u8), (45, 69), (55, 79), (57, 81)]
            .into_iter()
            .enumerate()
        {
            for midi in [bass, tune] {
                notes.push(note(id, midi, step as Ticks * TICKS_PER_QUARTER as Ticks, None));
                id += 1;
            }
        }
        let mut score = score_of(notes);
        assign_hands(&mut score, &HandAssignment::default());

        for chunk in score.notes.chunks(2) {
            let low = chunk.iter().min_by_key(|n| n.midi).unwrap();
            let high = chunk.iter().max_by_key(|n| n.midi).unwrap();
            assert_eq!(low.hand, Some(Hand::Left), "the bass note went right");
            assert_eq!(high.hand, Some(Hand::Right), "the melody went left");
        }
    }

    #[test]
    fn a_hand_already_holding_a_wide_stretch_does_not_take_more() {
        let q = TICKS_PER_QUARTER as Ticks;
        let mut notes = vec![
            Note { duration: 4 * q, ..note(0, 33, 0, None) },
            Note { duration: 4 * q, ..note(1, 45, 0, None) },
        ];
        for (i, midi) in [64u8, 65, 67].into_iter().enumerate() {
            notes.push(note(2 + i as u32, midi, (i as Ticks + 1) * q, None));
        }
        let mut score = score_of(notes);
        assign_hands(&mut score, &HandAssignment::default());

        for note in score.notes.iter().filter(|n| n.midi >= 64) {
            assert_eq!(
                note.hand,
                Some(Hand::Right),
                "midi {} went to the left hand, which is holding an octave below it",
                note.midi
            );
        }
    }

    #[test]
    fn a_wide_chord_is_split_rather_than_crammed_into_one_hand() {
        let mut s = score_of(vec![
            note(0, 48, 0, None),
            note(1, 52, 0, None),
            note(2, 64, 0, None),
            note(3, 67, 0, None),
            note(4, 72, 0, None),
        ]);
        assign_hands(&mut s, &HandAssignment::default());
        let left: Vec<_> = s.hand_notes(Hand::Left).map(|n| n.midi).collect();
        let right: Vec<_> = s.hand_notes(Hand::Right).map(|n| n.midi).collect();
        assert!(!left.is_empty() && !right.is_empty(), "left {left:?} right {right:?}");
        assert!(left.iter().max() < right.iter().min());
    }

    #[test]
    fn a_fast_figure_under_one_hand_stays_in_one_hand() {
        let step = TICKS_PER_QUARTER as Ticks / 4;
        let notes: Vec<Note> = [48u8, 55, 52, 55]
            .iter()
            .cycle()
            .take(16)
            .enumerate()
            .map(|(i, midi)| note(i as u32, *midi, i as Ticks * step, None))
            .collect();
        let mut score = score_of(notes);
        assign_hands(&mut score, &HandAssignment::default());

        let hands: std::collections::BTreeSet<_> =
            score.notes.iter().filter_map(|n| n.hand).collect();
        assert_eq!(
            hands.len(),
            1,
            "the figure fits under one hand, so one hand should have it"
        );
        assert!(hands.contains(&Hand::Left), "and down there it is the left one");
    }

    #[test]
    fn a_leap_no_hand_could_cover_is_still_charged_for() {
        let step = TICKS_PER_QUARTER as Ticks / 8;
        let notes: Vec<Note> = [24u8, 96]
            .iter()
            .cycle()
            .take(8)
            .enumerate()
            .map(|(i, midi)| note(i as u32, *midi, i as Ticks * step, None))
            .collect();
        let mut score = score_of(notes);
        assign_hands(&mut score, &HandAssignment::default());

        let hands: std::collections::BTreeSet<_> =
            score.notes.iter().filter_map(|n| n.hand).collect();
        assert_eq!(hands.len(), 2, "each end of the keyboard gets its own hand");
    }

    #[test]
    fn a_hand_is_still_committed_to_what_it_has_not_let_go_of() {
        let keyboard = Keyboard::new();
        let event = Simultaneity {
            onset: 0,
            onset_seconds: 0.0,
            notes: vec![NoteId(0), NoteId(1), NoteId(2)],
            pitches: vec![48, 52, 79],
            until: vec![4.0, 4.0, 0.5],
            pivot: HAND_DIVIDER_MIDI,
        };

        let held = still_held(&event, 2, Hand::Left, &keyboard, 1.0)
            .expect("the left hand is still holding its chord");
        assert!(
            (held.1 - held.0 - keyboard.interval_mm(48, 52)).abs() < 1e-3,
            "the hold should span the two notes still sounding"
        );

        assert_eq!(
            still_held(&event, 2, Hand::Right, &keyboard, 1.0),
            None,
            "a hand whose note has stopped is holding nothing and may go where it likes"
        );

        assert_eq!(still_held(&event, 2, Hand::Left, &keyboard, 9.0), None);
    }

    #[test]
    fn a_hand_holding_a_chord_does_not_go_and_fetch_something_else() {
        let q = TICKS_PER_QUARTER as Ticks;
        let mut notes = vec![
            Note { duration: 8 * q, ..note(0, 28, 0, None) },
            Note { duration: 8 * q, ..note(1, 40, 0, None) },
        ];
        let figure = [52u8, 64, 71, 64, 52, 64, 71, 64];
        for (i, midi) in figure.iter().enumerate() {
            notes.push(Note {
                duration: q / 2,
                ..note(2 + i as u32, *midi, (i as Ticks + 1) * q, None)
            });
        }
        let mut score = score_of(notes);
        assign_hands(&mut score, &HandAssignment::default());

        let hand_of = |id: u32| score.notes.iter().find(|n| n.id == NoteId(id)).unwrap().hand;
        assert_eq!(hand_of(0), Some(Hand::Left), "the bass octave is the left hand's");
        assert_eq!(hand_of(1), Some(Hand::Left));
        for (i, midi) in figure.iter().enumerate() {
            let id = 2 + i as u32;
            assert_eq!(
                hand_of(id),
                Some(Hand::Right),
                "note {id} ({midi}) belongs to the right hand: the left is holding the octave"
            );
        }
    }
}
