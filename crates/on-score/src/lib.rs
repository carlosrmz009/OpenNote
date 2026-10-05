pub mod hands;
pub mod midi_io;
pub mod musicxml_io;
pub mod timing;

pub use hands::assign_hands;
pub use midi_io::MidiDocument;
pub use musicxml_io::MusicXmlDocument;
pub use timing::TempoMap;

use on_hand::{Finger, Hand};

pub const TICKS_PER_QUARTER: u32 = 960;

pub type Ticks = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoteId(pub u32);

impl NoteId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TieState {
    pub start: bool,
    pub stop: bool,
}

impl TieState {
    pub fn is_continuation(&self) -> bool {
        self.stop
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRef {
    MusicXml {
        part: usize,
        measure: usize,
        element: usize,
    },
    Midi {
        track: usize,
        event: usize,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub id: NoteId,
    pub midi: u8,
    pub onset: Ticks,
    pub duration: Ticks,
    pub onset_seconds: f64,
    pub duration_seconds: f64,
    pub staff: Option<u8>,
    pub voice: Option<u16>,
    pub hand: Option<Hand>,
    pub tie: TieState,
    pub grace: bool,
    pub chord: bool,
    pub velocity: u8,
    pub given_finger: Option<Finger>,
    pub source: SourceRef,
}

impl Note {
    pub fn offset(&self) -> Ticks {
        self.onset + self.duration
    }

    pub fn offset_seconds(&self) -> f64 {
        self.onset_seconds + self.duration_seconds
    }

    pub fn sounds_at(&self, tick: Ticks) -> bool {
        self.onset <= tick && tick < self.offset()
    }

    pub fn is_black(&self) -> bool {
        on_hand::keyboard::is_black(self.midi)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Score {
    pub title: Option<String>,
    pub notes: Vec<Note>,
    pub tempo: TempoMap,
    pub pedal: Vec<(Ticks, Ticks)>,
}

impl Score {
    pub fn damped_seconds(&self, id: NoteId) -> f64 {
        let released = self.release_seconds(id);
        self.pedal
            .iter()
            .find(|(down, up)| {
                let down = self.tempo.seconds_at(*down);
                let up = self.tempo.seconds_at(*up);
                down <= released + 1e-9 && up > released
            })
            .map(|(_, up)| self.tempo.seconds_at(*up).max(released))
            .unwrap_or(released)
    }

    pub fn release_seconds(&self, id: NoteId) -> f64 {
        let note = self.note(id);
        let mut end = note.offset_seconds();
        if !note.tie.start {
            return end;
        }
        loop {
            let next = self.notes.iter().find(|other| {
                other.tie.is_continuation()
                    && other.midi == note.midi
                    && other.hand == note.hand
                    && (other.onset_seconds - end).abs() < 1e-4
            });
            match next {
                Some(next) => {
                    end = next.offset_seconds();
                    if !next.tie.start {
                        return end;
                    }
                }
                None => return end,
            }
        }
    }

    pub fn note(&self, id: NoteId) -> &Note {
        &self.notes[id.index()]
    }

    pub fn note_mut(&mut self, id: NoteId) -> &mut Note {
        &mut self.notes[id.index()]
    }

    pub fn hand_notes(&self, hand: Hand) -> impl Iterator<Item = &Note> {
        self.notes.iter().filter(move |n| n.hand == Some(hand))
    }

    pub fn duration(&self) -> Ticks {
        self.notes.iter().map(|n| n.offset()).max().unwrap_or(0)
    }

    pub fn duration_seconds(&self) -> f64 {
        self.notes.iter().map(|n| n.offset_seconds()).fold(0.0, f64::max)
    }

    pub fn finalise(&mut self) {
        self.drop_unplayable();
        self.notes.sort_by(|a, b| {
            a.onset
                .cmp(&b.onset)
                .then(a.midi.cmp(&b.midi))
                .then(a.staff.cmp(&b.staff))
        });
        for (i, note) in self.notes.iter_mut().enumerate() {
            note.id = NoteId(i as u32);
        }
        self.recompute_seconds();
    }

    fn drop_unplayable(&mut self) {
        let range = on_hand::keyboard::MIDI_LOWEST..=on_hand::keyboard::MIDI_HIGHEST;
        self.notes.retain(|note| range.contains(&note.midi));
    }

    pub fn recompute_seconds(&mut self) {
        for note in &mut self.notes {
            note.onset_seconds = self.tempo.seconds_at(note.onset);
            note.duration_seconds =
                self.tempo.seconds_at(note.offset()) - note.onset_seconds;
        }
    }

    pub fn chords(&self, hand: Hand) -> Vec<Chord> {
        let mut out: Vec<Chord> = Vec::new();
        for note in self.hand_notes(hand) {
            match out.last_mut() {
                Some(last) if last.onset == note.onset => last.notes.push(note.id),
                _ => out.push(Chord {
                    onset: note.onset,
                    onset_seconds: note.onset_seconds,
                    notes: vec![note.id],
                }),
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chord {
    pub onset: Ticks,
    pub onset_seconds: f64,
    pub notes: Vec<NoteId>,
}

impl Chord {
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingering {
    pub note: NoteId,
    pub finger: Finger,
    pub substitute: Option<Finger>,
}

impl Fingering {
    pub fn new(note: NoteId, finger: Finger) -> Self {
        Self { note, finger, substitute: None }
    }

    pub fn notation(&self) -> String {
        match self.substitute {
            Some(s) => format!("{}-{}", self.finger.number(), s.number()),
            None => self.finger.number().to_string(),
        }
    }
}

pub enum Document {
    MusicXml(Box<MusicXmlDocument>),
    Midi(Box<MidiDocument>),
}

impl Document {
    pub const EXTENSIONS: [&'static str; 5] = ["musicxml", "mxl", "xml", "mid", "midi"];

    pub fn open(path: &std::path::Path) -> anyhow::Result<Self> {
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "musicxml" | "xml" | "mxl" => {
                Ok(Document::MusicXml(Box::new(MusicXmlDocument::read(path)?)))
            }
            "mid" | "midi" => Ok(Document::Midi(Box::new(MidiDocument::read(path)?))),
            other => anyhow::bail!(
                "do not know how to read {other:?} files; \
                 give a .musicxml, .mxl, .xml, .mid or .midi file"
            ),
        }
    }

    pub fn score(&self) -> &Score {
        match self {
            Document::MusicXml(d) => d.score(),
            Document::Midi(d) => d.score(),
        }
    }

    pub fn score_mut(&mut self) -> &mut Score {
        match self {
            Document::MusicXml(d) => d.score_mut(),
            Document::Midi(d) => d.score_mut(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tie_carries_a_note_past_its_own_written_end() {
        let mut score = Score::default();
        for (id, onset, tie) in [
            (0u32, 0, TieState { start: true, stop: false }),
            (1, 480, TieState { start: true, stop: true }),
            (2, 960, TieState { start: false, stop: true }),
        ] {
            let mut n = note(id, 60, onset, Hand::Right);
            n.tie = tie;
            score.notes.push(n);
        }
        score.finalise();

        let written = score.note(NoteId(0)).offset_seconds();
        let sounding = score.release_seconds(NoteId(0));
        assert!(
            sounding > written + 1e-6,
            "the tie should carry the note past {written}, got {sounding}"
        );
        assert!((sounding - score.note(NoteId(2)).offset_seconds()).abs() < 1e-6);
    }

    #[test]
    fn an_untied_note_stops_where_it_is_written_to() {
        let mut score = Score::default();
        score.notes.push(note(0, 60, 0, Hand::Right));
        score.finalise();
        assert_eq!(
            score.release_seconds(NoteId(0)),
            score.note(NoteId(0)).offset_seconds()
        );
    }

    fn note(id: u32, midi: u8, onset: Ticks, hand: Hand) -> Note {
        Note {
            id: NoteId(id),
            midi,
            onset,
            duration: 480,
            onset_seconds: 0.0,
            duration_seconds: 0.0,
            staff: None,
            voice: None,
            hand: Some(hand),
            tie: TieState::default(),
            grace: false,
            chord: false,
            velocity: 64,
            given_finger: None,
            source: SourceRef::Midi { track: 0, event: id as usize },
        }
    }

    #[test]
    fn finalise_sorts_and_renumbers() {
        let mut s = Score {
            notes: vec![
                note(0, 67, 960, Hand::Right),
                note(1, 60, 0, Hand::Right),
                note(2, 64, 0, Hand::Right),
            ],
            ..Default::default()
        };
        s.finalise();
        assert_eq!(
            s.notes.iter().map(|n| n.midi).collect::<Vec<_>>(),
            vec![60, 64, 67]
        );
        for (i, n) in s.notes.iter().enumerate() {
            assert_eq!(n.id, NoteId(i as u32));
        }
    }

    #[test]
    fn chords_group_by_onset_within_a_hand() {
        let mut s = Score {
            notes: vec![
                note(0, 60, 0, Hand::Right),
                note(1, 64, 0, Hand::Right),
                note(2, 67, 0, Hand::Right),
                note(3, 48, 0, Hand::Left),
                note(4, 72, 960, Hand::Right),
            ],
            ..Default::default()
        };
        s.finalise();
        let right = s.chords(Hand::Right);
        assert_eq!(right.len(), 2);
        assert_eq!(right[0].len(), 3);
        assert_eq!(right[1].len(), 1);
        assert_eq!(s.chords(Hand::Left).len(), 1);
    }

    #[test]
    fn fingering_notation_covers_substitutions() {
        let plain = Fingering::new(NoteId(0), Finger::Middle);
        assert_eq!(plain.notation(), "3");
        let sub = Fingering {
            note: NoteId(0),
            finger: Finger::Middle,
            substitute: Some(Finger::Index),
        };
        assert_eq!(sub.notation(), "3-2");
    }
}
