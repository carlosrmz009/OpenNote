//! A statistical model of how pianists actually finger, learned from examples.
//!
//! The rules in [`crate::rules`] say what is comfortable. They do not say what is
//! *usual*, and those are not the same thing: an edition's fingering carries habits,
//! teaching traditions and the shape of the passage, none of which a comfort model
//! knows about. Nakamura, Saito & Yoshii (2020) measured the gap — a second-order
//! statistical model matched expert fingerings 64.3% of the time where the
//! constraint-based search managed 56.7%, with two humans agreeing 71.4% — which is
//! why this sits *alongside* the rules rather than replacing them.
//!
//! # Why counts rather than a network
//!
//! This is a table of how often each finger followed each other finger, over each
//! interval, between each pair of key colours. Nothing more. That choice is
//! deliberate:
//!
//! * it trains in a second on a laptop, from a handful of pieces;
//! * the file it produces can be read, and `opennote train --explain` prints what
//!   it learned in plain language;
//! * adding ten more pieces changes it by about what ten more pieces should, with no
//!   risk of it quietly memorising them;
//! * there is nothing to tune and no way for training to silently diverge.
//!
//! The whole model is a few thousand counts. It is not trying to be clever; it is
//! trying to notice that pianists put the thumb on the note after a black key far
//! more often than comfort alone would predict.
//!
//! # Smoothing
//!
//! Most contexts are never seen. A model that assigned them zero probability would
//! forbid perfectly good fingerings on the strength of a small corpus, so estimates
//! back off from specific contexts to general ones by Witten-Bell smoothing: the
//! weight given to the more general estimate is set by how many *different* fingers
//! have been seen in the specific one. A context seen once, with one finger, is
//! mostly ignored; a context seen a hundred times, with four different fingers, is
//! trusted and its diversity is taken as real.

use std::collections::HashMap;

use on_hand::{Finger, Hand};
use serde::{Deserialize, Serialize};

use crate::rules::Placement;
use crate::solver::FingeringPrior;

/// The widest interval the model distinguishes, in semitones.
///
/// Anything larger is a leap, and by then the previous finger says very little about
/// the next one; lumping them together keeps the table from filling up with contexts
/// seen once each.
const MAX_INTERVAL: i32 = 12;

/// How many interval buckets that comes to: −12..=12, plus one either side for
/// everything beyond.
#[cfg(test)]
const INTERVALS: usize = (2 * MAX_INTERVAL + 1) as usize + 2;

/// Counts of which finger came next, in one context.
type Tally = [u32; 5];

/// What a context is conditioned on, in order of how specific it is.
///
/// Serialized as a string so the model file can be read by eye.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Context {
    /// Nothing at all: how often each finger is used, over everything.
    Empty,
    /// The finger before.
    Previous(Finger),
    /// The finger before, and the interval to this note.
    Interval(Finger, usize),
    /// Also the colours of the two keys.
    Coloured(Finger, usize, u8),
    /// Also the finger before that.
    Trigram(Finger, usize, u8, Finger),
}

impl Context {
    /// The next most general context, or `None` at the bottom.
    fn backoff(&self) -> Option<Context> {
        match *self {
            Context::Empty => None,
            Context::Previous(_) => Some(Context::Empty),
            Context::Interval(f, _) => Some(Context::Previous(f)),
            Context::Coloured(f, d, _) => Some(Context::Interval(f, d)),
            Context::Trigram(f, d, c, _) => Some(Context::Coloured(f, d, c)),
        }
    }

    /// A short key for the serialized form.
    fn key(&self) -> String {
        match *self {
            Context::Empty => "-".into(),
            Context::Previous(f) => format!("p{}", f.number()),
            Context::Interval(f, d) => format!("i{}.{d}", f.number()),
            Context::Coloured(f, d, c) => format!("c{}.{d}.{c}", f.number()),
            Context::Trigram(f, d, c, g) => format!("t{}.{d}.{c}.{}", f.number(), g.number()),
        }
    }

    /// Read back a key written by [`Context::key`].
    fn from_key(key: &str) -> Option<Context> {
        let finger = |n: &str| Finger::from_number(n.parse::<u8>().ok()?);
        let (tag, rest) = key.split_at(1);
        let parts: Vec<&str> = rest.split('.').collect();
        match (tag, parts.as_slice()) {
            ("-", _) => Some(Context::Empty),
            ("p", [f]) => Some(Context::Previous(finger(f)?)),
            ("i", [f, d]) => Some(Context::Interval(finger(f)?, d.parse().ok()?)),
            ("c", [f, d, c]) => Some(Context::Coloured(
                finger(f)?,
                d.parse().ok()?,
                c.parse().ok()?,
            )),
            ("t", [f, d, c, g]) => Some(Context::Trigram(
                finger(f)?,
                d.parse().ok()?,
                c.parse().ok()?,
                finger(g)?,
            )),
            _ => None,
        }
    }
}

