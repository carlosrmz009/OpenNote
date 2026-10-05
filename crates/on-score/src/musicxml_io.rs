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

const GRACE_TICKS: Ticks = (TICKS_PER_QUARTER / 8) as Ticks;

const DEFAULT_VELOCITY: u8 = 72;

const FORTE_VELOCITY: f64 = 90.0;

fn marked_velocity(mark: &musicxml::elements::DynamicsType) -> Option<(u8, u8)> {
    use musicxml::elements::DynamicsType as D;
    Some(match mark {
        D::Pppppp(_) | D::Ppppp(_) | D::Pppp(_) => (20, 0),
        D::Ppp(_) => (30, 0),
        D::Pp(_) => (40, 0),
        D::P(_) => (52, 0),
        D::Mp(_) => (64, 0),
        D::Mf(_) => (76, 0),
        D::F(_) => (90, 0),
        D::Ff(_) => (104, 0),
        D::Fff(_) | D::Ffff(_) | D::Fffff(_) | D::Ffffff(_) => (116, 0),
        D::Fp(_) => (52, crate::art::ACCENT),
        D::Sf(_) | D::Sfz(_) | D::Sffz(_) | D::Fz(_) | D::Rf(_) | D::Rfz(_) => (0, crate::art::STRONG_ACCENT),
        D::Sfp(_) | D::Sfpp(_) => (52, crate::art::STRONG_ACCENT),
        _ => return None,
    })
}

fn note_marks(note: &musicxml::elements::Note) -> (u8, Vec<i8>) {
    use musicxml::elements::ArticulationsType as A;
    let mut flags = 0u8;
    let mut slurs = Vec::new();
    for notations in &note.content.notations {
        for entry in &notations.content.notations {
            match entry {
                NotationContentTypes::Articulations(list) => {
                    for a in &list.content {
                        flags |= match a {
                            A::Staccato(_) | A::Spiccato(_) => crate::art::STACCATO,
                            A::Staccatissimo(_) => crate::art::STACCATISSIMO,
                            A::Tenuto(_) => crate::art::TENUTO,
                            A::DetachedLegato(_) => crate::art::TENUTO | crate::art::STACCATO,
                            A::Accent(_) | A::Stress(_) => crate::art::ACCENT,
                            A::StrongAccent(_) => crate::art::STRONG_ACCENT,
                            _ => 0,
                        };
                    }
                }
                NotationContentTypes::Fermata(_) => flags |= crate::art::FERMATA,
                NotationContentTypes::Dynamics(dynamics) => {
                    for mark in &dynamics.content {
                        flags |= marked_velocity(mark).map_or(0, |(_, f)| f);
                    }
                }
                NotationContentTypes::Slur(slur) => slurs.push(match slur.attributes.r#type {
                    musicxml::datatypes::StartStopContinue::Start => 1,
                    musicxml::datatypes::StartStopContinue::Stop => -1,
                    musicxml::datatypes::StartStopContinue::Continue => 0,
                }),
                _ => {}
            }
        }
    }
    (flags, slurs)
}

pub struct MusicXmlDocument {
    document: ScorePartwise,
    score: Score,
}

impl MusicXmlDocument {
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

    pub fn from_document(document: ScorePartwise) -> Result<Self> {
        let score = extract(&document)?;
        Ok(Self { document, score })
    }

    pub fn score(&self) -> &Score {
        &self.score
    }

    pub fn score_mut(&mut self) -> &mut Score {
        &mut self.score
    }

    pub fn document(&self) -> &ScorePartwise {
        &self.document
    }

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
                Some(Hand::Left) => AboveBelow::Below,
                _ => AboveBelow::Above,
            };
            if annotate_note(&mut self.document, part, measure, element, f, placement)? {
                written += 1;
            }
        }
        Ok(written)
    }

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

fn to_ticks(divisions_value: i64, divisions_per_quarter: u32) -> Ticks {
    if divisions_per_quarter == 0 {
        return 0;
    }
    divisions_value * TICKS_PER_QUARTER as i64 / divisions_per_quarter as i64
}

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

fn read_part(
    part_index: usize,
    part: &musicxml::elements::Part,
    score: &mut Score,
) -> Result<()> {
    let mut divisions: u32 = 1;
    let mut velocity = DEFAULT_VELOCITY;
    let mut pressed: Option<Ticks> = None;
    let mut measure_start: Ticks = 0;
    let mut last_onset: Ticks = 0;
    let mut slurs: Vec<Ticks> = Vec::new();
    let mut wedge: Option<(Ticks, i8)> = None;

    for (measure_index, element) in part.content.iter().enumerate() {
        let PartElement::Measure(measure) = element else {
            continue;
        };
        if part_index == 0 {
            score.marks.measures.push(measure_start);
        }
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
                    for kind in &direction.content.direction_type {
                        use musicxml::elements::DirectionTypeContents as T;
                        match &kind.content {
                            T::Dynamics(list) => {
                                for mark in list.iter().flat_map(|d| d.content.iter()) {
                                    if let Some((level, _)) = marked_velocity(mark) {
                                        if level > 0 {
                                            velocity = level;
                                            score.marks.dynamics.push((cursor, level));
                                        }
                                    }
                                }
                            }
                            T::Wedge(w) => {
                                use musicxml::datatypes::WedgeType as W;
                                match w.attributes.r#type {
                                    W::Crescendo => wedge = Some((cursor, 1)),
                                    W::Diminuendo => wedge = Some((cursor, -1)),
                                    W::Stop => {
                                        if let Some((from, way)) = wedge.take() {
                                            if cursor > from {
                                                score.marks.hairpins.push((from, cursor, way));
                                            }
                                        }
                                    }
                                    W::Continue => {}
                                }
                            }
                            _ => {}
                        }
                    }
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
                        if let Some(parsed_note) = parsed.note {
                            let (flags, slur_marks) = note_marks(note);
                            if flags != 0 {
                                score.marks.articulations.insert(source, flags);
                            }
                            for mark in slur_marks {
                                if mark > 0 {
                                    slurs.push(parsed_note.onset);
                                } else if mark < 0 {
                                    if let Some(from) = slurs.pop() {
                                        if parsed_note.onset > from {
                                            score.marks.slurs.push((from, parsed_note.onset));
                                        }
                                    }
                                }
                            }
                            score.notes.push(parsed_note);
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

struct ParsedNote {
    note: Option<Note>,
    advances: bool,
    consumed: Ticks,
}

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

fn existing_fingering(note: &musicxml::elements::Note) -> Option<Finger> {
    for notations in &note.content.notations {
        for entry in &notations.content.notations {
            let NotationContentTypes::Technical(technical) = entry else {
                continue;
            };
            for item in &technical.content {
                if let TechnicalContents::Fingering(f) = item {
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

pub fn staff_histogram(score: &Score) -> HashMap<u8, usize> {
    let mut counts = HashMap::new();
    for note in &score.notes {
        if let Some(staff) = note.staff {
            *counts.entry(staff).or_insert(0) += 1;
        }
    }
    counts
}
