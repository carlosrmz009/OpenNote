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
//!
//! # Symmetry
//!
//! Backing off to a more general *context* is not the only way to make a small corpus
//! go further. A fingering is also the same fingering when the keyboard is looked at
//! in a mirror, or when the passage is played backwards, and two of the symmetries in
//! Nakamura, Saito & Yoshii (2020, sec. 5.2.2) say so:
//!
//! * **reflection** — the left hand is the right hand mirrored. Reflecting pitch about
//!   any D maps white keys to white and black to black, so it is an exact symmetry of
//!   the keyboard rather than an approximation: every interval flips sign and every
//!   colour survives. The thumb lands where the thumb was, because it lies on the low
//!   side of the right hand and the high side of the left.
//! * **time inversion** — a passage played from the end costs a hand what it cost
//!   forwards. Malwine Bree wrote this down for Leschetizky's students in 1902, and
//!   the scale fingerings bear it out: a rising C major right hand, 1-2-3-1-2-3-4-5,
//!   reversed is exactly the fingering taught for the descent.
//!
//! Both fill a second table per hand, in that hand's own frame, holding every image of
//! everything ever observed. Those tables are not a *more general context* — they hold
//! the same contexts, seen several times over — so they sit between the hand's own
//! counts and the backoff: an estimate is shrunk first toward what the symmetries say,
//! and only then toward the more general context.
//!
//! Per hand, and reflected on the way in rather than on the way out, because the two
//! hands answer the same context oppositely: a rising interval after the thumb means
//! one thing to a right hand and the reverse to a left one. Pooling them unreflected
//! measures worse than not pooling at all.
//!
//! This is what makes the symmetries free rather than a trade. Nakamura et al. found
//! (sec. 6.5, Fig. 7) that imposing them *raises* accuracy on a small corpus and
//! *lowers* it on a large one, because a real pianist is not quite symmetric and
//! enough data eventually shows it. Putting them in the backoff chain rather than into
//! the model gets both halves of that finding at once: with little data the hand's own
//! counts are thin and the pooled estimate carries, and with a lot of data they
//! dominate it and whatever asymmetry is real survives. There is nothing to switch and
//! nothing to tune.

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

/// The pitch that plays the same part in the other hand.
///
/// Reflection about D4. Any D would do, and so would any G sharp: those are the two
/// axes the pattern of black keys is symmetric about, which is what makes this an
/// exact symmetry rather than a near one. White keys map to white, black to black, and
/// every interval flips sign.
///
/// The result can land off the end of the keyboard, and it does not matter. Nothing
/// downstream reads the pitch itself — only the interval between two of them and the
/// colour of each, and the reflection preserves both wherever it lands.
fn reflect(midi: u8) -> u8 {
    (2 * 62i32 - i32::from(midi)) as u8
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

    /// One Witten-Bell step: shrink a broader estimate toward what was seen here.
    ///
    /// The estimate from this table is trusted in proportion to how much was seen in
    /// this context, and the number of *distinct* fingers seen sets how much weight
    /// `broader` keeps. A context with one observation of one finger barely moves it;
    /// a well-populated one nearly replaces it. Seeing nothing at all passes it
    /// through untouched, which is what makes the chain safe to extend.
    fn shrink(&self, context: &Context, finger: Finger, broader: f32) -> f32 {
        let Some(tally) = self.tallies.get(context) else {
            return broader;
        };
        let total: u32 = tally.iter().sum();
        if total == 0 {
            return broader;
        }
        let distinct = tally.iter().filter(|c| **c > 0).count() as f32;
        let count = tally[finger.index()] as f32;
        (count + distinct * broader) / (total as f32 + distinct)
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
    /// Every observation under its symmetry images, one table per hand and each in
    /// that hand's own frame. See the module documentation: this is what a thin corpus
    /// falls back on.
    ///
    /// Per hand rather than shared, because the two hands answer the same context
    /// oppositely — a rising interval after the thumb means one thing to a right hand
    /// and the reverse to a left one. Pooling them unreflected would average two
    /// contradictory distributions together, which measures worse than not pooling at
    /// all.
    pooled: [HandCounts; 2],
    /// Which symmetries fill that table. Measured rather than assumed; see
    /// `cargo run -p on-fingering --example symmetry`.
    symmetries: Symmetries,
}

/// Which of the two symmetries the pooled table is allowed to assume.
///
/// They are not equally safe. Nakamura, Saito & Yoshii (2020, sec. 6.5) found the
/// reflection to be the stronger assumption of the two — "the degree of asymmetry is
/// larger for the reflection symmetry than the time inversion symmetry" — and that is
/// visible here too, so the two can be chosen between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Symmetries {
    /// Neither: each hand learns only from itself, played only forwards.
    None,
    /// Time inversion only: a hand learns from its own passages played backwards.
    /// Adds nothing a balanced corpus does not already have, and is here because
    /// Nakamura et al. found it the safer of the two on a large one.
    Time,
    /// Both, including the reflection that ties the two hands together. The default:
    /// measured to help most where there is least data, by 11 points on one scale and
    /// still 4 on six, and to be the only thing that carries anything at all from one
    /// hand to the other.
    #[default]
    Full,
}

