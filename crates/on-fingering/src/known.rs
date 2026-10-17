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

const CHORD_SECONDS: f64 = 0.03;

const PASSAGE: usize = 6;

const LEAST_NOTES: usize = 100;

const LEAST_VOTERS: u32 = 3;

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
        notes.sort_by_key(|a| order(a.onset, a.midi));
        let mut last = [f64::NEG_INFINITY; 2];
        let packed = notes
            .iter()
            .map(|n| {
                let together = n.onset - last[n.hand as usize] < CHORD_SECONDS;
                last[n.hand as usize] = n.onset;
                pack(n, together)
            })
            .collect();
        self.pieces.push(packed);
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
        if played.len() < LEAST_NOTES {
            return Vec::new();
        }
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
                let own: HashSet<u64> = grams(&pitches).map(|(_, g)| g).collect();
                let found = own.iter().filter(|g| wanted.contains_key(g)).count() as f32;
                ((found / wanted.len() as f32).min(found / own.len().max(1) as f32), k)
            })
            .filter(|(share, _)| *share >= LEAST_SHARE)
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        ranked.truncate(CANDIDATES);

        let mut ballots = vec![[0.0f32; 5]; played.len()];
        let mut elsewhere = vec![0.0f32; played.len()];
        for (_, k) in ranked {
            let piece = &self.pieces[k];
            let pitches: Vec<u8> = piece.iter().map(|v| unpack(*v).0).collect();
            for (i, j) in aligned(&wanted, &pitches) {
                let (_, hand, finger, confidence) = unpack(piece[j]);
                let Some(finger) = finger else { continue };
                if hand == played[i].3 {
                    ballots[i][finger.index()] += confidence.max(LEAST_CONFIDENCE);
                } else {
                    elsewhere[i] += confidence.max(LEAST_CONFIDENCE);
                }
            }
        }

        let mut pins: HashMap<NoteId, Finger> = HashMap::new();
        for (i, ballot) in ballots.iter().enumerate() {
            let total: f32 = ballot.iter().sum::<f32>() + elsewhere[i];
            let (best, weight) =
                ballot.iter().enumerate().fold((0, 0.0f32), |a, (f, w)| if *w > a.1 { (f, *w) } else { a });
            if weight >= LEAST_WEIGHT && weight >= LEAST_AGREEMENT * total {
                pins.insert(played[i].2, Finger::ALL[best]);
            }
        }
        for hand in Hand::ALL {
            for chord in score.chords(hand) {
                let mut struck: Vec<(u8, Option<Finger>)> = chord
                    .notes
                    .iter()
                    .filter(|id| !score.note(**id).tie.is_continuation())
                    .map(|id| (score.note(*id).midi, pins.get(id).copied()))
                    .collect();
                struck.sort_by_key(|(midi, _)| *midi);
                let room = |rank: usize, finger: Finger| {
                    let (below, above) = match hand {
                        Hand::Right => (finger.index(), 4 - finger.index()),
                        Hand::Left => (4 - finger.index(), finger.index()),
                    };
                    let after: Vec<usize> =
                        struck[rank + 1..].iter().filter_map(|(_, f)| f.map(|f| f.index())).collect();
                    let rises = after.first().is_none_or(|next| match hand {
                        Hand::Right => *next > finger.index(),
                        Hand::Left => *next < finger.index(),
                    });
                    rises && below >= rank && above >= struck.len() - 1 - rank
                };
                let fits = struck.iter().enumerate().all(|(rank, (_, f))| f.is_none_or(|f| room(rank, f)));
                if !fits {
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

    pub fn suggest(&self, score: &Score) -> HashMap<NoteId, [f32; 5]> {
        let mut out = HashMap::new();
        if self.pieces.is_empty() {
            return out;
        }
        let mut wanted: [HashMap<u64, Vec<usize>>; 2] = Default::default();
        let mut ids: [Vec<NoteId>; 2] = Default::default();
        for hand in Hand::ALL {
            let mut played: Vec<&on_score::Note> =
                score.notes.iter().filter(|n| n.hand == Some(hand) && !n.tie.is_continuation()).collect();
            played.sort_by_key(|a| order(a.onset_seconds, a.midi));
            let shapes: Vec<u64> = played
                .iter()
                .enumerate()
                .map(|(k, n)| {
                    let before = k.checked_sub(1).map(|k| played[k]);
                    let together = before.is_some_and(|b| n.onset_seconds - b.onset_seconds < CHORD_SECONDS);
                    shape(before.map(|b| b.midi), n.midi, together)
                })
                .collect();
            for (at, key) in passages(&shapes) {
                wanted[hand as usize].entry(key).or_default().push(at);
            }
            ids[hand as usize] = played.iter().map(|n| n.id).collect();
        }
        let mut votes: [Vec<[f32; 5]>; 2] = [vec![[0.0; 5]; ids[0].len()], vec![[0.0; 5]; ids[1].len()]];
        let mut voters: [Vec<(u32, usize)>; 2] =
            [vec![(0, usize::MAX); ids[0].len()], vec![(0, usize::MAX); ids[1].len()]];
        for (k, piece) in self.pieces.iter().enumerate() {
            for hand in Hand::ALL {
                let side = hand as usize;
                if wanted[side].is_empty() {
                    continue;
                }
                let mine: Vec<(Option<Finger>, f32, u64)> = {
                    let mut previous = None;
                    piece
                        .iter()
                        .map(|v| (unpack(*v), v >> 15 == 1))
                        .filter(|((_, h, _, _), _)| *h == hand)
                        .map(|((midi, _, finger, confidence), together)| {
                            let s = shape(previous, midi, together);
                            previous = Some(midi);
                            (finger, confidence.max(LEAST_CONFIDENCE), s)
                        })
                        .collect()
                };
                let shapes: Vec<u64> = mine.iter().map(|m| m.2).collect();
                for (j, key) in passages(&shapes) {
                    let Some(at) = wanted[side].get(&key) else { continue };
                    for i in at {
                        for d in 0..PASSAGE {
                            let Some(finger) = mine[j + d].0 else { continue };
                            let seen = &mut voters[side][i + d];
                            if seen.1 != k {
                                *seen = (seen.0 + 1, k);
                                votes[side][i + d][finger.index()] += mine[j + d].1;
                            }
                        }
                    }
                }
            }
        }
        for side in 0..2 {
            for (i, id) in ids[side].iter().enumerate() {
                let total: f32 = votes[side][i].iter().sum();
                if voters[side][i].0 >= LEAST_VOTERS && total > 0.0 {
                    out.insert(*id, votes[side][i].map(|w| w / total));
                }
            }
        }
        out
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

fn pack(note: &KnownNote, together: bool) -> u16 {
    let finger = note.finger.map_or(0, |f| f.index() as u16 + 1);
    let confidence = (note.confidence.clamp(0.0, 1.0) * 15.0).round() as u16;
    u16::from(note.midi & 0x7f)
        | (note.hand as u16) << 7
        | finger << 8
        | confidence << 11
        | u16::from(together) << 15
}

fn unpack(value: u16) -> (u8, Hand, Option<Finger>, f32) {
    let finger = (value >> 8) & 7;
    (
        (value & 0x7f) as u8,
        Hand::ALL[usize::from((value >> 7) & 1)],
        (finger > 0).then(|| Finger::ALL[usize::from(finger - 1)]),
        f32::from((value >> 11) & 15) / 15.0,
    )
}

fn shape(previous: Option<u8>, midi: u8, together: bool) -> u64 {
    let step = previous.map_or(0, |p| (i32::from(midi) - i32::from(p)).clamp(-24, 24) + 25) as u64;
    step << 2 | u64::from(on_hand::keyboard::is_black(midi)) << 1 | u64::from(together)
}

fn passages(shapes: &[u64]) -> impl Iterator<Item = (usize, u64)> + '_ {
    shapes
        .windows(PASSAGE)
        .enumerate()
        .skip(1)
        .map(|(at, w)| (at, w.iter().fold(0u64, |key, s| key << 8 | s)))
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
        [0u8, 2, 4, 5, 7, 9]
            .iter()
            .flat_map(|shift| {
                pitches.iter().zip(fingers).map(move |(p, f)| (p + shift, Finger::from_number(f).unwrap()))
            })
            .collect()
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
    fn a_pin_that_leaves_a_chord_note_no_finger_is_dropped() {
        let tune = melody();
        let under: Vec<usize> = (0..tune.len()).filter(|k| tune[*k].1 == Finger::Thumb).collect();
        let mut known = Known::default();
        for seconds in [0.3, 0.41] {
            let mut notes = played(&tune, seconds);
            for k in &under {
                notes.push(KnownNote {
                    onset: *k as f64 * seconds,
                    midi: tune[*k].0 - 3,
                    hand: Hand::Left,
                    finger: Some(Finger::Index),
                    confidence: 0.9,
                });
            }
            known.add(notes);
        }
        let single = score_of(&tune.iter().map(|(p, _)| *p).collect::<Vec<_>>());
        let mut score = Score::default();
        let mut above = Vec::new();
        for (k, note) in single.notes.iter().enumerate() {
            if under.contains(&k) {
                let mut below = note.clone();
                below.midi -= 3;
                below.id = NoteId(score.notes.len() as u32);
                score.notes.push(below);
                above.push(NoteId(score.notes.len() as u32));
            }
            let mut note = note.clone();
            note.id = NoteId(score.notes.len() as u32);
            score.notes.push(note);
        }
        let pins: HashMap<NoteId, Finger> = known.recall(&score).into_iter().collect();
        assert!(pins.len() > tune.len() / 2, "the rest of the piece is still recalled");
        for id in &above {
            assert!(!pins.contains_key(id), "note {}: a right thumb pinned above a right-hand note", id.0);
        }
    }

    #[test]
    fn a_note_the_pianists_took_with_the_other_hand_is_left_to_the_solver() {
        let tune = melody();
        let mut known = Known::default();
        for seconds in [0.3, 0.41] {
            let mut notes = played(&tune, seconds);
            for (k, note) in notes.iter_mut().enumerate() {
                if k % 4 == 0 {
                    note.hand = Hand::Left;
                }
            }
            known.add(notes);
        }
        let score = score_of(&tune.iter().map(|(p, _)| *p).collect::<Vec<_>>());
        let pins = known.recall(&score);
        assert!(!pins.is_empty());
        assert!(pins.iter().all(|(id, _)| id.index() % 4 != 0), "a note played by the other hand was pinned");
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
        let solution =
            crate::finger_score_with_prior(&score, &crate::FingeringOptions::default(), Some(&prior));
        for fingering in &solution.fingerings {
            assert_eq!(fingering.finger, tune[fingering.note.index()].1, "note {}", fingering.note.index());
        }
    }

    #[test]
    fn a_passage_many_pianists_finger_alike_is_suggested_in_an_unknown_piece() {
        let tune = melody();
        let mut known = Known::default();
        for lead in [40u8, 43, 47] {
            let mut piece: Vec<(u8, Finger)> =
                (0..6).map(|k| (lead + 2 * k, Finger::ALL[k as usize % 5])).collect();
            piece.extend(tune[..10].iter().copied());
            known.add(played(&piece, 0.3));
        }
        let mut pitches: Vec<u8> = vec![79, 77, 76, 74, 72, 71];
        pitches.extend(tune[..10].iter().map(|(p, _)| p + 12));
        let suggested = known.suggest(&score_of(&pitches));
        assert!(known.recall(&score_of(&pitches)).is_empty(), "the piece itself is not known");
        for k in 7..16 {
            let share =
                suggested.get(&NoteId(k as u32)).unwrap_or_else(|| panic!("note {k} had no suggestion"));
            let finger = tune[k - 6].1;
            assert!(share[finger.index()] > 0.99, "note {k}: {share:?}, documented {finger:?}");
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
    fn a_short_exercise_inside_a_long_piece_is_not_that_piece() {
        let tune = melody();
        let mut long: Vec<(u8, Finger)> = Vec::new();
        for shift in [0u8, 1, 3, 12, 14] {
            long.extend(tune.iter().map(|(p, f)| (p + shift, *f)));
        }
        let mut known = Known::default();
        known.add(played(&long, 0.3));
        known.add(played(&long, 0.35));
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
