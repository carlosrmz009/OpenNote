use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use on_fingering::Placement;
use on_fingering::playability::CHORD_SECONDS;
use on_hand::{Finger, Hand};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct FingeredNote {
    pub midi: u8,
    pub onset: f64,
    #[serde(default = "assumed_duration")]
    pub duration: f64,
    pub hand: HandLabel,
    pub finger: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

pub const ASSUMED_DURATION: f64 = 0.25;

fn assumed_duration() -> f64 {
    ASSUMED_DURATION
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HandLabel {
    Left,
    Right,
}

impl From<HandLabel> for Hand {
    fn from(label: HandLabel) -> Self {
        match label {
            HandLabel::Left => Hand::Left,
            HandLabel::Right => Hand::Right,
        }
    }
}

impl From<Hand> for HandLabel {
    fn from(hand: Hand) -> Self {
        match hand {
            Hand::Left => HandLabel::Left,
            Hand::Right => HandLabel::Right,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Piece {
    pub piece: String,
    pub annotator: String,
    pub source: String,
    pub notes: Vec<FingeredNote>,
}

impl Piece {
    pub fn trusted(&self, floor: f32) -> Piece {
        let mut piece = self.clone();
        for note in &mut piece.notes {
            if note.confidence.is_some_and(|c| c < floor) {
                note.finger = None;
            }
        }
        piece
    }

    pub fn voice(&self, hand: Hand) -> Vec<Placement> {
        let label = HandLabel::from(hand);
        let mut out: Vec<(f64, Placement)> = Vec::new();
        for note in self.notes.iter().filter(|note| note.hand == label) {
            let Some(finger) = note.finger.and_then(Finger::from_number) else {
                continue;
            };
            let placement = Placement::new(note.midi, finger);
            match out.last_mut() {
                Some((onset, last)) if (note.onset - *onset).abs() <= CHORD_SECONDS => {
                    if placement.midi > last.midi {
                        *last = placement;
                    }
                }
                _ => out.push((note.onset, placement)),
            }
        }
        out.into_iter().map(|(_, placement)| placement).collect()
    }
}

pub struct Corpus {
    root: PathBuf,
}

impl Corpus {
    pub fn at(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)
            .with_context(|| format!("making the corpus directory {}", root.display()))?;
        Ok(Self { root })
    }

    fn data_dir(&self) -> PathBuf {
        self.root.join("normalised")
    }

    pub fn load(&self) -> Result<Vec<Piece>> {
        let directory = self.data_dir();
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut pieces = Vec::new();
        let mut files: Vec<PathBuf> = std::fs::read_dir(&directory)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|e| e == "jsonl"))
            .collect();
        files.sort();
        for file in files {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            for (line_number, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let piece: Piece = serde_json::from_str(line).with_context(|| {
                    format!("{}, line {}", file.display(), line_number + 1)
                })?;
                pieces.push(piece);
            }
        }
        Ok(pieces)
    }

    pub fn add_piece(&self, piece: &Piece, label: Option<&str>) -> Result<PathBuf> {
        let name = label.map(sanitise).unwrap_or_else(|| sanitise(&piece.piece));
        let directory = self.data_dir();
        std::fs::create_dir_all(&directory)?;
        let file = directory.join(format!("{name}.jsonl"));
        let mut text = serde_json::to_string(piece)?;
        text.push('\n');
        use std::io::Write as _;
        let mut out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)
            .with_context(|| format!("writing {}", file.display()))?;
        out.write_all(text.as_bytes())?;
        Ok(file)
    }

    pub fn add(&self, path: &Path, label: Option<&str>) -> Result<Ingested> {
        let mut pieces = Vec::new();
        let mut skipped = Vec::new();
        collect(path, &mut pieces, &mut skipped)?;
        if pieces.is_empty() {
            bail!(
                "found no fingering data under {}.\n\
                 This reads PIG `_fingering.txt` files and MusicXML that already has \
                 <fingering> marks on its notes.",
                path.display()
            );
        }

        let name = label
            .map(str::to_owned)
            .or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .map(sanitise)
            })
            .unwrap_or_else(|| "added".into());

        let directory = self.data_dir();
        std::fs::create_dir_all(&directory)?;
        let file = directory.join(format!("{name}.jsonl"));
        let mut text = String::new();
        for piece in &pieces {
            text.push_str(&serde_json::to_string(piece)?);
            text.push('\n');
        }
        std::fs::write(&file, text)
            .with_context(|| format!("writing {}", file.display()))?;

        let notes = pieces.iter().map(|p| p.notes.len()).sum();
        let distinct: std::collections::BTreeSet<&str> =
            pieces.iter().map(|p| p.piece.as_str()).collect();
        Ok(Ingested {
            file,
            annotations: pieces.len(),
            pieces: distinct.len(),
            notes,
            skipped,
        })
    }
}

