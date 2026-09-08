//! Deciding which hand plays which note.
//!
//! Engraved piano music answers this for us: staff 1 is the right hand, staff 2 the
//! left. MIDI does not, and neither do single-staff sources, so those get a small
//! dynamic program.
//!
//! The program exploits the fact that at any instant the two hands almost never
//! interleave in pitch — one hand takes the low notes and the other the high ones,
//! and the only question is where the boundary falls. So each simultaneity has just
//! `n + 1` possible splits, and Viterbi over those splits is exact rather than
//! heuristic. A hand crossing where one hand reaches past the other is representable
//! as the boundary sweeping all the way to one end, so a hand can take over the far
//! side of the keyboard; what the model cannot express is the two hands
//! *interleaving* within a single simultaneity, which is rare enough to accept.

use on_hand::keyboard::Keyboard;
use on_hand::profile::HandProfile;
use on_hand::Hand;

use crate::{NoteId, Score, Ticks};

/// Options for hand assignment.
#[derive(Debug, Clone)]
pub struct HandAssignment {
    /// The hand being assigned for, used to size the reachable span.
    pub profile: HandProfile,
    /// Cost of a hand having to cover more than it can span.
    pub overspan_weight: f32,
    /// Cost of moving a hand, per millimetre per second.
    pub travel_weight: f32,
    /// Cost, per octave, of a hand playing on the wrong side of the keyboard.
    pub register_weight: f32,
    /// Cost, per millimetre, of a hand having to let go of something it is holding.
    ///
    /// A hand that is holding a chord cannot also be somewhere else. Giving it a note
    /// it can only reach by abandoning what it is holding used to be free, so a left
    /// hand sitting on a held chord would take a passing note that the right hand could
    /// have reached, and let the chord go to get it.
    ///
    /// Not prohibitive, because letting go is a real thing to do — that is what the
    /// pedal is for, and it is often the right answer. It is charged rather than
    /// forbidden so the search weighs it against what the other hand would have to do.
    pub abandon_weight: f32,
}

impl Default for HandAssignment {
    fn default() -> Self {
        Self {
            profile: HandProfile::default(),
            overspan_weight: 4.0,
            travel_weight: 0.004,
            register_weight: 0.6,
            abandon_weight: 0.4,
        }
    }
}

/// Give every note in the score a hand.
///
/// Uses staff numbers when the source provides them for every note, and otherwise
/// separates the hands by dynamic programming.
pub fn assign_hands(score: &mut Score, options: &HandAssignment) {
    if assign_from_staves(score) {
        return;
    }
    assign_by_search(score, options);
}

/// Use staff numbers if they are present and describe a two-staff piano part.
///
/// Returns whether the assignment succeeded.
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

    // The lower-numbered staff is the upper one on the page, which is the right hand.
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

/// One instant at which notes begin, with the notes sorted low to high.
struct Simultaneity {
    onset: Ticks,
    onset_seconds: f64,
    notes: Vec<NoteId>,
    pitches: Vec<u8>,
    /// When each note stops sounding, so the search can tell a note the hand has let go
    /// of from one it is still holding.
    until: Vec<f64>,
}

