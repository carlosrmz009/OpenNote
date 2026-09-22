//! Reading MIDI into a [`Score`], and writing one back out carrying fingerings.
//!
//! MIDI says nothing about staves, voices or ties, so a MIDI import leans on
//! [`crate::hands`] to work out which hand plays what. It does carry real timing and
//! real dynamics, which notation does not, and the fingering model uses both.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
};

use crate::{Fingering, Note, NoteId, Score, SourceRef, Ticks, TieState, TICKS_PER_QUARTER};

/// A MIDI file and the score extracted from it.
pub struct MidiDocument {
    raw: Vec<u8>,
    score: Score,
    /// Ticks per quarter note as declared by the source file.
    source_ppq: u16,
}

impl MidiDocument {
    /// Read a `.mid` file.
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_bytes(raw)
    }

    /// Parse MIDI already in memory.
    pub fn from_bytes(raw: Vec<u8>) -> Result<Self> {
        let smf = Smf::parse(&raw).map_err(|e| anyhow!("{e}"))?;
        let source_ppq = match smf.header.timing {
            Timing::Metrical(t) => t.as_int(),
            Timing::Timecode(..) => {
                bail!("SMPTE-timed MIDI files are not supported; re-export with metrical timing")
            }
        };
        if source_ppq == 0 {
            bail!("the MIDI file declares zero ticks per quarter note");
        }
        let score = extract(&smf, source_ppq)?;
        Ok(Self { raw, score, source_ppq })
    }

    /// The flattened score.
    pub fn score(&self) -> &Score {
        &self.score
    }

    /// Mutable access to the flattened score.
    pub fn score_mut(&mut self) -> &mut Score {
        &mut self.score
    }

    /// Ticks per quarter note in the source file.
    pub fn source_ppq(&self) -> u16 {
        self.source_ppq
    }

    /// Write the file back out with a text marker on every fingered note.
    ///
    /// MIDI has nowhere structural to put a fingering, so each one is emitted as a
    /// text meta event immediately before its note-on, in the form `f>3` — hand
    /// tag then finger number. That is readable by any MIDI tool and round-trips
    /// through this crate.
    pub fn write_annotated(&self, path: impl AsRef<Path>, fingerings: &[Fingering]) -> Result<()> {
        let smf = Smf::parse(&self.raw).map_err(|e| anyhow!("{e}"))?;

        // Index the fingerings by the event they belong to.
        let mut by_event: HashMap<(usize, usize), String> = HashMap::new();
        for f in fingerings {
            let Some(note) = self.score.notes.get(f.note.index()) else {
                continue;
            };
            let SourceRef::Midi { track, event } = note.source else {
                continue;
            };
            let hand = note.hand.map(|h| h.tag()).unwrap_or('?');
            by_event.insert((track, event), format!("f{hand}{}", f.notation()));
        }

        let mut out = Smf::new(smf.header);
        for (track_index, track) in smf.tracks.iter().enumerate() {
            let mut new_track = Track::new();
            for (event_index, event) in track.iter().enumerate() {
                if let Some(text) = by_event.get(&(track_index, event_index)) {
                    // The marker takes the note's delta so the note itself lands at
                    // the same tick it always did.
                    new_track.push(TrackEvent {
                        delta: event.delta,
                        kind: TrackEventKind::Meta(MetaMessage::Text(text.as_bytes())),
                    });
                    new_track.push(TrackEvent { delta: 0.into(), kind: event.kind });
                } else {
                    new_track.push(*event);
                }
            }
            out.tracks.push(new_track);
        }

        let path = path.as_ref();
        out.save(path)
            .with_context(|| format!("writing {}", path.display()))
    }
}

/// The controller number the damper pedal speaks on.
///
/// Half-pedalling is a real technique and a real pedal is continuous, but as far as
/// this is concerned the string either rings on or it does not, and sixty-four is where
/// the convention puts the line.
const DAMPER_PEDAL: u8 = 64;