/// Which interval bucket a semitone distance falls in.
fn bucket(semitones: i32) -> usize {
    (semitones.clamp(-MAX_INTERVAL - 1, MAX_INTERVAL + 1) + MAX_INTERVAL + 1) as usize
}

/// Two key colours, as a number 0..=3.
fn colours(from: u8, to: u8) -> u8 {
    u8::from(on_hand::keyboard::is_black(from)) * 2 + u8::from(on_hand::keyboard::is_black(to))
}

/// The counts for one hand.
#[derive(Debug, Default, Clone)]
struct HandCounts {
    tallies: HashMap<Context, Tally>,
}

impl HandCounts {
    /// Record that `finger` was used in this context.
    fn observe(&mut self, context: Context, finger: Finger) {
        self.tallies.entry(context).or_insert([0; 5])[finger.index()] += 1;
    }

    /// Probability of `finger` in this context, smoothed by backing off.
    ///
    /// Witten-Bell: the specific estimate is trusted in proportion to how much was
    /// seen there, and the number of *distinct* fingers seen sets how much weight the
    /// more general estimate keeps. A context with one observation of one finger is
    /// nearly ignored; a well-populated one is nearly taken at face value.
    fn probability(&self, context: &Context, finger: Finger) -> f32 {
        let backoff = match context.backoff() {
            Some(parent) => self.probability(&parent, finger),
            // At the very bottom, before anything has been seen, every finger is
            // equally likely. This is the only place a number is invented.
            None => 0.2,
        };
        let Some(tally) = self.tallies.get(context) else {
            return backoff;
        };
        let total: u32 = tally.iter().sum();
        if total == 0 {
            return backoff;
        }
        let distinct = tally.iter().filter(|c| **c > 0).count() as f32;
        let count = tally[finger.index()] as f32;
        (count + distinct * backoff) / (total as f32 + distinct)
    }

    /// How many observations went into this model.
    fn observations(&self) -> u32 {
        self.tallies
            .get(&Context::Empty)
            .map(|t| t.iter().sum())
            .unwrap_or(0)
    }
}

/// A trained model of how a pianist fingers.
#[derive(Debug, Default, Clone)]
pub struct NgramPrior {
    hands: [HandCounts; 2],
}

impl NgramPrior {
    /// An empty model, which says every finger is equally likely.
    pub fn new() -> Self {
        Self::default()
    }

    /// Learn from one hand's part of one piece.
    ///
    /// `placements` are the notes of that hand in time order, each with the finger a
    /// pianist actually used. Every context this note falls into is counted, from the
    /// most specific down, which is what lets the backoff work later.
    pub fn observe(&mut self, hand: Hand, placements: &[Placement]) {
        let counts = &mut self.hands[hand as usize];
        for (index, current) in placements.iter().enumerate() {
            counts.observe(Context::Empty, current.finger);
            let Some(previous) = index.checked_sub(1).map(|i| placements[i]) else {
                continue;
            };
            let step = bucket(i32::from(current.midi) - i32::from(previous.midi));
            let colour = colours(previous.midi, current.midi);
            counts.observe(Context::Previous(previous.finger), current.finger);
            counts.observe(Context::Interval(previous.finger, step), current.finger);
            counts.observe(
                Context::Coloured(previous.finger, step, colour),
                current.finger,
            );
            if let Some(before) = index.checked_sub(2).map(|i| placements[i]) {
                counts.observe(
                    Context::Trigram(previous.finger, step, colour, before.finger),
                    current.finger,
                );
            }
        }
    }

    /// How many notes this model was trained on, both hands together.
    pub fn observations(&self) -> u32 {
        self.hands.iter().map(HandCounts::observations).sum()
    }

    /// Whether anything has been learned at all.
    pub fn is_empty(&self) -> bool {
        self.observations() == 0
    }

