//! A correction to the fingering cost, learned from what pianists actually play.
//!
//! The rules and the hand model propose; this adjusts. Every chord shape and every move
//! between two chords is described by a handful of plain facts — which fingers, how
//! far apart in semitones, on black keys or white, how quickly — and each fact carries a
//! weight, learned by running the search over pieces whose fingering is known and
//! nudging the weights until the search's own answer agrees with the pianist's (a
//! structured perceptron; Collins, 2002). The weights are in the same units as the rest
//! of the cost, so what is learned is exactly how far the rules' preferences should
//! move, and no further.
//!
//! What it may never do is make an impossible fingering possible. However the weights
//! come out, the most they can add or take away at one chord or one move is
//! [`LIMIT`], well under what the hand model charges for a stretch the hand cannot
//! make — so the rules keep their veto.

use std::collections::HashMap;

use on_hand::keyboard::is_black;
use on_hand::{Finger, Hand};
use serde::{Deserialize, Serialize};

/// The most the learned correction may add or remove at one chord or one move.
///
/// A third of [`crate::biomech::UNREACHABLE_PENALTY`], so no learned preference,
/// however strong, can buy a shape the hand cannot reach.
pub const LIMIT: f32 = 20.0;

/// Learned weights, one per fact.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Learned {
    /// Keyed by the packed fact; see [`chord_features`] and [`step_features`].
    pub weights: HashMap<u64, f32>,
}

impl Learned {
    /// What the correction charges for holding a chord shape.
    pub fn chord(&self, hand: Hand, notes: &[u8], fingers: &[Finger]) -> f32 {
        let mut facts = Vec::with_capacity(8);
        chord_features(hand, notes, fingers, &mut facts);
        self.sum(&facts)
    }

    /// What the correction charges for moving from one chord shape to the next.
    pub fn step(
        &self,
        hand: Hand,
        from: (&[u8], &[Finger]),
        to: (&[u8], &[Finger]),
        seconds: f64,
    ) -> f32 {
        let mut facts = Vec::with_capacity(8);
        step_features(hand, from, to, seconds, &mut facts);
        self.sum(&facts)
    }

    fn sum(&self, facts: &[u64]) -> f32 {
        if self.weights.is_empty() {
            return 0.0;
        }
        facts
            .iter()
            .filter_map(|fact| self.weights.get(fact))
            .sum::<f32>()
            .clamp(-LIMIT, LIMIT)
    }
}

/// A fact, packed into one number: what kind it is, then its fields, a few bits each.
fn pack(kind: u64, fields: &[(u64, u32)]) -> u64 {
    let mut key = kind;
    for (value, bits) in fields {
        key = (key << bits) | (value & ((1 << bits) - 1));
    }
    key
}

fn colour(midi: u8) -> u64 {
    u64::from(is_black(midi))
}

fn finger(f: Finger) -> u64 {
    f.index() as u64
}

/// A distance in semitones, clipped to what a hand does in one reach and made
/// non-negative for packing.
fn interval(from: u8, to: u8, clip: i32) -> u64 {
    ((i32::from(to) - i32::from(from)).clamp(-clip, clip) + clip) as u64
}

/// How long the hand had, bucketed: a fast run, a moderate line, a slow one, a pause.
fn pace(seconds: f64) -> u64 {
    match seconds {
        s if s < 0.12 => 0,
        s if s < 0.25 => 1,
        s if s < 0.5 => 2,
        _ => 3,
    }
}

/// The facts about one chord shape: each neighbouring pair of its fingers with the
/// interval between them and the colours of their keys, and each finger on its key's
/// colour.
pub fn chord_features(hand: Hand, notes: &[u8], fingers: &[Finger], out: &mut Vec<u64>) {
    let h = hand as u64;
    for (n, f) in notes.iter().zip(fingers) {
        out.push(pack(1, &[(h, 1), (finger(*f), 3), (colour(*n), 1), (notes.len().min(5) as u64, 3)]));
    }
    for i in 1..notes.len().min(fingers.len()) {
        out.push(pack(
            2,
            &[
                (h, 1),
                (finger(fingers[i - 1]), 3),
                (finger(fingers[i]), 3),
                (interval(notes[i - 1], notes[i], 24), 6),
                (colour(notes[i - 1]), 1),
                (colour(notes[i]), 1),
            ],
        ));
    }
}

/// The facts about a move from one chord shape to the next, along the voices the rules
/// follow — the lowest note and the highest, which for single notes are one line: the
/// fingers at either end of the move, the interval and the colours, and more coarsely
/// the fingers, the direction and how quickly it had to happen.
pub fn step_features(
    hand: Hand,
    from: (&[u8], &[Finger]),
    to: (&[u8], &[Finger]),
    seconds: f64,
    out: &mut Vec<u64>,
) {
    if from.0.is_empty() || to.0.is_empty() {
        return;
    }
    let h = hand as u64;
    let line = from.0.len() == 1 && to.0.len() == 1;
    let sides: &[(u64, usize, usize)] = if line {
        &[(2, 0, 0)]
    } else {
        &[(0, 0, 0), (1, from.0.len() - 1, to.0.len() - 1)]
    };
    for &(side, a, b) in sides {
        let (Some(&p), Some(&q)) = (from.0.get(a), to.0.get(b)) else { continue };
        let (Some(&f), Some(&g)) = (from.1.get(a), to.1.get(b)) else { continue };
        out.push(pack(
            3,
            &[
                (h, 1),
                (side, 2),
                (finger(f), 3),
                (finger(g), 3),
                (interval(p, q, 16), 6),
                (colour(p), 1),
                (colour(q), 1),
            ],
        ));
        let direction = match q.cmp(&p) {
            std::cmp::Ordering::Less => 0,
            std::cmp::Ordering::Equal => 1,
            std::cmp::Ordering::Greater => 2,
        };
        out.push(pack(
            4,
            &[(h, 1), (side, 2), (finger(f), 3), (finger(g), 3), (direction, 2), (pace(seconds), 2)],
        ));
    }
}