impl NgramPrior {
    /// An empty model, which says every finger is equally likely.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty model assuming the given symmetries.
    pub fn with_symmetries(symmetries: Symmetries) -> Self {
        Self { symmetries, ..Self::default() }
    }

    /// Learn from one hand's part of one piece.
    ///
    /// `placements` are the notes of that hand in time order, each with the finger a
    /// pianist actually used. Every context this note falls into is counted, from the
    /// most specific down, which is what lets the backoff work later.
    ///
    /// The same notes are counted a second time into the pooled table, under each of
    /// the four images the two symmetries generate: as played, mirrored into the other
    /// hand, reversed in time, and both at once.
    pub fn observe(&mut self, hand: Hand, placements: &[Placement]) {
        Self::count(&mut self.hands[hand as usize], placements);
        if self.symmetries == Symmetries::None {
            return;
        }

        // Time inversion: this hand's own table gets the passage forwards and
        // backwards.
        let mut backwards = placements.to_vec();
        backwards.reverse();
        Self::count(&mut self.pooled[hand as usize], placements);
        Self::count(&mut self.pooled[hand as usize], &backwards);

        if self.symmetries != Symmetries::Full {
            return;
        }
        // Reflection: the *other* hand's table gets the same passage mirrored, which is
        // what that hand would have played. Both images go in, so each hand's pooled
        // table ends up holding all four.
        let other = match hand {
            Hand::Right => Hand::Left,
            Hand::Left => Hand::Right,
        };
        let mirrored: Vec<Placement> = placements
            .iter()
            .map(|p| Placement::new(reflect(p.midi), p.finger))
            .collect();
        let mut mirrored_backwards = mirrored.clone();
        mirrored_backwards.reverse();
        Self::count(&mut self.pooled[other as usize], &mirrored);
        Self::count(&mut self.pooled[other as usize], &mirrored_backwards);
    }

    /// Tally one sequence of placements into one table.
    fn count(counts: &mut HandCounts, placements: &[Placement]) {
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

    /// Probability of `finger` in this context, smoothed.
    ///
    /// Three tiers, narrowest first: what this hand did here, what the symmetries say
    /// about here, and what happens in the more general context. Each shrinks the one
    /// behind it, so a tier with nothing in it costs nothing.
    fn probability(&self, hand: Hand, context: &Context, finger: Finger) -> f32 {
        let general = match context.backoff() {
            Some(parent) => self.probability(hand, &parent, finger),
            // At the very bottom, before anything has been seen, every finger is
            // equally likely. This is the only place a number is invented.
            None => 0.2,
        };
        // Always in this hand's own frame: the reflection was applied when the
        // observations went in, not when the question is asked.
        let pooled = self.pooled[hand as usize].shrink(context, finger, general);
        self.hands[hand as usize].shrink(context, finger, pooled)
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
            version: 2,
            symmetries: self.symmetries,
            left: serialize_hand(&self.hands[Hand::Left as usize]),
            right: serialize_hand(&self.hands[Hand::Right as usize]),
            pooled_left: serialize_hand(&self.pooled[Hand::Left as usize]),
            pooled_right: serialize_hand(&self.pooled[Hand::Right as usize]),
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
            stored.version == 2,
            "this model was written by a different version of opennote; retrain it"
        );
        let mut prior = Self::new();
        prior.hands[Hand::Left as usize] = deserialize_hand(&stored.left);
        prior.hands[Hand::Right as usize] = deserialize_hand(&stored.right);
        prior.pooled[Hand::Left as usize] = deserialize_hand(&stored.pooled_left);
        prior.pooled[Hand::Right as usize] = deserialize_hand(&stored.pooled_right);
        prior.symmetries = stored.symmetries;
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
        self.probability(hand, &context, current.finger)
            .max(1e-4)
            .ln()
    }
}

/// The model as it is stored on disk.
#[derive(Serialize, Deserialize)]
struct StoredModel {
    version: u32,
    left: HashMap<String, Tally>,
    right: HashMap<String, Tally>,
    /// The symmetry images, which cannot be rebuilt on load: the placements they were
    /// derived from are gone by then.
    pooled_left: HashMap<String, Tally>,
    /// As above, for the right hand.
    pooled_right: HashMap<String, Tally>,
    /// Which symmetries built that table. Stored because it decides the frame a
    /// left-hand query is asked in, so reading it back wrong would silently mirror
    /// every left-hand lookup.
    #[serde(default)]
    symmetries: Symmetries,
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


