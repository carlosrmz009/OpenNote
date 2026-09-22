//! Reading MusicXML into a [`Score`], and writing fingerings back into the
//! original document.
//!
//! The document tree is kept alive alongside the flattened score. Annotation walks
//! back to the exact `<note>` element each [`Note`] came from and inserts a
//! `<fingering>` into its `<notations><technical>`, creating those wrappers only
//! when they are missing. Nothing else in the file is touched, so an engraved
//! edition comes back out with its layout, slurs, pedalling and dynamics intact and
//! finger numbers added.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use on_hand::{Finger, Hand};
use musicxml::datatypes::{AboveBelow, StartStop, Step};
use musicxml::elements::{
    Fingering as XmlFingering, FingeringAttributes, GraceType, MeasureElement,
    NotationContentTypes, Notations, NotationsContents, NoteType, PartElement, ScorePartwise,
    Technical, TechnicalContents,
};

use crate::{Fingering, Note, NoteId, Score, SourceRef, Ticks, TieState, TICKS_PER_QUARTER};

/// Duration given to a grace note, which carries none of its own.
///
/// Grace notes still need a finger, and they still constrain the fingers around
/// them, so they cannot simply be dropped.
const GRACE_TICKS: Ticks = (TICKS_PER_QUARTER / 8) as Ticks;

/// Default loudness for notes imported from notation, which records dynamics as
/// marks rather than numbers.
const DEFAULT_VELOCITY: u8 = 72;

/// What MusicXML means by a dynamic of one hundred per cent.
///
/// The format states dynamics as a percentage of forte, and says forte is ninety.
const FORTE_VELOCITY: f64 = 90.0;

/// A MusicXML document and the score extracted from it.
pub struct MusicXmlDocument {
    document: ScorePartwise,
    score: Score,
}

impl MusicXmlDocument {
    /// Read a `.musicxml` or compressed `.mxl` file.
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = path
            .to_str()
            .ok_or_else(|| anyhow!("path is not valid UTF-8: {}", path.display()))?;
        let document = musicxml::read_score_partwise(text)
            .map_err(|e| anyhow!("{e}"))
            .with_context(|| format!("reading {}", path.display()))?;
        let score = extract(&document)?;
        Ok(Self { document, score })
    }

    /// Build from an already-parsed document, for tests and in-memory pipelines.
    pub fn from_document(document: ScorePartwise) -> Result<Self> {
        let score = extract(&document)?;
        Ok(Self { document, score })
    }

    /// The flattened score.
    pub fn score(&self) -> &Score {
        &self.score
    }

    /// Mutable access to the flattened score, for hand assignment and the like.
    pub fn score_mut(&mut self) -> &mut Score {
        &mut self.score
    }

    /// The underlying document.
    pub fn document(&self) -> &ScorePartwise {
        &self.document
    }

    /// Write fingerings onto the notes they belong to.
    ///
    /// Returns how many notes were annotated. Notes whose source is not this
    /// document, or which the parser could not locate, are skipped rather than
    /// silently mis-annotated.
    pub fn annotate(&mut self, fingerings: &[Fingering]) -> Result<usize> {
        let mut written = 0;
        for f in fingerings {
            let note = self
                .score
                .notes
                .get(f.note.index())
                .ok_or_else(|| anyhow!("fingering refers to unknown note {:?}", f.note))?;
            let SourceRef::MusicXml { part, measure, element } = note.source else {
                continue;
            };
            let placement = match note.hand {
                // Right-hand fingerings sit above the staff, left-hand below, which
                // is what editions do and what keeps them off the notes.
                Some(Hand::Left) => AboveBelow::Below,
                _ => AboveBelow::Above,
            };
            if annotate_note(&mut self.document, part, measure, element, f, placement)? {
                written += 1;
            }
        }
        Ok(written)
    }

    /// Write the document out. Pass `compressed` to produce a `.mxl`.
    pub fn write(&self, path: impl AsRef<Path>, compressed: bool) -> Result<()> {
        let path = path.as_ref();
        let text = path
            .to_str()
            .ok_or_else(|| anyhow!("path is not valid UTF-8: {}", path.display()))?;
        musicxml::write_partwise_score(text, &self.document, compressed, false)
            .map_err(|e| anyhow!("{e}"))
            .with_context(|| format!("writing {}", path.display()))
    }
}