    /// Write the model out.
    ///
    /// JSON rather than a packed binary. It is a few hundred kilobytes either way,
    /// and being able to open the file and see what the model believes is worth more
    /// than the space — particularly to somebody training a model for the first time.
    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let stored = StoredModel {
            version: 1,
            left: serialize_hand(&self.hands[Hand::Left as usize]),
            right: serialize_hand(&self.hands[Hand::Right as usize]),
        };
        let text = serde_json::to_string_pretty(&stored)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Read a model back.
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let stored: StoredModel = serde_json::from_str(&text)?;
        anyhow::ensure!(
            stored.version == 1,
            "this model was written by a different version of opennote"
        );
        let mut prior = Self::new();
        prior.hands[Hand::Left as usize] = deserialize_hand(&stored.left);
        prior.hands[Hand::Right as usize] = deserialize_hand(&stored.right);
        Ok(prior)
    }

    /// What the model learned, in sentences.
    ///
    /// Meant to be read by somebody who wants to know whether training worked, not by
    /// somebody who wants to debug the model. It reports the strongest habits it
    /// picked up — the contexts where one finger dominates — because those are the
    /// ones that will actually change a fingering.
    pub fn explain(&self, per_hand: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for hand in Hand::ALL {
            let counts = &self.hands[hand as usize];
            lines.push(format!(
                "{hand:?} hand: {} notes seen, {} contexts",
                counts.observations(),
                counts.tallies.len()
            ));

            let mut habits: Vec<(f32, u32, String)> = counts
                .tallies
                .iter()
                .filter_map(|(context, tally)| {
                    let total: u32 = tally.iter().sum();
                    // A habit needs to have been seen often enough to be a habit.
                    if total < 12 {
                        return None;
                    }
                    let (best, count) = tally
                        .iter()
                        .enumerate()
                        .max_by_key(|(_, c)| **c)
                        .map(|(i, c)| (Finger::from_index(i), *c))?;
                    let share = count as f32 / total as f32;
                    if share < 0.6 {
                        return None;
                    }
                    Some((share, total, describe(context, best, share, total)?))
                })
                .collect();
            // Strongest first, and among equally strong ones the best attested.
            habits.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
            for (_, _, line) in habits.into_iter().take(per_hand) {
                lines.push(format!("  {line}"));
            }
        }
        lines
    }
}

/// One habit, in words.
fn describe(context: &Context, finger: Finger, share: f32, total: u32) -> Option<String> {
    let percent = (share * 100.0).round() as u32;
    let step = |d: usize| {
        let semitones = d as i32 - MAX_INTERVAL - 1;
        match semitones {
            0 => "repeating the note".to_string(),
            s if s.abs() > MAX_INTERVAL => {
                format!("leaping {}", if s > 0 { "up" } else { "down" })
            }
            s if s > 0 => format!("going up {s} semitone{}", plural(s)),
            s => format!("going down {} semitone{}", -s, plural(-s)),
        }
    };
    let colour = |c: u8| match c {
        0 => "white to white",
        1 => "white to black",
        2 => "black to white",
        _ => "black to black",
    };
    let line = match *context {
        Context::Empty => return None,
        Context::Previous(prev) => format!(
            "after finger {}, uses {} {percent}% of the time ({total} notes)",
            prev.number(),
            finger.number()
        ),
        Context::Interval(prev, d) => format!(
            "after finger {} and {}, uses {} {percent}% ({total} notes)",
            prev.number(),
            step(d),
            finger.number()
        ),
        Context::Coloured(prev, d, c) => format!(
            "after finger {}, {}, {}, uses {} {percent}% ({total} notes)",
            prev.number(),
            step(d),
            colour(c),
            finger.number()
        ),
        Context::Trigram(prev, d, c, before) => format!(
            "after fingers {}-{}, {}, {}, uses {} {percent}% ({total} notes)",
            before.number(),
            prev.number(),
            step(d),
            colour(c),
            finger.number()
        ),
    };
    Some(line)
}

/// "s", unless there is only one.
fn plural(n: i32) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

impl FingeringPrior for NgramPrior {
    fn log_probability(
        &self,
        hand: Hand,
        prev2: Option<Placement>,
        prev: Option<Placement>,
        current: Placement,
    ) -> f32 {
        let counts = &self.hands[hand as usize];
        let context = match (prev2, prev) {
            (_, None) => Context::Empty,
            (before, Some(previous)) => {
                let step = bucket(i32::from(current.midi) - i32::from(previous.midi));
                let colour = colours(previous.midi, current.midi);
                match before {
                    Some(before) => {
                        Context::Trigram(previous.finger, step, colour, before.finger)
                    }
                    None => Context::Coloured(previous.finger, step, colour),
                }
            }
        };
        // Floored, so an unseen fingering is merely unlikely rather than forbidden.
        // The rules, not the corpus, are what rule a fingering out.
        counts.probability(&context, current.finger).max(1e-4).ln()
    }
}

/// The model as it is stored on disk.
#[derive(Serialize, Deserialize)]
struct StoredModel {
    version: u32,
    left: HashMap<String, Tally>,
    right: HashMap<String, Tally>,
}

fn serialize_hand(counts: &HandCounts) -> HashMap<String, Tally> {
    counts
        .tallies
        .iter()
        .map(|(context, tally)| (context.key(), *tally))
        .collect()
}

