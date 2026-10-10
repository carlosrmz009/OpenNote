use std::collections::{HashMap, HashSet};

use on_hand::{Finger, Hand};
use on_score::{NoteId, Score};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

const GRAM: usize = 6;

const LEAST_SHARE: f32 = 0.3;

const CANDIDATES: usize = 8;

const BUSIEST_GRAM: usize = 64;

const LEAST_AGREEMENT: f32 = 0.6;

const LEAST_WEIGHT: f32 = 0.5;

const LEAST_CONFIDENCE: f32 = 0.05;

const TOGETHER_SECONDS: f64 = 0.01;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Known {
    pieces: Vec<Vec<u16>>,
}

pub struct KnownNote {
    pub onset: f64,
    pub midi: u8,
    pub hand: Hand,
    pub finger: Option<Finger>,
    pub confidence: f32,
}

impl Known {
    pub fn add(&mut self, mut notes: Vec<KnownNote>) {
        notes.sort_by(|a, b| order(a.onset, a.midi).cmp(&order(b.onset, b.midi)));
        self.pieces.push(notes.iter().map(pack).collect());
    }

    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    pub fn recall(&self, score: &Score) -> Vec<(NoteId, Finger)> {
        if self.pieces.is_empty() {
            return Vec::new();
        }
        let mut played: Vec<(i64, u8, NoteId, Hand)> = score
            .notes
            .iter()
            .filter(|n| !n.tie.is_continuation())
            .filter_map(|n| n.hand.map(|hand| (order(n.onset_seconds, n.midi).0, n.midi, n.id, hand)))
            .collect();
        played.sort_by_key(|(at, midi, _, _)| (*at, *midi));
        let pitches: Vec<u8> = played.iter().map(|p| p.1).collect();
        let mut wanted: HashMap<u64, Vec<usize>> = HashMap::new();
        for (at, gram) in grams(&pitches) {
            wanted.entry(gram).or_default().push(at);
        }
        if wanted.is_empty() {
            return Vec::new();
        }

        let mut ranked: Vec<(f32, usize)> = self
            .pieces
            .iter()
            .enumerate()
            .map(|(k, piece)| {
                let pitches: Vec<u8> = piece.iter().map(|v| unpack(*v).0).collect();
                let found: HashSet<u64> = grams(&pitches).map(|(_, g)| g).filter(|g| wanted.contains_key(g)).collect();
                (found.len() as f32 / wanted.len() as f32, k)
            })
            .filter(|(share, _)| *share >= LEAST_SHARE)
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        ranked.truncate(CANDIDATES);

        let mut ballots = vec![[0.0f32; 5]; played.len()];
        for (_, k) in ranked {
            let piece = &self.pieces[k];
            let pitches: Vec<u8> = piece.iter().map(|v| unpack(*v).0).collect();
            for (i, j) in aligned(&wanted, &pitches) {
                let (_, hand, finger, confidence) = unpack(piece[j]);
                if let Some(finger) = finger.filter(|_| hand == played[i].3) {
                    ballots[i][finger.index()] += confidence.max(LEAST_CONFIDENCE);
                }
            }
        }

        let mut pins: HashMap<NoteId, Finger> = HashMap::new();
        for (i, ballot) in ballots.iter().enumerate() {
            let total: f32 = ballot.iter().sum();
            let (best, weight) = ballot.iter().enumerate().fold((0, 0.0f32), |a, (f, w)| if *w > a.1 { (f, *w) } else { a });
            if weight >= LEAST_WEIGHT && weight >= LEAST_AGREEMENT * total {
                pins.insert(played[i].2, Finger::ALL[best]);
            }
        }
        for hand in Hand::ALL {
            for chord in score.chords(hand) {
                let mut held: Vec<(u8, Finger)> =
                    chord.notes.iter().filter_map(|id| pins.get(id).map(|f| (score.note(*id).midi, *f))).collect();
                held.sort_by_key(|(midi, _)| *midi);
                let rising = held.windows(2).all(|w| match hand {
                    Hand::Right => w[0].1.index() < w[1].1.index(),
                    Hand::Left => w[0].1.index() > w[1].1.index(),
                });
                if !rising {
                    for id in &chord.notes {
                        pins.remove(id);
                    }
                }
            }
        }
        let mut pins: Vec<(NoteId, Finger)> = pins.into_iter().collect();
        pins.sort_by_key(|(id, _)| id.0);
        pins
    }
}