    /// Reflecting about a D is an exact symmetry of the keyboard: it is its own
    /// inverse, it never turns a white key black, and it flips every interval.
    #[test]
    fn reflection_is_an_exact_symmetry_of_the_keyboard() {
        use on_hand::keyboard::is_black;
        for midi in 21..=108u8 {
            assert_eq!(reflect(reflect(midi)), midi, "midi {midi}");
            assert_eq!(
                is_black(reflect(midi)),
                is_black(midi),
                "reflecting {midi} changed its colour"
            );
        }
        for (low, high) in [(60u8, 64u8), (61, 66), (70, 71)] {
            let forward = i32::from(high) - i32::from(low);
            let mirrored = i32::from(reflect(high)) - i32::from(reflect(low));
            assert_eq!(forward, -mirrored, "{low} to {high}");
        }
    }

    /// The point of the reflection symmetry: a hand learns from the other hand.
    ///
    /// Shown a right-hand scale and nothing else, the model should have an opinion
    /// about the mirrored left-hand passage — which, with separate per-hand tables and
    /// no pooling, it could not have.
    #[test]
    fn what_one_hand_is_shown_teaches_the_other() {
        let scale = run(&[(60, 1), (62, 2), (64, 3), (65, 1), (67, 2), (69, 3)]);
        let mut prior = NgramPrior::new();
        for _ in 0..20 {
            prior.observe(Hand::Right, &scale);
        }

        // The same passage in the mirror, which is a left hand descending. After 64
        // with the middle finger, the taught continuation is the thumb on 65 — so in
        // the mirror, after reflect(64) with the middle finger comes reflect(65) with
        // the thumb.
        let previous = Some(Placement::new(reflect(64), Finger::Middle));
        let taught = Placement::new(reflect(65), Finger::Thumb);
        let not_taught = Placement::new(reflect(65), Finger::Little);
        let left_taught = prior.log_probability(Hand::Left, None, previous, taught);
        let left_other = prior.log_probability(Hand::Left, None, previous, not_taught);
        assert!(
            left_taught > left_other,
            "the left hand learned nothing from the right: {left_taught} vs {left_other}"
        );
    }

    /// The point of the time inversion symmetry: a rising passage teaches the fall.
    ///
    /// Shown only the ascent, the model should expect the descending fingering, which
    /// is the ascent read backwards. Bree recorded the same property of fingering for
    /// Leschetizky's students in 1902.
    #[test]
    fn a_rising_passage_teaches_the_falling_one() {
        let up = run(&[(60, 1), (62, 2), (64, 3), (65, 1), (67, 2)]);
        let mut prior = NgramPrior::new();
        for _ in 0..20 {
            prior.observe(Hand::Right, &up);
        }

        // Coming down onto 64: the ascent has 64 with the middle finger before the
        // thumb on 65, so the descent from 65 should reach 64 with the middle finger.
        let previous = Some(Placement::new(65, Finger::Thumb));
        let taught = Placement::new(64, Finger::Middle);
        let not_taught = Placement::new(64, Finger::Little);
        let down_taught = prior.log_probability(Hand::Right, None, previous, taught);
        let down_other = prior.log_probability(Hand::Right, None, previous, not_taught);
        assert!(
            down_taught > down_other,
            "the descent learned nothing from the ascent: {down_taught} vs {down_other}"
        );
    }

    /// The symmetries must not overrule what the hand was actually shown.
    ///
    /// This is the half of Nakamura et al.'s finding that says a real pianist is not
    /// quite symmetric: given enough evidence that one hand does something its mirror
    /// does not, the hand's own counts have to win. They sit in front of the pooled
    /// table precisely so that they can.
    #[test]
    fn a_hands_own_evidence_outweighs_the_symmetries() {
        let mut prior = NgramPrior::new();
        // The right hand is shown one thing a great many times...
        let habit = run(&[(60, 1), (62, 2)]);
        for _ in 0..200 {
            prior.observe(Hand::Right, &habit);
        }
        // ...and the left hand the mirrored context resolved the other way, once.
        let contrary = run(&[(reflect(60), 1), (reflect(62), 4)]);
        prior.observe(Hand::Left, &contrary);

        let previous = Some(Placement::new(60, Finger::Thumb));
        let shown = prior.log_probability(
            Hand::Right,
            None,
            previous,
            Placement::new(62, Finger::Index),
        );
        let mirrored_only = prior.log_probability(
            Hand::Right,
            None,
            previous,
            Placement::new(62, Finger::Ring),
        );
        assert!(
            shown > mirrored_only,
            "the mirror overruled two hundred observations: {shown} vs {mirrored_only}"
        );
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