/// Assign hands by Viterbi over pitch splits.
fn assign_by_search(score: &mut Score, options: &HandAssignment) {
    let mut events = collect_simultaneities(score);
    if events.is_empty() {
        return;
    }
    // A hand that is still holding something is not free to go elsewhere, and the
    // search cannot know that unless each event says what is still down.
    hold_sustained(&mut events);
    // When the foot is holding the strings, in seconds.
    //
    // A hand is only obliged to stay on a key while the key is what is holding the note.
    // Under the pedal it is not, and a bass note held under a figure two octaves above it
    // is the ordinary way of writing a pedal point — the hand plays it, leaves it to the
    // foot and goes where it is needed. Without knowing that, keeping every sounding note
    // under its own hand would refuse to let a pianist do the commonest thing they do.
    let pedal: Vec<(f64, f64)> = score
        .pedal
        .iter()
        .map(|(down, up)| (score.tempo.seconds_at(*down), score.tempo.seconds_at(*up)))
        .collect();
    let footed = |at: f64| pedal.iter().any(|(down, up)| *down <= at && at < *up);

    let keyboard = Keyboard::new();
    let max_span = options.profile.comfortable_span_mm();

    // State: how many of this event's notes go to the left hand.
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

    // Walk the best path back and apply it.
    let mut split = costs
        .last()
        .unwrap()
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
        .unwrap_or(0);

    // Backwards, and that matters. A note held across several events appears in all of
    // them, so it is written more than once; going back to front means the event that
    // *struck* it writes last and wins, which is the one whose split actually decided
    // whose note it was.
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

/// Fold notes that are still sounding into every event that happens while they sound.
///
/// Without this the search sees one instant at a time and nothing else. A hand holding a
/// bass octave struck four events ago looks perfectly free, because [`Simultaneity`]
/// only carries the notes that *begin* at its own onset — so the octave was invisible,
/// and the cheapest way to reach a note two octaves above it was to send the hand that
/// was already holding the octave. It let go of both and went, while the other hand sat
/// idle. A hand cannot be in two places, and the search had no way of knowing it was
/// asking one to be.
///
/// Folded in, the note is part of what its hand has to cover, so the ordinary span and
/// travel costs charge for it and the search sends the free hand instead. It stays a
/// cost rather than a prohibition on purpose: when both hands are genuinely committed
/// something has to give, and then the cheapest thing to give up is what gives.
///
/// A note is carried only as long as its written duration, which is the span over which
/// the score is asking for it to be held. What the pedal does with it afterwards is the
/// pedal's business.
fn hold_sustained(events: &mut [Simultaneity]) {
    // What each earlier event struck, and when it stops.
    let mut sounding: Vec<(NoteId, u8, f64)> = Vec::new();
    for index in 0..events.len() {
        let now = events[index].onset_seconds;
        sounding.retain(|(_, _, until)| *until > now + 1e-6);

        // Remember what this event strikes before anything is folded into it, so a
        // held note is never recorded twice.
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

/// Sort an event's parallel arrays low to high, which the split argument depends on.
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

/// Group note onsets into simultaneities, sorted low to high within each.
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
            }),
        }
    }
    // `Score::finalise` already sorts by pitch within an onset, but be explicit:
    // the split argument depends on it.
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

/// What it costs for one hand to hold a set of pitches at one instant.
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
            // More notes than fingers: something has to give, and it is usually the
            // split that is wrong.
            cost += options.overspan_weight * (range.len() - 5) as f32;
        }
    }

    // Which side of the keyboard each hand belongs on. Without this the split is
    // exactly tied for a single note — either hand could take it — and the path
    // would flip back and forth arbitrarily through a one-line passage.
    for (i, pitch) in event.pitches.iter().enumerate() {
        let hand = if i < split { Hand::Left } else { Hand::Right };
        cost += register_cost(*pitch, hand, options.register_weight);
    }
    cost
}

/// Pitch class of the keyboard's midpoint between the hands: middle C.
const HAND_DIVIDER_MIDI: f32 = 60.0;

/// What it costs a hand to reach across to the other side of the keyboard, per
/// octave. Zero on its own side, so it never fights a genuine hand crossing that
/// the travel and span terms have already paid for.
fn register_cost(midi: u8, hand: Hand, weight: f32) -> f32 {
    let octaves = (midi as f32 - HAND_DIVIDER_MIDI) / 12.0;
    let wrong_way = match hand {
        Hand::Right => -octaves,
        Hand::Left => octaves,
    };
    weight * wrong_way.max(0.0)
}

/// What it costs to move the hands from one simultaneity to the next.
///
/// The distance from one note to the next is not the distance a hand travels. A hand
/// covers a span, and two notes a fifth apart are reached by two fingers without
/// anything moving at all — which is the whole reason figures that sit under the hand
/// get written. What actually has to move is only whatever one hand position cannot
/// cover: if everything the hand played and everything it plays next fits inside its
/// reach, it can stay exactly where it is, however fast the passage goes.
///
/// Charging the full interval instead, and then dividing it by how little time there
/// was, made an Alberti bass look impossible for one hand as soon as it went quickly,
/// and the search would tear the figure in half and hand a note to each.
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

        // Travel is what it costs a hand to go somewhere. This is what it costs a hand
        // to go somewhere it cannot go without first letting go of what it is holding.
        //
        // The two are not the same and the difference is the whole point: travel is
        // divided by the time available, so given a moment it is nearly free, and a
        // hand sitting on a held chord would happily take a passing note the other hand
        // could have reached and drop the chord to get it. What it has to abandon does
        // not get cheaper because there was time.
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