/// Locate a note in the document tree and attach a fingering to it.
fn annotate_note(
    document: &mut ScorePartwise,
    part: usize,
    measure: usize,
    element: usize,
    fingering: &Fingering,
    placement: AboveBelow,
) -> Result<bool> {
    let Some(part) = document.content.part.get_mut(part) else {
        return Ok(false);
    };
    let Some(PartElement::Measure(measure)) = part.content.get_mut(measure) else {
        return Ok(false);
    };
    let Some(MeasureElement::Note(note)) = measure.content.get_mut(element) else {
        return Ok(false);
    };

    let attributes =
        FingeringAttributes { placement: Some(placement), ..Default::default() };
    let xml = XmlFingering { attributes, content: fingering.notation() };

    // Reuse an existing <technical> if the note has one, so we do not end up with
    // two of them; MusicXML allows it but engravers lay it out badly.
    for notations in note.content.notations.iter_mut() {
        for entry in notations.content.notations.iter_mut() {
            if let NotationContentTypes::Technical(technical) = entry {
                technical
                    .content
                    .retain(|t| !matches!(t, TechnicalContents::Fingering(_)));
                technical.content.push(TechnicalContents::Fingering(xml));
                return Ok(true);
            }
        }
    }

    let technical = Technical {
        attributes: Default::default(),
        content: vec![TechnicalContents::Fingering(xml)],
    };

    if let Some(notations) = note.content.notations.first_mut() {
        notations
            .content
            .notations
            .push(NotationContentTypes::Technical(technical));
    } else {
        note.content.notations.push(Notations {
            attributes: Default::default(),
            content: NotationsContents {
                footnote: None,
                level: None,
                notations: vec![NotationContentTypes::Technical(technical)],
            },
        });
    }
    Ok(true)
}

/// Semitone offset of each diatonic step above C.
fn step_semitones(step: &Step) -> i32 {
    match step {
        Step::C => 0,
        Step::D => 2,
        Step::E => 4,
        Step::F => 5,
        Step::G => 7,
        Step::A => 9,
        Step::B => 11,
    }
}

/// Convert a duration expressed in the current divisions into internal ticks.
fn to_ticks(divisions_value: i64, divisions_per_quarter: u32) -> Ticks {
    if divisions_per_quarter == 0 {
        return 0;
    }
    divisions_value * TICKS_PER_QUARTER as i64 / divisions_per_quarter as i64
}

/// Flatten a parsed document into a [`Score`].
fn extract(document: &ScorePartwise) -> Result<Score> {
    let mut score = Score {
        title: document
            .content
            .work
            .as_ref()
            .and_then(|w| w.content.work_title.as_ref())
            .map(|t| t.content.clone())
            .or_else(|| {
                document
                    .content
                    .movement_title
                    .as_ref()
                    .map(|t| t.content.clone())
            }),
        ..Default::default()
    };

    for (part_index, part) in document.content.part.iter().enumerate() {
        read_part(part_index, part, &mut score)?;
    }

    if score.notes.is_empty() {
        bail!("the score contains no sounding notes");
    }
    score.finalise();
    Ok(score)
}