fn order(onset: f64, midi: u8) -> (i64, u8) {
    ((onset / TOGETHER_SECONDS).round() as i64, midi)
}

fn grams(pitches: &[u8]) -> impl Iterator<Item = (usize, u64)> + '_ {
    pitches
        .windows(GRAM)
        .enumerate()
        .map(|(at, w)| (at, w.iter().fold(0u64, |key, p| (key << 7) | u64::from(p & 0x7f))))
}

fn aligned(wanted: &HashMap<u64, Vec<usize>>, pitches: &[u8]) -> Vec<(usize, usize)> {
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (j, gram) in grams(pitches) {
        if let Some(at) = wanted.get(&gram).filter(|at| at.len() <= BUSIEST_GRAM) {
            pairs.extend(at.iter().map(|i| (*i, j)));
        }
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    let mut tails: Vec<usize> = Vec::new();
    let mut before = vec![usize::MAX; pairs.len()];
    for (k, pair) in pairs.iter().enumerate() {
        let slot = tails.partition_point(|t| pairs[*t].1 < pair.1);
        if slot > 0 {
            before[k] = tails[slot - 1];
        }
        if slot == tails.len() {
            tails.push(k);
        } else {
            tails[slot] = k;
        }
    }
    let mut chain = Vec::new();
    let mut at = tails.last().copied().unwrap_or(usize::MAX);
    while at != usize::MAX {
        chain.push(pairs[at]);
        at = before[at];
    }
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (i, j) in chain.into_iter().rev() {
        for k in 0..GRAM {
            if out.last().is_none_or(|last| last.0 < i + k && last.1 < j + k) {
                out.push((i + k, j + k));
            }
        }
    }
    out
}

fn pack(note: &KnownNote) -> u16 {
    let finger = note.finger.map_or(0, |f| f.index() as u16 + 1);
    let confidence = (note.confidence.clamp(0.0, 1.0) * 15.0).round() as u16;
    u16::from(note.midi & 0x7f) | (note.hand as u16) << 7 | finger << 8 | confidence << 11
}

fn unpack(value: u16) -> (u8, Hand, Option<Finger>, f32) {
    let finger = (value >> 8) & 7;
    (
        (value & 0x7f) as u8,
        Hand::ALL[usize::from((value >> 7) & 1)],
        (finger > 0).then(|| Finger::ALL[usize::from(finger - 1)]),
        f32::from(value >> 11) / 15.0,
    )
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

impl Serialize for Known {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut bytes = Vec::new();
        for piece in &self.pieces {
            bytes.extend_from_slice(&(piece.len() as u32).to_le_bytes());
            for value in piece {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        let mut text = String::with_capacity(bytes.len() / 3 * 4 + 4);
        for chunk in bytes.chunks(3) {
            let word = chunk.iter().enumerate().fold(0u32, |w, (k, b)| w | u32::from(*b) << (16 - 8 * k));
            for k in 0..=chunk.len() {
                text.push(ALPHABET[(word >> (18 - 6 * k) & 63) as usize] as char);
            }
        }
        serializer.serialize_str(&text)
    }
}

impl<'de> Deserialize<'de> for Known {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let mut digit = [u8::MAX; 256];
        for (k, c) in ALPHABET.iter().enumerate() {
            digit[usize::from(*c)] = k as u8;
        }
        let mut bytes = Vec::with_capacity(text.len() / 4 * 3 + 3);
        for chunk in text.as_bytes().chunks(4) {
            let mut word = 0u32;
            for (k, c) in chunk.iter().enumerate() {
                let d = digit[usize::from(*c)];
                if d == u8::MAX {
                    return Err(serde::de::Error::custom("the known pieces are damaged"));
                }
                word |= u32::from(d) << (18 - 6 * k);
            }
            bytes.extend((0..chunk.len() - 1).map(|k| (word >> (16 - 8 * k)) as u8));
        }
        let mut pieces = Vec::new();
        let mut at = 0;
        while at + 4 <= bytes.len() {
            let len = u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes")) as usize;
            at += 4;
            let end = at + 2 * len;
            if end > bytes.len() {
                return Err(serde::de::Error::custom("the known pieces are cut short"));
            }
            pieces.push(bytes[at..end].chunks(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect());
            at = end;
        }
        Ok(Known { pieces })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use on_score::{Note, SourceRef, TieState};

    fn melody() -> Vec<(u8, Finger)> {
        let fingers = [1, 2, 3, 1, 2, 3, 4, 5, 3, 2, 1, 2, 4, 3, 2, 1, 3, 5, 4, 2];
        let pitches = [60, 62, 64, 65, 67, 69, 71, 72, 69, 67, 65, 67, 71, 69, 67, 65, 69, 74, 72, 67];
        pitches.iter().zip(fingers).map(|(p, f)| (*p, Finger::from_number(f).unwrap())).collect()
    }

    fn played(tune: &[(u8, Finger)], seconds: f64) -> Vec<KnownNote> {
        tune.iter()
            .enumerate()
            .map(|(k, (midi, finger))| KnownNote {
                onset: k as f64 * seconds,
                midi: *midi,
                hand: Hand::Right,
                finger: Some(*finger),
                confidence: 0.9,
            })
            .collect()
    }

    fn score_of(pitches: &[u8]) -> Score {
        let mut score = Score::default();
        for (k, midi) in pitches.iter().enumerate() {
            score.notes.push(Note {
                id: NoteId(k as u32),
                midi: *midi,
                onset: k as i64 * 480,
                duration: 480,
                onset_seconds: k as f64 * 0.5,
                duration_seconds: 0.5,
                staff: None,
                voice: None,
                hand: Some(Hand::Right),
                tie: TieState::default(),
                grace: false,
                chord: false,
                velocity: 80,
                given_finger: None,
                source: SourceRef::Midi { track: 0, event: k },
            });
        }
        score
    }

    #[test]
    fn a_documented_piece_is_played_as_documented() {
        let tune = melody();
        let mut known = Known::default();
        known.add(played(&tune, 0.3));
        known.add(played(&tune, 0.41));
        let score = score_of(&tune.iter().map(|(p, _)| *p).collect::<Vec<_>>());
        let pins = known.recall(&score);
        assert_eq!(pins.len(), tune.len(), "every note was documented");
        for (id, finger) in pins {
            assert_eq!(finger, tune[id.index()].1, "note {}", id.index());
        }
    }

    #[test]
    fn a_model_that_knows_the_piece_fingers_it_as_documented() {
        let tune = melody();
        let mut prior = crate::NgramPrior::new();
        prior.known.add(played(&tune, 0.3));
        let score = score_of(&tune.iter().map(|(p, _)| *p).collect::<Vec<_>>());
        let solution = crate::finger_score_with_prior(&score, &crate::FingeringOptions::default(), Some(&prior));
        for fingering in &solution.fingerings {
            assert_eq!(fingering.finger, tune[fingering.note.index()].1, "note {}", fingering.note.index());
        }
    }

    #[test]
    fn versions_that_disagree_pin_nothing() {
        let tune = melody();
        let mut other: Vec<(u8, Finger)> = tune.clone();
        for (_, f) in &mut other {
            *f = Finger::ALL[(f.index() + 1) % 5];
        }
        let mut known = Known::default();
        known.add(played(&tune, 0.3));
        known.add(played(&other, 0.3));
        let score = score_of(&tune.iter().map(|(p, _)| *p).collect::<Vec<_>>());
        assert!(known.recall(&score).is_empty());
    }

    #[test]
    fn an_unknown_piece_is_left_to_the_solver() {
        let mut known = Known::default();
        known.add(played(&melody(), 0.3));
        let score = score_of(&[48, 50, 52, 53, 55, 57, 59, 60, 59, 57, 55, 53, 52, 50, 48]);
        assert!(known.recall(&score).is_empty());
    }

    #[test]
    fn the_library_survives_being_written_and_read() {
        let mut known = Known::default();
        known.add(played(&melody(), 0.3));
        known.add(played(&melody()[..7], 0.2));
        let text = serde_json::to_string(&known).unwrap();
        let read: Known = serde_json::from_str(&text).unwrap();
        assert_eq!(read, known);
    }
}
