//! Reading expert fingerings in, and keeping them in one shape.
//!
//! Fingering data arrives in whatever form whoever collected it chose. The PIG
//! dataset is tab-separated text with the hand encoded in the sign of the finger
//! number; an edition marked up in a notation program is MusicXML with
//! `<technical><fingering>` on some of its notes. Both say the same thing, so both
//! are read into [`Piece`] and written out as one line of JSON each.
//!
//! Normalising on the way in rather than at training time means the corpus directory
//! is readable, diffable, and cheap to load, and that adding support for a third
//! source later changes only this file.
//!
//! # A note on licences
//!
//! The obvious corpus to train on is PIG (Nakamura, Saito & Yoshii), and it is free
//! but registration-gated and licensed for nonprofit academic use. Nothing here
//! downloads it and nothing here redistributes it: `opennote corpus add` reads a
//! copy the user already has, and `corpus/` is not committed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use on_fingering::Placement;
use on_hand::{Finger, Hand};
use serde::{Deserialize, Serialize};

/// One note somebody fingered.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct FingeredNote {
    /// MIDI pitch.
    pub midi: u8,
    /// When it sounds, in seconds.
    pub onset: f64,
    /// How long it sounds for, in seconds.
    ///
    /// What the hand is still holding is a real constraint on what it can reach for
    /// next, so a piece cannot be fingered the way it was written without this. Corpus
    /// files written before it was recorded have no such field, and they fall back to
    /// [`ASSUMED_DURATION`] — which is what the evaluation used to assume for every
    /// note in every piece.
    #[serde(default = "assumed_duration")]
    pub duration: f64,
    /// Which hand played it.
    pub hand: HandLabel,
    /// Which finger, 1..=5.
    pub finger: u8,
}

/// What to assume a note's length is when the corpus file does not say.
///
/// An eighth note at a moderate tempo. It is a poor guess for any particular note and
/// the only one available for a file written before lengths were recorded.
pub const ASSUMED_DURATION: f64 = 0.25;

fn assumed_duration() -> f64 {
    ASSUMED_DURATION
}

/// Which hand, in a form that survives a round trip through JSON.
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

/// One fingering of one piece, by one person.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Piece {
    /// What the piece is. Two annotations of the same music share this.
    pub piece: String,
    /// Who fingered it. Distinguishes one annotator's reading from another's.
    pub annotator: String,
    /// Where it came from, for tracing a problem back.
    pub source: String,
    /// Every fingered note, in time order.
    pub notes: Vec<FingeredNote>,
}

impl Piece {
    /// One hand's notes, in time order, as the prior wants them.
    pub fn placements(&self, hand: Hand) -> Vec<Placement> {
        let label = HandLabel::from(hand);
        self.notes
            .iter()
            .filter(|note| note.hand == label)
            .filter_map(|note| Some(Placement::new(note.midi, Finger::from_number(note.finger)?)))
            .collect()
    }
}

/// A directory of normalised fingering data.
pub struct Corpus {
    root: PathBuf,
}

impl Corpus {
    /// Point at a corpus directory, creating it if it is not there.
    pub fn at(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)
            .with_context(|| format!("making the corpus directory {}", root.display()))?;
        Ok(Self { root })
    }

    /// Where the normalised data lives.
    fn data_dir(&self) -> PathBuf {
        self.root.join("normalised")
    }

    /// Read everything in the corpus.
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
        // Sorted, so training is reproducible: the same corpus gives the same model
        // and the same held-out split whatever order the filesystem hands them over.
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

    /// Add one already-built fingering to the corpus, and say where it went.
    ///
    /// The route in for anything that did not come from a file this module can read —
    /// a fingering recovered from video, most obviously.
    pub fn add_piece(&self, piece: &Piece, label: Option<&str>) -> Result<PathBuf> {
        let name = label.map(sanitise).unwrap_or_else(|| sanitise(&piece.piece));
        let directory = self.data_dir();
        std::fs::create_dir_all(&directory)?;
        let file = directory.join(format!("{name}.jsonl"));
        let mut text = serde_json::to_string(piece)?;
        text.push('\n');
        // Appended, so a second video of the same piece is a second fingering of it
        // rather than a replacement for the first.
        use std::io::Write as _;
        let mut out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)
            .with_context(|| format!("writing {}", file.display()))?;
        out.write_all(text.as_bytes())?;
        Ok(file)
    }

    /// Add everything under a path, and say what was taken in.
    ///
    /// Directories are walked; a single file is read on its own. Files that are not
    /// fingering data are skipped rather than treated as errors, because pointing
    /// this at a downloaded dataset directory should just work.
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

