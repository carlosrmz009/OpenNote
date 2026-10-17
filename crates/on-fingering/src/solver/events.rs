//! Turns a score into the chords each hand plays, with the notes it is still holding.

use super::*;

#[derive(Debug, Clone)]
pub struct Event {
    pub notes: Vec<u8>,
    pub ids: Vec<NoteId>,
    pub struck: Vec<bool>,
    pub onset_seconds: f64,
    pub pinned: Vec<Option<Finger>>,
}

impl Event {
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }
}

pub fn build_events(score: &Score, hand: Hand) -> Vec<Event> {
    let mut events: Vec<Event> = Vec::new();
    for chord in score.chords(hand) {
        let mut notes = Vec::new();
        let mut ids = Vec::new();
        let mut pinned = Vec::new();
        for id in &chord.notes {
            let note = score.note(*id);
            if note.tie.is_continuation() {
                continue;
            }
            if notes.contains(&note.midi) {
                continue;
            }
            notes.push(note.midi);
            ids.push(*id);
            pinned.push(note.given_finger);
        }
        if notes.is_empty() {
            continue;
        }
        let struck = vec![true; notes.len()];
        events.push(Event { notes, ids, struck, onset_seconds: chord.onset_seconds, pinned });
    }
    events
}

pub(crate) const FINGERS: usize = 5;

pub(crate) const RELEASE_SECONDS: f64 = crate::playability::CHORD_SECONDS;

/// Adds to each chord the notes still held from earlier ones. Within a scale or run found
/// beforehand (`lines`, by event index), a note overlapping the next of the same run is
/// finger legato, released as the thumb passes, and is not held.
pub(crate) fn hold_sustained(
    events: &mut [Event],
    score: &Score,
    reach: i32,
    lines: &std::collections::HashSet<usize>,
) {
    let mut sounding: Vec<Sounding> = Vec::new();
    for index in 0..events.len() {
        let now = events[index].onset_seconds;
        sounding.retain(|s| s.until > now + RELEASE_SECONDS);
        let restruck: Vec<u8> = events[index].notes.clone();
        sounding.retain(|s| !restruck.contains(&s.midi) || events[index].ids.contains(&s.id));

        let struck: Vec<(NoteId, u8)> =
            events[index].ids.iter().zip(&events[index].notes).map(|(id, midi)| (*id, *midi)).collect();
        let (struck_low, struck_high) =
            struck.iter().fold((u8::MAX, u8::MIN), |(lo, hi), (_, m)| (lo.min(*m), hi.max(*m)));

        sounding.retain(|s| i32::from(s.high.max(struck_high) - s.low.min(struck_low)) <= reach);
        for s in &mut sounding {
            s.low = s.low.min(struck_low);
            s.high = s.high.max(struck_high);
        }

        let mut held: Vec<(NoteId, u8)> = sounding
            .iter()
            .rev()
            .filter(|s| !struck.iter().any(|(other, _)| *other == s.id))
            .filter(|s| !(lines.contains(&index) && lines.contains(&s.from)))
            .map(|s| (s.id, s.midi))
            .collect();

        let (mut low, mut high) =
            struck.iter().fold((u8::MAX, u8::MIN), |(lo, hi), (_, m)| (lo.min(*m), hi.max(*m)));
        let mut room = FINGERS.saturating_sub(struck.len());
        held.retain(|(_, midi)| {
            let (would_low, would_high) = (low.min(*midi), high.max(*midi));
            if room == 0 || i32::from(would_high - would_low) > reach {
                return false;
            }
            low = would_low;
            high = would_high;
            room -= 1;
            true
        });

        if !held.is_empty() {
            let event = &mut events[index];
            for (id, midi) in held {
                event.ids.push(id);
                event.notes.push(midi);
                event.struck.push(false);
                event.pinned.push(score.note(id).given_finger);
            }
            let mut order: Vec<usize> = (0..event.notes.len()).collect();
            order.sort_by_key(|i| event.notes[*i]);
            event.notes = order.iter().map(|i| event.notes[*i]).collect();
            event.ids = order.iter().map(|i| event.ids[*i]).collect();
            event.struck = order.iter().map(|i| event.struck[*i]).collect();
            event.pinned = order.iter().map(|i| event.pinned[*i]).collect();
        }

        for (id, midi) in struck {
            sounding.push(Sounding {
                id,
                midi,
                until: score.release_seconds(id),
                from: index,
                low: struck_low,
                high: struck_high,
            });
        }
    }
}

pub(crate) struct Sounding {
    pub(crate) id: NoteId,
    pub(crate) midi: u8,
    pub(crate) until: f64,
    pub(crate) from: usize,
    pub(crate) low: u8,
    pub(crate) high: u8,
}