/// Walk one part, tracking the divisions and the position cursor.
fn read_part(
    part_index: usize,
    part: &musicxml::elements::Part,
    score: &mut Score,
) -> Result<()> {
    let mut divisions: u32 = 1;
    // How loud the part is playing, and whether its foot is down. Both are stated by
    // `<sound>`, which appears on its own and inside a `<direction>`, and both persist
    // until something says otherwise.
    let mut velocity = DEFAULT_VELOCITY;
    let mut pressed: Option<Ticks> = None;
    // Where the current measure begins, in internal ticks.
    let mut measure_start: Ticks = 0;
    // Onset of the last note that advanced the cursor, so chord members can share it.
    let mut last_onset: Ticks = 0;

    for (measure_index, element) in part.content.iter().enumerate() {
        let PartElement::Measure(measure) = element else {
            continue;
        };
        // Absolute position of the next note, and how far into the measure anything
        // has reached. They differ whenever a <backup> rewinds the cursor to lay in
        // another voice, which is how two-staff piano parts are written.
        let mut cursor = measure_start;
        let mut measure_end = measure_start;

        for (element_index, item) in measure.content.iter().enumerate() {
            match item {
                MeasureElement::Attributes(attributes) => {
                    if let Some(d) = &attributes.content.divisions {
                        if *d.content > 0 {
                            divisions = *d.content;
                        }
                    }
                }
                MeasureElement::Backup(backup) => {
                    cursor -= to_ticks(*backup.content.duration.content as i64, divisions);
                }
                MeasureElement::Forward(forward) => {
                    cursor += to_ticks(*forward.content.duration.content as i64, divisions);
                    measure_end = measure_end.max(cursor);
                }
                MeasureElement::Sound(sound) => {
                    read_sound(sound, cursor, score, &mut velocity, &mut pressed);
                }
                MeasureElement::Direction(direction) => {
                    if let Some(sound) = &direction.content.sound {
                        read_sound(sound, cursor, score, &mut velocity, &mut pressed);
                    }
                }
                MeasureElement::Note(note) => {
                    let source = SourceRef::MusicXml {
                        part: part_index,
                        measure: measure_index,
                        element: element_index,
                    };
                    if let Some(parsed) =
                        read_note(note, divisions, cursor, last_onset, source, velocity)
                    {
                        if parsed.advances {
                            last_onset = cursor;
                            cursor += parsed.consumed;
                            measure_end = measure_end.max(cursor);
                        }
                        if let Some(note) = parsed.note {
                            score.notes.push(note);
                        }
                    }
                }
                _ => {}
            }
        }

        measure_start = measure_end.max(cursor);
    }
    Ok(())
}

fn insert_tempo(map: &mut crate::TempoMap, tick: Ticks, bpm: f64) {
    if bpm > 0.0 {
        map.insert(tick, (60_000_000.0 / bpm) as u32);
    }
}

/// The outcome of reading a single `<note>`.
struct ParsedNote {
    /// The note, unless the element was a rest.
    note: Option<Note>,
    /// Whether the cursor should move past this element.
    advances: bool,
    /// How far, in ticks.
    consumed: Ticks,
}

/// Read what a `<sound>` says about loudness, tempo and the damper.
///
/// Notation records dynamics as marks — a *p* under the stave — and leaves what they
/// mean to the player, so a file need not carry playable numbers at all. Where it does,
/// they are here, as a percentage of forte; and where it does not, everything stays at
/// the one default loudness, which is honest rather than invented.
fn read_sound(
    sound: &musicxml::elements::Sound,
    cursor: Ticks,
    score: &mut Score,
    velocity: &mut u8,
    pressed: &mut Option<Ticks>,
) {
    if let Some(tempo) = &sound.attributes.tempo {
        insert_tempo(&mut score.tempo, cursor, **tempo);
    }
    if let Some(dynamics) = &sound.attributes.dynamics {
        let loudness = FORTE_VELOCITY * **dynamics / 100.0;
        *velocity = loudness.round().clamp(1.0, 127.0) as u8;
    }
    if let Some(damper) = &sound.attributes.damper_pedal {
        // Half-pedalling is a percentage here rather than a controller value, but the
        // string either rings on or it does not as far as this is concerned.
        let down = match damper {
            musicxml::datatypes::YesNoNumber::Yes => true,
            musicxml::datatypes::YesNoNumber::No => false,
            musicxml::datatypes::YesNoNumber::Decimal(percent) => *percent >= 50.0,
        };
        match (down, *pressed) {
            (true, None) => *pressed = Some(cursor),
            (false, Some(from)) => {
                *pressed = None;
                if cursor > from {
                    score.pedal.push((from, cursor));
                }
            }
            _ => {}
        }
    }
}