fn deserialize_hand(stored: &HashMap<String, Tally>) -> HandCounts {
    let mut counts = HandCounts::default();
    for (key, tally) in stored {
        if let Some(context) = Context::from_key(key) {
            counts.tallies.insert(context, *tally);
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(pattern: &[(u8, u8)]) -> Vec<Placement> {
        pattern
            .iter()
            .map(|(midi, finger)| Placement::new(*midi, Finger::from_number(*finger).unwrap()))
            .collect()
    }

    /// An untrained model has no opinion, so every finger comes out equally likely.
    #[test]
    fn an_empty_model_has_no_opinion() {
        let prior = NgramPrior::new();
        let here = Placement::new(62, Finger::Index);
        let there = Placement::new(62, Finger::Little);
        let previous = Some(Placement::new(60, Finger::Thumb));
        assert!(
            (prior.log_probability(Hand::Right, None, previous, here)
                - prior.log_probability(Hand::Right, None, previous, there))
            .abs()
                < 1e-6
        );
        assert!(prior.is_empty());
    }

    /// Shown a scale over and over, the model should come to expect it.
    #[test]
    fn it_learns_a_habit_it_is_shown() {
        let scale = run(&[
            (60, 1),
            (62, 2),
            (64, 3),
            (65, 1),
            (67, 2),
            (69, 3),
            (71, 4),
            (72, 5),
        ]);
        let mut prior = NgramPrior::new();
        for _ in 0..20 {
            prior.observe(Hand::Right, &scale);
        }

        // After 1 on C going up a tone, the corpus only ever shows 2.
        let previous = Some(Placement::new(60, Finger::Thumb));
        let expected = prior.log_probability(
            Hand::Right,
            None,
            previous,
            Placement::new(62, Finger::Index),
        );
        let unexpected = prior.log_probability(
            Hand::Right,
            None,
            previous,
            Placement::new(62, Finger::Little),
        );
        assert!(
            expected > unexpected + 1.0,
            "expected {expected}, unexpected {unexpected}"
        );
        assert_eq!(prior.observations(), 160);
    }

    /// Nothing is ever ruled out, however lopsided the training data.
    #[test]
    fn nothing_is_impossible() {
        let mut prior = NgramPrior::new();
        for _ in 0..500 {
            prior.observe(Hand::Left, &run(&[(60, 1), (62, 2)]));
        }
        let value = prior.log_probability(
            Hand::Left,
            None,
            Some(Placement::new(60, Finger::Thumb)),
            Placement::new(62, Finger::Ring),
        );
        assert!(value.is_finite() && value < 0.0, "got {value}");
    }

    /// What is learned has to survive a round trip through a file.
    #[test]
    fn a_model_survives_being_saved_and_read_back() {
        let mut prior = NgramPrior::new();
        prior.observe(Hand::Right, &run(&[(60, 1), (64, 3), (67, 5)]));
        prior.observe(Hand::Left, &run(&[(48, 5), (52, 3), (55, 1)]));

        let dir = std::env::temp_dir().join("opennote-prior-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prior.json");
        prior.save(&path).unwrap();
        let read = NgramPrior::load(&path).unwrap();

        let previous = Some(Placement::new(60, Finger::Thumb));
        for finger in [Finger::Thumb, Finger::Index, Finger::Middle, Finger::Ring] {
            let here = Placement::new(64, finger);
            assert!(
                (prior.log_probability(Hand::Right, None, previous, here)
                    - read.log_probability(Hand::Right, None, previous, here))
                .abs()
                    < 1e-6
            );
        }
        assert_eq!(read.observations(), prior.observations());
        std::fs::remove_file(&path).ok();
    }

    /// Every context key has to survive being written and parsed back.
    #[test]
    fn context_keys_round_trip() {
        let contexts = [
            Context::Empty,
            Context::Previous(Finger::Ring),
            Context::Interval(Finger::Thumb, 7),
            Context::Coloured(Finger::Little, 0, 3),
            Context::Trigram(Finger::Index, 25, 2, Finger::Middle),
        ];
        for context in contexts {
            let key = context.key();
            assert_eq!(Context::from_key(&key), Some(context), "key {key}");
        }
    }

    /// The explanation should mention a habit that was actually trained in.
    #[test]
    fn it_can_say_what_it_learned() {
        let mut prior = NgramPrior::new();
        for _ in 0..20 {
            prior.observe(Hand::Right, &run(&[(60, 1), (62, 2), (64, 3)]));
        }
        let said = prior.explain(6).join("\n");
        assert!(said.contains("Right hand"), "{said}");
        assert!(said.contains("uses 2"), "{said}");
    }

    /// Intervals beyond an octave all land in the same bucket, and nothing overflows.
    #[test]
    fn intervals_are_bucketed_without_running_off_the_end() {
        for semitones in -60..=60 {
            assert!(bucket(semitones) < INTERVALS, "{semitones}");
        }
        assert_eq!(bucket(40), bucket(13));
        assert_eq!(bucket(-40), bucket(-13));
        assert_ne!(bucket(12), bucket(13));
    }
}