/// What one `corpus add` took in.
pub struct Ingested {
    /// Where it was written.
    pub file: PathBuf,
    /// How many fingerings — one piece fingered by two people counts twice.
    pub annotations: usize,
    /// How many distinct pieces those cover.
    pub pieces: usize,
    /// How many fingered notes altogether.
    pub notes: usize,
    /// Files that looked like they might be data but could not be read.
    pub skipped: Vec<String>,
}

/// Turn a name into something safe to use as a filename.
fn sanitise(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect()
}

/// Walk a path, reading whatever fingering data is under it.
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
        "musicxml" | "mxl" | "xml" => read_musicxml(path),
        // Not a mistake, just not data: a README, a licence, a MIDI file with no
        // fingering alongside it.
        _ => return Ok(()),
    };
    match read {
        Ok(Some(piece)) => pieces.push(piece),
        Ok(None) => {}
        Err(error) => skipped.push(format!("{}: {error:#}", path.display())),
    }
    Ok(())
}

/// Read one PIG fingering file.
///
/// The format is one note per line, tab-separated:
/// `id  onset  offset  spelled-pitch  onset-velocity  offset-velocity  channel  finger`.
/// The finger number carries the hand in its sign — positive for the right hand,
/// negative for the left — and a substitution is written as two numbers joined by an
/// underscore, of which the first is the finger that strikes the note.
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
        // The struck finger, before any substitution.
        let raw = fields[7].split('_').next().unwrap_or_default();
        let Ok(signed) = raw.parse::<i32>() else {
            continue;
        };
        if signed == 0 {
            continue;
        }
        let hand = if signed > 0 {
            HandLabel::Right
        } else {
            HandLabel::Left
        };
        let finger = signed.unsigned_abs() as u8;
        if !(1..=5).contains(&finger) {
            continue;
        }
        notes.push(FingeredNote {
            midi,
            onset,
            duration,
            hand,
            finger,
        });
    }
    if notes.is_empty() {
        return Ok(None);
    }
    notes.sort_by(|a, b| a.onset.total_cmp(&b.onset).then(a.midi.cmp(&b.midi)));

    // PIG names its files `<piece>-<annotator>_fingering.txt`, so two annotations of
    // the same music can be told apart — which is what the soft and highest match
    // rates need.
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

/// Turn PIG's spelled pitch — `C4`, `Bb3`, `F##2` — into a MIDI number.
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
    // MIDI 60 is middle C, which this notation calls C4.
    let value = (octave + 1) * 12 + step + alteration;
    u8::try_from(value).ok().filter(|v| (21..=108).contains(v))
}

/// Read fingerings out of a MusicXML file that already carries them.
fn read_musicxml(path: &Path) -> Result<Option<Piece>> {
    let document = on_score::MusicXmlDocument::read(path)?;
    let mut score = document.score().clone();
    // The staff a note is on is what says which hand played it, and the reader
    // already works that out; this fills in anything cross-staff or ambiguous.
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
                finger: note.given_finger?.number(),
            })
        })
        .collect();
    if notes.is_empty() {
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

/// Group annotations by the piece they are of.
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
        // Off the end of a piano, or not a pitch at all.
        assert_eq!(spelled_pitch_to_midi("C-1"), None);
        assert_eq!(spelled_pitch_to_midi("H4"), None);
        assert_eq!(spelled_pitch_to_midi(""), None);
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
                // PIG records when a note stops as well as when it starts, and the
                // difference is what the hand is actually holding.
                duration: 0.5,
                hand: HandLabel::Right,
                finger: 1
            }
        );
        // A substitution keeps the finger that strikes the note.
        assert_eq!(piece.notes[3].hand, HandLabel::Left);
        assert_eq!(piece.notes[3].finger, 2);

        let right = piece.placements(Hand::Right);
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
        // How long a note sounds was added to the format after the fact, because what a
        // hand is still holding turned out to constrain what it can reach for next. A
        // file written without it has to keep loading, with the length the evaluation
        // used to assume for everything.
        let old = r#"{"midi":60,"onset":0.5,"hand":"right","finger":1}"#;
        let note: FingeredNote = serde_json::from_str(old).unwrap();
        assert_eq!(note.duration, ASSUMED_DURATION);

        // And one written with it keeps what it says.
        let new = r#"{"midi":60,"onset":0.5,"duration":2.0,"hand":"right","finger":1}"#;
        let note: FingeredNote = serde_json::from_str(new).unwrap();
        assert_eq!(note.duration, 2.0);
    }
}
