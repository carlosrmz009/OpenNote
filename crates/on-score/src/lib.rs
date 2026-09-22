//! The score, reduced to what a fingering engine needs to know.
//!
//! A [`Score`] is a flat, time-ordered list of [`Note`]s carrying pitch, timing,
//! staff, voice and tie information, plus a back-reference into whichever file it
//! came from. That back-reference is what makes annotation lossless: fingerings are
//! written into the *original* document tree rather than into a regenerated one, so
//! everything the engraver put there — layout, slurs, dynamics, articulations —
//! survives untouched.

pub mod hands;
pub mod midi_io;
pub mod musicxml_io;
pub mod timing;

pub use hands::assign_hands;
pub use midi_io::MidiDocument;
pub use musicxml_io::MusicXmlDocument;
pub use timing::TempoMap;

use on_hand::{Finger, Hand};

/// Ticks per quarter note used throughout the internal representation.
///
/// MusicXML files declare their own divisions, and can change them mid-piece; MIDI
/// files declare theirs in the header. Both are normalised to this grid on import
/// so that downstream code never has to think about it. 960 divides cleanly by 2,
/// 3, 4, 5, 6 and 8, which covers every tuplet in the common repertoire.
pub const TICKS_PER_QUARTER: u32 = 960;

/// A position or duration on the internal tick grid.
pub type Ticks = i64;

/// Stable identifier for a note within a [`Score`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoteId(pub u32);

impl NoteId {
    /// Index into [`Score::notes`].
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// How a note connects to its neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TieState {
    /// A tie begins on this note.
    pub start: bool,
    /// A tie ends on this note.
    pub stop: bool,
}

impl TieState {
    /// Whether this note is a continuation of an earlier one, and so must keep
    /// whatever finger that one was given.
    pub fn is_continuation(&self) -> bool {
        self.stop
    }
}

/// Where a note came from in its source document, so a fingering can be written
/// back onto exactly that note.
///
/// (See [`Score::release_seconds`] for why a tie needs following rather than reading.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRef {
    /// A note inside a MusicXML `<part>` / `<measure>`.
    MusicXml {
        /// Index into the score's parts.
        part: usize,
        /// Index into the part's elements, identifying the measure.
        measure: usize,
        /// Index into the measure's elements.
        element: usize,
    },
    /// A note-on event in a MIDI file.
    Midi {
        /// Track index.
        track: usize,
        /// Index of the note-on event within that track.
        event: usize,
    },
}

/// One sounding note.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    /// Identifier, equal to this note's index in [`Score::notes`].
    pub id: NoteId,
    /// MIDI pitch number, where 60 is middle C.
    pub midi: u8,
    /// Onset on the internal tick grid.
    pub onset: Ticks,
    /// Duration on the internal tick grid. Grace notes get a token duration.
    pub duration: Ticks,
    /// Onset in seconds, following the tempo map.
    pub onset_seconds: f64,
    /// Duration in seconds, following the tempo map.
    pub duration_seconds: f64,
    /// Staff number within the part, when the source says. Staff 1 is the upper one.
    pub staff: Option<u8>,
    /// Voice within the staff, when the source says.
    pub voice: Option<u16>,
    /// Which hand plays it. Resolved during import; see [`hands`].
    pub hand: Option<Hand>,
    /// Tie state.
    pub tie: TieState,
    /// Whether this is a grace note.
    pub grace: bool,
    /// Whether the note is part of a chord, i.e. shares an onset with the note
    /// before it in the same voice.
    pub chord: bool,
    /// Loudness, 1..=127. MusicXML sources get a default.
    pub velocity: u8,
    /// A fingering already present in the source, if any. Respected as a constraint
    /// rather than overwritten, so partially fingered editions can be completed.
    pub given_finger: Option<Finger>,
    /// Where this note lives in its source document.
    pub source: SourceRef,
}

impl Note {
    /// Tick at which the note stops sounding.
    pub fn offset(&self) -> Ticks {
        self.onset + self.duration
    }

    /// Second at which the note stops sounding.
    pub fn offset_seconds(&self) -> f64 {
        self.onset_seconds + self.duration_seconds
    }

    /// Whether this note is still sounding at the given tick.
    pub fn sounds_at(&self, tick: Ticks) -> bool {
        self.onset <= tick && tick < self.offset()
    }

    /// Whether this note lands on a black key.
    pub fn is_black(&self) -> bool {
        on_hand::keyboard::is_black(self.midi)
    }
}

/// A piece of music, flattened.
#[derive(Debug, Clone, Default)]
pub struct Score {
    /// Title, if the source names one.
    pub title: Option<String>,
    /// Every sounding note, sorted by onset and then by pitch.
    pub notes: Vec<Note>,
    /// Tempo changes over the piece.
    pub tempo: TempoMap,
    /// When the sustain pedal is down, in ticks: each span is one press.
    ///
    /// The pedal changes what is *heard* and nothing else. It does not change the
    /// fingering, and deliberately so — the standard advice is to work a fingering out
    /// with no pedal at all, precisely so that everything the score holds is held by a
    /// finger rather than by the foot. So nothing in the hand model or the search reads
    /// this; only the sound does.
    pub pedal: Vec<(Ticks, Ticks)>,
}

impl Score {
    /// When the string is actually damped, which the pedal can put off.
    ///
    /// A damper cannot fall while the pedal holds it off the string, so a note whose key
    /// is released during a press goes on ringing until the press ends. This is what the
    /// sound wants; anything about the hands wants [`Score::release_seconds`], which is
    /// when the finger actually let go.
    pub fn damped_seconds(&self, id: NoteId) -> f64 {
        let released = self.release_seconds(id);
        self.pedal
            .iter()
            .find(|(down, up)| {
                // Down before the key came up, and still down when it did.
                let down = self.tempo.seconds_at(*down);
                let up = self.tempo.seconds_at(*up);
                down <= released + 1e-9 && up > released
            })
            .map(|(_, up)| self.tempo.seconds_at(*up).max(released))
            .unwrap_or(released)
    }