/// The span a hand is still committed to from the previous instant: the notes it took
/// then that have not stopped sounding by now.
///
/// `None` when it is holding nothing and is free to go where it likes.
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

/// How far beyond the music an idle hand is assumed to be waiting, in millimetres.
/// Roughly a seventh, which is about where the other hand sits in ordinary
/// two-stave writing.
const IDLE_PARK_MM: f32 = 150.0;

/// What a hand has to cover, in millimetres along the keyboard: its lowest note and
/// its highest.
///
/// A hand with nothing to play still has a position. Saying so matters more than it
/// looks: if an idle hand had no position, then handing a melody back and forth note
/// by note would cost nothing at all, because each hand would appear to teleport into
/// place. Parking it just beyond the notes the other hand is taking makes alternation
/// as expensive as it really is.
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
        // Left hand walking bass, right hand melody two octaves up.
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

    /// A melodic line that runs through the middle of the keyboard stays in one hand.
    ///
    /// This is the failure that shows up worst on screen. Notes near where the hands
    /// meet are nearly tied — either hand could take them — so without a cost on the
    /// division moving, a scale crossing middle C gets handed back and forth note by
    /// note. No pianist plays it that way and no editor writes it that way.
    #[test]
    fn a_line_through_the_middle_is_not_shared_out_between_the_hands() {
        // Two octaves of G major, G3 up to G5, straight through middle C.
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

    /// The division still moves when the music genuinely moves.
    ///
    /// The stability cost must not become a ban: a piece that starts low and ends two
    /// octaves higher has to be allowed to take its hands with it.
    #[test]
    fn the_hands_still_follow_the_music_up_the_keyboard() {
        // A bass line and a melody, both rising by an octave part way through.
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

    /// A hand cannot take a note it has no finger free for.
    ///
    /// The left hand holds two low notes for a whole bar while a line appears a
    /// twelfth above them. Those upper notes cannot be the left hand's: it is already
    /// stretched across an octave and has nothing left to reach with. The right hand
    /// is idle and takes them.
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
        // A tenth plus a third above it cannot be one hand.
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
        // An Alberti bass: low, high, middle, high, over and over, the whole thing
        // inside a fifth. One hand plays it and the hand does not move — that is what
        // the figure is for. Measuring the travel from one note to the next and then
        // dividing by how little time there was made it look impossible as it got
        // quicker, and the search would tear it in half and give a note to each hand.
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
        // Three octaves apart in a twentieth of a second. Nothing about the change
        // above should make a jump like this look free, or a passage of them would all
        // be given to one hand.
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
        // The span a hand is tied to is the notes it took that are still sounding, and
        // that is a different set from the notes it took. Travel alone cannot see the
        // difference: it is divided by the time available, so given a moment it is
        // nearly free, and a hand sitting on a held chord would take a passing note the
        // other hand could have reached and drop the chord to get it.
        let keyboard = Keyboard::new();
        let event = Simultaneity {
            onset: 0,
            onset_seconds: 0.0,
            notes: vec![NoteId(0), NoteId(1), NoteId(2)],
            pitches: vec![48, 52, 79],
            // The left hand's two notes ring on; the right hand's has stopped.
            until: vec![4.0, 4.0, 0.5],
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

        // And once everything has stopped, neither hand is committed to anything.
        assert_eq!(still_held(&event, 2, Hand::Left, &keyboard, 9.0), None);
    }

    #[test]
    fn a_hand_holding_a_chord_does_not_go_and_fetch_something_else() {
        // The shape of the opening of a great many romantic pieces: the left hand puts
        // down a bass octave and holds it, and a broken figure runs above it. The
        // figure's lowest note sits between the two hands, so it can be argued either
        // way — until you notice that the left hand is holding the octave and cannot
        // be in two places.
        //
        // It used to be argued the wrong way, and not because the cost was too low.
        // Each instant was scored on its own, so an octave struck four events earlier
        // and still held was invisible: the left hand looked idle, and reaching down
        // for the figure looked free. It let go of both notes and went, while the
        // right hand — which had a gap exactly there — sat still.
        let q = TICKS_PER_QUARTER as Ticks;
        let mut notes = vec![
            // The bass octave, held right through.
            Note { duration: 8 * q, ..note(0, 28, 0, None) },
            Note { duration: 8 * q, ..note(1, 40, 0, None) },
        ];
        // A figure above it: 52, 64, 71, 64 repeating. The 52s are the ones in
        // question, and the right hand is between notes when each of them falls.
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