/// A packed fact, said in words: the inverse of [`chord_features`] and
/// [`step_features`], for reading what a model learned.
pub fn describe(fact: u64) -> String {
    // Each kind of fact packs to its own range of numbers, since the kind sits above a
    // fixed number of bits of fields.
    let layouts: [(u64, &[u32]); 4] = [
        (1, &[1, 3, 1, 3]),
        (2, &[1, 3, 3, 6, 1, 1]),
        (3, &[1, 2, 3, 3, 6, 1, 1]),
        (4, &[1, 2, 3, 3, 2, 2]),
    ];
    for (kind, bits) in layouts {
        let width: u32 = bits.iter().sum();
        if fact >> width != kind {
            continue;
        }
        let mut fields = Vec::with_capacity(bits.len());
        let mut shift = width;
        for b in bits {
            shift -= b;
            fields.push((fact >> shift) & ((1 << b) - 1));
        }
        let hand = if fields[0] == 0 { "left" } else { "right" };
        let key = |c: u64| if c == 1 { "black" } else { "white" };
        let side = |s: u64| match s {
            0 => "lowest voice",
            1 => "highest voice",
            _ => "line",
        };
        return match kind {
            1 => format!("{hand} hand, finger {} on a {} key, in a {}-note chord", fields[1] + 1, key(fields[2]), fields[3]),
            2 => format!(
                "{hand} hand chord, fingers {}-{} a {} semitones apart, {} then {}",
                fields[1] + 1,
                fields[2] + 1,
                fields[3] as i64 - 24,
                key(fields[4]),
                key(fields[5])
            ),
            3 => format!(
                "{hand} hand {}, finger {} to {} moving {:+} semitones, {} to {}",
                side(fields[1]),
                fields[2] + 1,
                fields[3] + 1,
                fields[4] as i64 - 16,
                key(fields[5]),
                key(fields[6])
            ),
            _ => format!(
                "{hand} hand {}, finger {} to {} {}, {}",
                side(fields[1]),
                fields[2] + 1,
                fields[3] + 1,
                ["downwards", "on the same key", "upwards"][fields[4].min(2) as usize],
                ["fast", "moderately", "slowly", "after a pause"][fields[5] as usize]
            ),
        };
    }
    format!("unknown fact {fact}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_fact_can_be_read_back() {
        let mut facts = Vec::new();
        chord_features(Hand::Left, &[48, 52], &[Finger::Little, Finger::Middle], &mut facts);
        step_features(Hand::Right, (&[61], &[Finger::Middle]), (&[60], &[Finger::Thumb]), 0.1, &mut facts);
        let said: Vec<String> = facts.iter().map(|f| describe(*f)).collect();
        assert!(said.iter().all(|s| !s.starts_with("unknown")), "{said:?}");
        assert!(said.contains(&"left hand chord, fingers 5-3 a 4 semitones apart, white then white".to_string()), "{said:?}");
        assert!(said.contains(&"right hand line, finger 3 to 1 moving -1 semitones, black to white".to_string()), "{said:?}");
    }

    use super::*;

    #[test]
    fn an_empty_model_changes_nothing() {
        let model = Learned::default();
        assert_eq!(model.chord(Hand::Right, &[60, 64], &[Finger::Thumb, Finger::Middle]), 0.0);
        assert_eq!(
            model.step(Hand::Right, (&[60], &[Finger::Thumb]), (&[62], &[Finger::Index]), 0.2),
            0.0
        );
    }

    #[test]
    fn no_weight_can_move_a_chord_past_the_limit() {
        let mut facts = Vec::new();
        chord_features(Hand::Right, &[60, 64], &[Finger::Thumb, Finger::Middle], &mut facts);
        let model = Learned { weights: facts.iter().map(|f| (*f, -1000.0)).collect() };
        assert_eq!(model.chord(Hand::Right, &[60, 64], &[Finger::Thumb, Finger::Middle]), -LIMIT);
    }

    #[test]
    fn different_facts_are_different_keys() {
        let mut a = Vec::new();
        let mut b = Vec::new();
        step_features(Hand::Right, (&[60], &[Finger::Thumb]), (&[62], &[Finger::Index]), 0.2, &mut a);
        step_features(Hand::Right, (&[60], &[Finger::Thumb]), (&[62], &[Finger::Middle]), 0.2, &mut b);
        assert!(a.iter().all(|f| !b.contains(f)), "{a:?} {b:?}");
    }
}