pub struct Ingested {
    pub file: PathBuf,
    pub annotations: usize,
    pub pieces: usize,
    pub notes: usize,
    pub skipped: Vec<String>,
}

fn sanitise(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect()
}

fn collect(path: &Path, pieces: &mut Vec<Piece>, skipped: &mut Vec<String>) -> Result<()> {
    if path.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(path)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .collect();
        entries.sort();
        for entry in entries {
            collect(&entry, pieces, skipped)?;
        }
        return Ok(());
    }

    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let read = match extension.as_str() {
        "txt" => read_pig(path),
        "musicxml" | "mxl" | "xml" => std::panic::catch_unwind(|| read_musicxml(path))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("the MusicXML parser crashed on this file"))),
        _ => return Ok(()),
    };
    match read {
        Ok(Some(piece)) => pieces.push(piece),
        Ok(None) => {}
        Err(error) => skipped.push(format!("{}: {error:#}", path.display())),
    }
    Ok(())
}

fn read_pig(path: &Path) -> Result<Option<Piece>> {
    let text = std::fs::read_to_string(path)?;
    let mut notes = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').filter(|f| !f.is_empty()).collect();
        if fields.len() < 8 {
            continue;
        }
        let Ok(onset) = fields[1].parse::<f64>() else {
            continue;
        };
        let duration = fields[2]
            .parse::<f64>()
            .map_or(ASSUMED_DURATION, |offset| (offset - onset).max(0.0));
        let Some(midi) = spelled_pitch_to_midi(fields[3]) else {
            continue;
        };
        let raw = fields[7].split('_').next().unwrap_or_default();
        let signed = raw.parse::<i32>().unwrap_or(0);
        let finger = match signed.unsigned_abs() as u8 {
            f @ 1..=5 => Some(f),
            _ => None,
        };
        let hand = match signed {
            0 => match fields[6] {
                "0" => HandLabel::Right,
                "1" => HandLabel::Left,
                _ if midi < 60 => HandLabel::Left,
                _ => HandLabel::Right,
            },
            n if n > 0 => HandLabel::Right,
            _ => HandLabel::Left,
        };
        notes.push(FingeredNote {
            midi,
            onset,
            duration,
            hand,
            finger,
            confidence: None,
        });
    }
    if notes.is_empty() {
        return Ok(None);
    }
    notes.sort_by(|a, b| a.onset.total_cmp(&b.onset).then(a.midi.cmp(&b.midi)));

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .trim_end_matches("_fingering")
        .to_string();
    let (piece, annotator) = match stem.rsplit_once('-') {
        Some((piece, annotator)) => (piece.to_string(), annotator.to_string()),
        None => (stem.clone(), "1".to_string()),
    };
    Ok(Some(Piece {
        piece,
        annotator,
        source: path.display().to_string(),
        notes,
    }))
}