    /// When a note actually stops sounding, following any ties.
    ///
    /// A note's own `offset_seconds` is where its written duration ends, which for a
    /// tied note is not where the sound stops — the notes it is tied to carry it on, and
    /// they are separate notes in the score. Anything asking what a hand is holding has
    /// to follow the tie, or a note tied across a bar line stops constraining the hand
    /// halfway through.
    pub fn release_seconds(&self, id: NoteId) -> f64 {
        let note = self.note(id);
        let mut end = note.offset_seconds();
        if !note.tie.start {
            return end;
        }
        // Continuations of the same pitch in the same hand, each picking up where the
        // last left off. Ties are written note to note, so this walks the chain.
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

    /// Look up a note by identifier.
    pub fn note(&self, id: NoteId) -> &Note {
        &self.notes[id.index()]
    }

    /// Mutable access to a note.
    pub fn note_mut(&mut self, id: NoteId) -> &mut Note {
        &mut self.notes[id.index()]
    }

    /// Every note assigned to one hand, in time order.
    pub fn hand_notes(&self, hand: Hand) -> impl Iterator<Item = &Note> {
        self.notes.iter().filter(move |n| n.hand == Some(hand))
    }

    /// Total length of the piece in ticks.
    pub fn duration(&self) -> Ticks {
        self.notes.iter().map(|n| n.offset()).max().unwrap_or(0)
    }

    /// Total length of the piece in seconds.
    pub fn duration_seconds(&self) -> f64 {
        self.notes.iter().map(|n| n.offset_seconds()).fold(0.0, f64::max)
    }

    /// Sort notes into canonical order and renumber their identifiers.
    ///
    /// Everything downstream assumes this ordering, so importers call it once they
    /// have collected every note.
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

    /// Drop notes no piano has a key for.
    ///
    /// MIDI carries 128 pitches and a piano has 88 of them, so a file can perfectly
    /// legally ask for notes that do not exist — arrangements transposed out of range,
    /// percussion left on a piano track, or the bottom and top of a synthesised part
    /// that was never meant for hands. Everything downstream of here assumes a real
    /// keyboard: the geometry, the span tables and the hand model all index by key.
    ///
    /// Dropping them is the only honest answer. Clamping would move a note to a pitch
    /// the composer did not write, and transposing it by octaves would change the
    /// music; an instrument that cannot play a note cannot play it. They are removed
    /// here rather than in each reader because every path into a score ends in
    /// [`Score::finalise`], and a guard in one place is a guard that cannot be
    /// forgotten in the next reader somebody writes.
    fn drop_unplayable(&mut self) {
        let range = on_hand::keyboard::MIDI_LOWEST..=on_hand::keyboard::MIDI_HIGHEST;
        self.notes.retain(|note| range.contains(&note.midi));
    }

    /// Refresh every note's wall-clock timing from the tempo map.
    pub fn recompute_seconds(&mut self) {
        for note in &mut self.notes {
            note.onset_seconds = self.tempo.seconds_at(note.onset);
            note.duration_seconds =
                self.tempo.seconds_at(note.offset()) - note.onset_seconds;
        }
    }

    /// Group notes into chords: maximal sets sharing a hand and an onset.
    ///
    /// This is the unit the fingering solver works in, because a chord's fingers are
    /// chosen together rather than one at a time.
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

/// Notes played together by one hand.
#[derive(Debug, Clone, PartialEq)]
pub struct Chord {
    /// Shared onset tick.
    pub onset: Ticks,
    /// Shared onset in seconds.
    pub onset_seconds: f64,
    /// The notes, low to high.
    pub notes: Vec<NoteId>,
}

impl Chord {
    /// How many notes are in the chord.
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    /// Whether the chord is empty, which should not happen for a constructed score.
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }
}

/// A finger assigned to a note, plus how it should be notated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingering {
    /// The note.
    pub note: NoteId,
    /// The finger that plays it.
    pub finger: Finger,
    /// A second finger that takes over while the note is still held, for the
    /// substitutions that make legato possible on held notes.
    pub substitute: Option<Finger>,
}

impl Fingering {
    /// A plain fingering with no substitution.
    pub fn new(note: NoteId, finger: Finger) -> Self {
        Self { note, finger, substitute: None }
    }

    /// How a pianist would write it: `3` or, for a substitution, `3-2`.
    pub fn notation(&self) -> String {
        match self.substitute {
            Some(s) => format!("{}-{}", self.finger.number(), s.number()),
            None => self.finger.number().to_string(),
        }
    }
}

/// A score on disk, in whichever format it happens to be.
///
/// Reading is by extension rather than by sniffing the contents, because the two
/// formats this reads are told apart perfectly well that way and a file with the
/// wrong extension is a mistake worth reporting rather than working around.
pub enum Document {
    /// MusicXML, plain or compressed.
    MusicXml(Box<MusicXmlDocument>),
    /// A MIDI file.
    Midi(Box<MidiDocument>),
}

impl Document {
    /// The file extensions this can read, for a file dialog's filter.
    pub const EXTENSIONS: [&'static str; 5] = ["musicxml", "mxl", "xml", "mid", "midi"];

    /// Read a score, choosing the reader by file extension.
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

    /// The score itself.
    pub fn score(&self) -> &Score {
        match self {
            Document::MusicXml(d) => d.score(),
            Document::Midi(d) => d.score(),
        }
    }

    /// Mutable access, for assigning hands.
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
        // All the way along the chain, not one link of it.
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