fn read_note(
    note: &musicxml::elements::Note,
    divisions: u32,
    cursor: Ticks,
    last_onset: Ticks,
    source: SourceRef,
    velocity: u8,
) -> Option<ParsedNote> {
    use musicxml::elements::AudibleType;

    let (audible, duration_divisions, ties, is_chord, grace) = match &note.content.info {
        NoteType::Normal(info) => (
            &info.audible,
            *info.duration.content as i64,
            info.tie.as_slice(),
            info.chord.is_some(),
            false,
        ),
        NoteType::Grace(info) => {
            let (audible, ties, is_chord) = match &info.info {
                GraceType::Normal(n) => (&n.audible, n.tie.as_slice(), n.chord.is_some()),
                GraceType::Cue(c) => (&c.audible, [].as_slice(), c.chord.is_some()),
            };
            (audible, 0, ties, is_chord, true)
        }
        // Cue notes are not played, so they get no finger.
        NoteType::Cue(info) => {
            return Some(ParsedNote {
                note: None,
                advances: !info.chord.is_some(),
                consumed: to_ticks(*info.duration.content as i64, divisions),
            })
        }
    };

    let consumed = to_ticks(duration_divisions, divisions);

    let pitch = match audible {
        AudibleType::Pitch(pitch) => pitch,
        // Rests and unpitched percussion move the cursor but sound no note.
        _ => {
            return Some(ParsedNote {
                note: None,
                advances: !is_chord && !grace,
                consumed,
            })
        }
    };

    let octave = *pitch.content.octave.content as i32;
    let alter = pitch
        .content
        .alter
        .as_ref()
        .map(|a| *a.content as i32)
        .unwrap_or(0);
    let midi = (octave + 1) * 12 + step_semitones(&pitch.content.step.content) + alter;
    let midi = u8::try_from(midi.clamp(0, 127)).ok()?;

    let tie = TieState {
        start: ties.iter().any(|t| t.attributes.r#type == StartStop::Start),
        stop: ties.iter().any(|t| t.attributes.r#type == StartStop::Stop),
    };

    let onset = if is_chord { last_onset } else { cursor };
    let duration = if grace { GRACE_TICKS } else { consumed };

    Some(ParsedNote {
        note: Some(Note {
            id: NoteId(0),
            midi,
            onset,
            duration,
            onset_seconds: 0.0,
            duration_seconds: 0.0,
            staff: note.content.staff.as_ref().map(|s| *s.content as u8),
            voice: note
                .content
                .voice
                .as_ref()
                .and_then(|v| v.content.trim().parse::<u16>().ok()),
            hand: None,
            tie,
            grace,
            chord: is_chord,
            velocity,
            given_finger: existing_fingering(note),
            source,
        }),
        advances: !is_chord && !grace,
        consumed,
    })
}

/// A fingering the edition already carries, which is treated as a constraint.
fn existing_fingering(note: &musicxml::elements::Note) -> Option<Finger> {
    for notations in &note.content.notations {
        for entry in &notations.content.notations {
            let NotationContentTypes::Technical(technical) = entry else {
                continue;
            };
            for item in &technical.content {
                if let TechnicalContents::Fingering(f) = item {
                    // Editions write substitutions as "3-2"; the first digit is the
                    // finger that strikes the key.
                    let first = f.content.trim().split(['-', '_']).next()?;
                    if let Ok(n) = first.trim().parse::<u8>() {
                        return Finger::from_number(n);
                    }
                }
            }
        }
    }
    None
}

/// Group notes by their staff, which is how a piano part says which hand plays what.
pub fn staff_histogram(score: &Score) -> HashMap<u8, usize> {
    let mut counts = HashMap::new();
    for note in &score.notes {
        if let Some(staff) = note.staff {
            *counts.entry(staff).or_insert(0) += 1;
        }
    }
    counts
}