/// A note-on waiting for its note-off.
#[derive(Clone, Copy)]
struct Pending {
    tick: Ticks,
    velocity: u8,
    event: usize,
}

fn extract(smf: &Smf, source_ppq: u16) -> Result<Score> {
    let mut score = Score::default();

    // Tempo lives on track 0 in format 1 files but may appear anywhere, so sweep
    // every track for it before doing anything with timing.
    for track in &smf.tracks {
        let mut tick: u64 = 0;
        for event in track {
            tick += event.delta.as_int() as u64;
            if let TrackEventKind::Meta(MetaMessage::Tempo(micros)) = event.kind {
                let t = rescale(tick as i64, source_ppq);
                score.tempo.insert(t, micros.as_int());
            }
            if let TrackEventKind::Meta(MetaMessage::TrackName(name)) = event.kind {
                if score.title.is_none() {
                    let name = String::from_utf8_lossy(name).trim().to_string();
                    if !name.is_empty() {
                        score.title = Some(name);
                    }
                }
            }
        }
    }

    // The damper, gathered across every track: it is a foot, not a voice, and which
    // track a sequencer happened to write it on means nothing.
    let mut pressed: Option<Ticks> = None;
    let mut pedal: Vec<(Ticks, Ticks)> = Vec::new();

    for (track_index, track) in smf.tracks.iter().enumerate() {
        let mut tick: u64 = 0;
        let mut pending: HashMap<(u8, u8), Pending> = HashMap::new();

        for (event_index, event) in track.iter().enumerate() {
            tick += event.delta.as_int() as u64;
            let TrackEventKind::Midi { channel, message } = event.kind else {
                continue;
            };
            let channel = channel.as_int();
            match message {
                MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                    // A second note-on for a key already sounding ends the first,
                    // which is what most sequencers mean by it.
                    if let Some(open) = pending.remove(&(channel, key.as_int())) {
                        push_note(&mut score, track_index, key.as_int(), open, rescale(tick as i64, source_ppq));
                    }
                    pending.insert(
                        (channel, key.as_int()),
                        Pending {
                            tick: rescale(tick as i64, source_ppq),
                            velocity: vel.as_int(),
                            event: event_index,
                        },
                    );
                }
                MidiMessage::NoteOff { key, .. } | MidiMessage::NoteOn { key, .. } => {
                    if let Some(open) = pending.remove(&(channel, key.as_int())) {
                        push_note(&mut score, track_index, key.as_int(), open, rescale(tick as i64, source_ppq));
                    }
                }
                MidiMessage::Controller { controller, value }
                    if controller.as_int() == DAMPER_PEDAL =>
                {
                    let at = rescale(tick as i64, source_ppq);
                    match (value.as_int() >= DAMPER_PEDAL, pressed) {
                        (true, None) => pressed = Some(at),
                        (false, Some(down)) => {
                            pressed = None;
                            if at > down {
                                pedal.push((down, at));
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // Anything still sounding at the end of the track gets closed there.
        let end = rescale(tick as i64, source_ppq);
        let mut leftovers: Vec<_> = pending.into_iter().collect();
        leftovers.sort_by_key(|((_, key), open)| (open.tick, *key));
        for ((_, key), open) in leftovers {
            push_note(&mut score, track_index, key, open, end);
        }
    }

    if score.notes.is_empty() {
        bail!("the MIDI file contains no notes");
    }
    // A press left open at the end of the file is a press that never came up.
    if let Some(down) = pressed {
        let end = score.notes.iter().map(|n| n.offset()).max().unwrap_or(down);
        if end > down {
            pedal.push((down, end));
        }
    }
    pedal.sort_by_key(|(down, _)| *down);
    score.pedal = pedal;
    score.finalise();
    Ok(score)
}

fn push_note(score: &mut Score, track: usize, key: u8, open: Pending, off_tick: Ticks) {
    let duration = (off_tick - open.tick).max(1);
    score.notes.push(Note {
        id: NoteId(0),
        midi: key,
        onset: open.tick,
        duration,
        onset_seconds: 0.0,
        duration_seconds: 0.0,
        staff: None,
        voice: None,
        hand: None,
        tie: TieState::default(),
        grace: false,
        chord: false,
        velocity: open.velocity.max(1),
        given_finger: None,
        source: SourceRef::Midi { track, event: open.event },
    });
}

/// Convert a tick in the file's own resolution to the internal grid.
fn rescale(tick: Ticks, source_ppq: u16) -> Ticks {
    tick * TICKS_PER_QUARTER as Ticks / source_ppq as Ticks
}

/// Build a minimal MIDI file from a score, for tests and for exporting a
/// visualization timeline.
pub fn to_smf(score: &Score) -> Smf<'static> {
    let mut smf = Smf::new(Header::new(
        Format::SingleTrack,
        Timing::Metrical((TICKS_PER_QUARTER as u16).into()),
    ));
    let mut events: Vec<(Ticks, TrackEventKind<'static>)> = Vec::new();

    for change in score.tempo.changes() {
        events.push((
            change.tick,
            TrackEventKind::Meta(MetaMessage::Tempo(change.micros_per_quarter.into())),
        ));
    }
    for note in &score.notes {
        events.push((
            note.onset,
            TrackEventKind::Midi {
                channel: 0.into(),
                message: MidiMessage::NoteOn {
                    key: note.midi.into(),
                    vel: note.velocity.into(),
                },
            },
        ));
        events.push((
            note.offset(),
            TrackEventKind::Midi {
                channel: 0.into(),
                message: MidiMessage::NoteOff {
                    key: note.midi.into(),
                    vel: 0.into(),
                },
            },
        ));
    }
    events.sort_by_key(|(tick, _)| *tick);

    let mut track = Track::new();
    let mut previous = 0;
    for (tick, kind) in events {
        track.push(TrackEvent {
            delta: ((tick - previous) as u32).into(),
            kind,
        });
        previous = tick;
    }
    track.push(TrackEvent {
        delta: 0.into(),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    smf.tracks.push(track);
    smf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a two-note MIDI file in memory and read it back.
    fn round_trip(notes: &[(u8, i64, i64)]) -> Score {
        let mut score = Score::default();
        for (i, (midi, onset, duration)) in notes.iter().enumerate() {
            score.notes.push(Note {
                id: NoteId(i as u32),
                midi: *midi,
                onset: *onset,
                duration: *duration,
                onset_seconds: 0.0,
                duration_seconds: 0.0,
                staff: None,
                voice: None,
                hand: None,
                tie: TieState::default(),
                grace: false,
                chord: false,
                velocity: 90,
                given_finger: None,
                source: SourceRef::Midi { track: 0, event: i },
            });
        }
        score.finalise();

        let mut bytes = Vec::new();
        to_smf(&score).write(&mut bytes).unwrap();
        MidiDocument::from_bytes(bytes).unwrap().score().clone()
    }

    #[test]
    fn notes_survive_a_midi_round_trip() {
        let ppq = TICKS_PER_QUARTER as i64;
        let back = round_trip(&[(60, 0, ppq), (64, ppq, ppq), (67, 2 * ppq, 2 * ppq)]);
        assert_eq!(back.notes.len(), 3);
        assert_eq!(back.notes.iter().map(|n| n.midi).collect::<Vec<_>>(), vec![60, 64, 67]);
        assert_eq!(back.notes[1].onset, ppq);
        assert_eq!(back.notes[2].duration, 2 * ppq);
    }

    #[test]
    fn simultaneous_notes_keep_their_shared_onset() {
        let ppq = TICKS_PER_QUARTER as i64;
        let back = round_trip(&[(60, 0, ppq), (64, 0, ppq), (67, 0, ppq)]);
        assert_eq!(back.chords(on_hand::Hand::Right).len(), 0, "hands are not assigned yet");
        assert!(back.notes.iter().all(|n| n.onset == 0));
    }

    #[test]
    fn tempo_is_read_back_from_the_file() {
        let mut score =
            Score { tempo: crate::TempoMap::constant_bpm(144.0), ..Default::default() };
        score.notes.push(Note {
            id: NoteId(0),
            midi: 60,
            onset: 0,
            duration: 480,
            onset_seconds: 0.0,
            duration_seconds: 0.0,
            staff: None,
            voice: None,
            hand: None,
            tie: TieState::default(),
            grace: false,
            chord: false,
            velocity: 80,
            given_finger: None,
            source: SourceRef::Midi { track: 0, event: 0 },
        });
        score.finalise();

        let mut bytes = Vec::new();
        to_smf(&score).write(&mut bytes).unwrap();
        let back = MidiDocument::from_bytes(bytes).unwrap();
        assert!((back.score().tempo.bpm_at(0) - 144.0).abs() < 0.5);
    }

    #[test]
    fn a_zero_velocity_note_on_ends_the_note() {
        // Sequencers commonly use note-on with velocity 0 instead of note-off.
        let mut bytes = Vec::new();
        let mut smf = Smf::new(Header::new(
            Format::SingleTrack,
            Timing::Metrical((TICKS_PER_QUARTER as u16).into()),
        ));
        let mut track = Track::new();
        track.push(TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Midi {
                channel: 0.into(),
                message: MidiMessage::NoteOn { key: 60.into(), vel: 100.into() },
            },
        });
        track.push(TrackEvent {
            delta: 480.into(),
            kind: TrackEventKind::Midi {
                channel: 0.into(),
                message: MidiMessage::NoteOn { key: 60.into(), vel: 0.into() },
            },
        });
        track.push(TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        });
        smf.tracks.push(track);
        smf.write(&mut bytes).unwrap();

        let doc = MidiDocument::from_bytes(bytes).unwrap();
        assert_eq!(doc.score().notes.len(), 1);
        assert_eq!(doc.score().notes[0].duration, 480);
    }

    #[test]
    fn the_damper_pedal_is_read_and_holds_notes_past_their_keys() {
        // A note a beat long, with the pedal going down while it sounds and coming up
        // two beats after the key does. The hand lets the key go on the beat; the string
        // rings until the foot lets it.
        let q = TICKS_PER_QUARTER as u32;
        let mut track = Track::new();
        let mut push = |delta: u32, kind: TrackEventKind<'static>| {
            track.push(TrackEvent { delta: delta.into(), kind });
        };
        push(0, TrackEventKind::Midi {
            channel: 0.into(),
            message: MidiMessage::Controller { controller: 64.into(), value: 127.into() },
        });
        push(0, TrackEventKind::Midi {
            channel: 0.into(),
            message: MidiMessage::NoteOn { key: 60.into(), vel: 90.into() },
        });
        push(q, TrackEventKind::Midi {
            channel: 0.into(),
            message: MidiMessage::NoteOff { key: 60.into(), vel: 0.into() },
        });
        push(2 * q, TrackEventKind::Midi {
            channel: 0.into(),
            message: MidiMessage::Controller { controller: 64.into(), value: 0.into() },
        });
        push(0, TrackEventKind::Meta(MetaMessage::EndOfTrack));

        let mut smf = Smf::new(Header::new(
            Format::SingleTrack,
            Timing::Metrical((TICKS_PER_QUARTER as u16).into()),
        ));
        smf.tracks.push(track);
        let mut bytes = Vec::new();
        smf.write(&mut bytes).unwrap();
        let document = MidiDocument::from_bytes(bytes).unwrap();
        let score = document.score();

        assert_eq!(score.pedal.len(), 1, "one press: {:?}", score.pedal);
        let note = score.notes[0].id;
        let key_up = score.notes[0].offset_seconds();
        let damped = score.damped_seconds(note);
        assert!(
            damped > key_up + 0.5,
            "the string should ring on past the key: up at {key_up}, damped at {damped}"
        );
    }
}