fn spelled_pitch_to_midi(spelled: &str) -> Option<u8> {
    let mut chars = spelled.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let step = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest: String = chars.collect();
    let mut alteration = 0i32;
    let mut digits = String::new();
    for c in rest.chars() {
        match c {
            '#' => alteration += 1,
            'b' | 'B' => alteration -= 1,
            '-' => digits.push('-'),
            c if c.is_ascii_digit() => digits.push(c),
            _ => return None,
        }
    }
    let octave: i32 = digits.parse().ok()?;
    let value = (octave + 1) * 12 + step + alteration;
    u8::try_from(value).ok().filter(|v| (21..=108).contains(v))
}

fn read_musicxml(path: &Path) -> Result<Option<Piece>> {
    let document = on_score::MusicXmlDocument::read(path)?;
    let mut score = document.score().clone();
    on_score::assign_hands(&mut score, &on_score::hands::HandAssignment::default());

    let notes: Vec<FingeredNote> = score
        .notes
        .iter()
        .filter_map(|note| {
            Some(FingeredNote {
                midi: note.midi,
                onset: note.onset_seconds,
                duration: note.duration_seconds,
                hand: HandLabel::from(note.hand?),
                finger: note.given_finger.map(|f| f.number()),
                confidence: None,
            })
        })
        .collect();
    if notes.iter().all(|note| note.finger.is_none()) {
        return Ok(None);
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    Ok(Some(Piece {
        piece: stem.clone(),
        annotator: "edition".into(),
        source: path.display().to_string(),
        notes,
    }))
}

pub fn read(path: &Path) -> Result<(Vec<Piece>, Vec<String>)> {
    let mut pieces = Vec::new();
    let mut skipped = Vec::new();
    collect(path, &mut pieces, &mut skipped)?;
    Ok((pieces, skipped))
}

pub fn by_piece(pieces: &[Piece]) -> BTreeMap<&str, Vec<&Piece>> {
    let mut grouped: BTreeMap<&str, Vec<&Piece>> = BTreeMap::new();
    for piece in pieces {
        grouped.entry(piece.piece.as_str()).or_default().push(piece);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spelled_pitches_become_midi_numbers() {
        assert_eq!(spelled_pitch_to_midi("C4"), Some(60));
        assert_eq!(spelled_pitch_to_midi("A0"), Some(21));
        assert_eq!(spelled_pitch_to_midi("C8"), Some(108));
        assert_eq!(spelled_pitch_to_midi("Bb3"), Some(58));
        assert_eq!(spelled_pitch_to_midi("F#4"), Some(66));
        assert_eq!(spelled_pitch_to_midi("Cb4"), Some(59));
        assert_eq!(spelled_pitch_to_midi("C-1"), None);
        assert_eq!(spelled_pitch_to_midi("H4"), None);
        assert_eq!(spelled_pitch_to_midi(""), None);
    }

    #[test]
    fn reading_a_dataset_stores_nothing() {
        let dir = std::env::temp_dir().join("opennote-read-only-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("099-1_fingering.txt"),
            "0	0.0	0.5	C4	80	80	0	1
             1	0.5	1.0	E4	80	80	0	3
",
        )
        .unwrap();

        let before: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
        let (pieces, _) = read(&dir).unwrap();
        let after: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();

        assert_eq!(pieces.len(), 1, "should have read the one piece");
        assert_eq!(pieces[0].notes.len(), 2);
        assert_eq!(before, after, "reading a dataset created or removed a file");
    }

    #[test]
    fn a_doubtful_reading_is_dropped_and_a_person_is_always_believed() {
        let note = |finger: u8, confidence: Option<f32>| FingeredNote {
            midi: 60,
            onset: 0.0,
            duration: ASSUMED_DURATION,
            hand: HandLabel::Right,
            finger: Some(finger),
            confidence,
        };
        let piece = Piece {
            piece: "p".into(),
            annotator: "video".into(),
            source: "somewhere".into(),
            notes: vec![note(1, None), note(2, Some(0.9)), note(3, Some(0.2))],
        };

        let kept = piece.trusted(0.5);
        assert_eq!(kept.notes.len(), 3, "the notes stay; only the doubtful labels go");
        assert_eq!(kept.notes[0].finger, Some(1), "a person is always believed");
        assert_eq!(kept.notes[1].finger, Some(2), "a confident reading is kept");
        assert_eq!(kept.notes[2].finger, None, "a doubtful one is dropped");
        assert_eq!(piece.trusted(0.0).notes[2].finger, Some(3), "a floor of 0 keeps everything");
    }

    #[test]
    fn a_corpus_written_before_confidence_existed_still_reads() {
        let old = r#"{"midi":60,"onset":0.0,"duration":0.25,"hand":"right","finger":3}"#;
        let note: FingeredNote = serde_json::from_str(old).unwrap();
        assert_eq!(note.confidence, None);
        assert_eq!(note.finger, Some(3));
        assert!(!serde_json::to_string(&note).unwrap().contains("confidence"));
    }

    #[test]
    fn a_pig_file_is_read_with_its_hands_the_right_way_round() {
        let dir = std::env::temp_dir().join("opennote-corpus-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("042-2_fingering.txt");
        std::fs::write(
            &path,
            "//Version: PIG v1.0\n\
             0\t0.0\t0.5\tC4\t80\t80\t0\t1\n\
             1\t0.5\t1.0\tE4\t80\t80\t0\t3\n\
             2\t1.0\t1.5\tC3\t80\t80\t1\t-5\n\
             3\t1.5\t2.0\tG3\t80\t80\t1\t-2_-1\n",
        )
        .unwrap();

        let piece = read_pig(&path).unwrap().unwrap();
        assert_eq!(piece.piece, "042");
        assert_eq!(piece.annotator, "2");
        assert_eq!(piece.notes.len(), 4);
        assert_eq!(
            piece.notes[0],
            FingeredNote {
                midi: 60,
                onset: 0.0,
                duration: 0.5,
                hand: HandLabel::Right,
                finger: Some(1),
                confidence: None,
            }
        );
        assert_eq!(piece.notes[3].hand, HandLabel::Left);
        assert_eq!(piece.notes[3].finger, Some(2));

        let right = piece.voice(Hand::Right);
        assert_eq!(right.len(), 2);
        assert_eq!(right[0].midi, 60);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_corpus_round_trips_through_its_directory() {
        let dir = std::env::temp_dir().join("opennote-corpus-round-trip");
        let _ = std::fs::remove_dir_all(&dir);
        let source = dir.join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join("001-1_fingering.txt"),
            "0\t0.0\t0.5\tC4\t80\t80\t0\t1\n1\t0.5\t1.0\tD4\t80\t80\t0\t2\n",
        )
        .unwrap();
        std::fs::write(source.join("README.md"), "not data").unwrap();

        let corpus = Corpus::at(dir.join("corpus")).unwrap();
        let report = corpus.add(&source, Some("test")).unwrap();
        assert_eq!(report.annotations, 1);
        assert_eq!(report.notes, 2);
        assert!(report.skipped.is_empty());

        let loaded = corpus.load().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].notes.len(), 2);
        assert_eq!(loaded[0].piece, "001");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_empty_corpus_is_not_an_error() {
        let dir = std::env::temp_dir().join("opennote-corpus-empty");
        let _ = std::fs::remove_dir_all(&dir);
        let corpus = Corpus::at(&dir).unwrap();
        assert!(corpus.load().unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_corpus_file_written_before_lengths_still_reads() {
        let old = r#"{"midi":60,"onset":0.5,"hand":"right","finger":1}"#;
        let note: FingeredNote = serde_json::from_str(old).unwrap();
        assert_eq!(note.duration, ASSUMED_DURATION);

        let new = r#"{"midi":60,"onset":0.5,"duration":2.0,"hand":"right","finger":1}"#;
        let note: FingeredNote = serde_json::from_str(new).unwrap();
        assert_eq!(note.duration, 2.0);
    }

    #[test]
    fn a_chord_contributes_one_note_to_the_line_and_an_arpeggio_contributes_three() {
        let note = |midi: u8, finger: u8, onset: f64| FingeredNote {
            midi,
            onset,
            duration: ASSUMED_DURATION,
            hand: HandLabel::Right,
            finger: Some(finger),
            confidence: None,
        };

        let chord = Piece {
            piece: "chord".into(),
            annotator: "1".into(),
            source: "test".into(),
            notes: vec![note(60, 1, 0.0), note(64, 3, 0.0), note(67, 5, 0.0)],
        };
        let line = chord.voice(Hand::Right);
        assert_eq!(line.len(), 1, "one moment, one note on the line");
        assert_eq!(line[0].midi, 67, "the top of the chord is the voice");
        assert_eq!(line[0].finger.number(), 5);

        let arpeggio = Piece {
            notes: vec![note(60, 1, 0.0), note(64, 3, 0.5), note(67, 5, 1.0)],
            ..chord.clone()
        };
        let line = arpeggio.voice(Hand::Right);
        assert_eq!(line.len(), 3, "an arpeggio is three moments");
        assert_eq!(
            line.iter().map(|p| p.midi).collect::<Vec<_>>(),
            vec![60, 64, 67]
        );
    }

    #[test]
    fn a_chord_survives_being_played_slightly_unevenly() {
        let note = |midi: u8, finger: u8, onset: f64| FingeredNote {
            midi,
            onset,
            duration: ASSUMED_DURATION,
            hand: HandLabel::Right,
            finger: Some(finger),
            confidence: None,
        };
        let piece = Piece {
            piece: "uneven".into(),
            annotator: "1".into(),
            source: "test".into(),
            notes: vec![note(60, 1, 0.0), note(64, 3, 0.004), note(67, 5, 0.009)],
        };
        assert_eq!(piece.voice(Hand::Right).len(), 1);
    }

    #[test]
    fn a_partly_annotated_source_keeps_the_notes_nobody_fingered() {
        let dir = std::env::temp_dir().join("opennote-corpus-partial");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("partial_fingering.txt");
        std::fs::write(
            &path,
            "//Version: PIG 1.0\n\
             0\t0.0\t0.5\tC4\t0\t0\t0\t0\n\
             1\t0.5\t1.0\tD4\t0\t0\t0\t2\n\
             2\t1.0\t1.5\tE4\t0\t0\t0\t0\n\
             3\t1.5\t2.0\tF4\t0\t0\t0\t4\n\
             4\t2.0\t2.5\tG4\t0\t0\t0\t0\n",
        )
        .unwrap();

        let piece = read_pig(&path).unwrap().unwrap();
        assert_eq!(piece.notes.len(), 5, "every note survives, annotated or not");
        let fingers: Vec<Option<u8>> = piece.notes.iter().map(|n| n.finger).collect();
        assert_eq!(fingers, vec![None, Some(2), None, Some(4), None]);

        let placements = piece.voice(Hand::Right);
        assert_eq!(
            placements.len(),
            2,
            "only the annotated notes teach the model anything"
        );
    }

    #[test]
    fn an_unannotated_note_takes_its_hand_from_the_channel() {
        let dir = std::env::temp_dir().join("opennote-corpus-channel");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("channel_fingering.txt");
        std::fs::write(
            &path,
            "//Version: ThumbSet_v1\n\
             0\t0.0\t0.5\tE4\t80\t80\t1\t0\n\
             1\t0.0\t0.5\tC3\t80\t80\t1\t0\n\
             2\t0.5\t1.0\tA4\t80\t80\t0\t0\n\
             3\t0.5\t1.0\tG4\t80\t80\t1\t-1\n",
        )
        .unwrap();
        let piece = read_pig(&path).unwrap().unwrap();
        let hands: Vec<(u8, HandLabel)> = piece.notes.iter().map(|n| (n.midi, n.hand)).collect();
        assert!(hands.contains(&(64, HandLabel::Left)), "E4 on channel 1 is the left hand: {hands:?}");
        assert!(hands.contains(&(48, HandLabel::Left)));
        assert!(hands.contains(&(69, HandLabel::Right)));
        assert!(hands.contains(&(67, HandLabel::Left)), "a fingered note still goes by its sign");
    }
}
