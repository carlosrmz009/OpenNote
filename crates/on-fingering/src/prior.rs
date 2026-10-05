use std::collections::HashMap;

use on_hand::{Finger, Hand};
use serde::{Deserialize, Serialize};

use crate::rules::Placement;
use crate::solver::FingeringPrior;

const MAX_INTERVAL: i32 = 12;

#[cfg(test)]
const INTERVALS: usize = (2 * MAX_INTERVAL + 1) as usize + 2;

type Tally = [u32; 5];

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Context {
    Empty,
    Previous(Finger),
    Interval(Finger, usize),
    Coloured(Finger, usize, u8),
    Trigram(Finger, usize, u8, Finger),
}

impl Context {
    fn backoff(&self) -> Option<Context> {
        match *self {
            Context::Empty => None,
            Context::Previous(_) => Some(Context::Empty),
            Context::Interval(f, _) => Some(Context::Previous(f)),
            Context::Coloured(f, d, _) => Some(Context::Interval(f, d)),
            Context::Trigram(f, d, c, _) => Some(Context::Coloured(f, d, c)),
        }
    }

    fn key(&self) -> String {
        match *self {
            Context::Empty => "-".into(),
            Context::Previous(f) => format!("p{}", f.number()),
            Context::Interval(f, d) => format!("i{}.{d}", f.number()),
            Context::Coloured(f, d, c) => format!("c{}.{d}.{c}", f.number()),
            Context::Trigram(f, d, c, g) => format!("t{}.{d}.{c}.{}", f.number(), g.number()),
        }
    }

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

fn bucket(semitones: i32) -> usize {
    (semitones.clamp(-MAX_INTERVAL - 1, MAX_INTERVAL + 1) + MAX_INTERVAL + 1) as usize
}

fn reflect(midi: u8) -> u8 {
    (2 * 62i32 - i32::from(midi)) as u8
}

fn colours(from: u8, to: u8) -> u8 {
    u8::from(on_hand::keyboard::is_black(from)) * 2 + u8::from(on_hand::keyboard::is_black(to))
}

#[derive(Debug, Default, Clone)]
struct HandCounts {
    tallies: HashMap<Context, Tally>,
}

impl HandCounts {
    fn observe(&mut self, context: Context, finger: Finger) {
        self.tallies.entry(context).or_insert([0; 5])[finger.index()] += 1;
    }

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

    fn observations(&self) -> u32 {
        self.tallies
            .get(&Context::Empty)
            .map(|t| t.iter().sum())
            .unwrap_or(0)
    }
}

#[derive(Debug, Default, Clone)]
pub struct NgramPrior {
    hands: [HandCounts; 2],
    pooled: [HandCounts; 2],
    symmetries: Symmetries,
    pub learned: Option<crate::learned::Learned>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Symmetries {
    None,
    Time,
    #[default]
    Full,
}

impl NgramPrior {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_symmetries(symmetries: Symmetries) -> Self {
        Self { symmetries, ..Self::default() }
    }

    pub fn observe(&mut self, hand: Hand, placements: &[Placement]) {
        Self::count(&mut self.hands[hand as usize], placements);
        if self.symmetries == Symmetries::None {
            return;
        }

        let mut backwards = placements.to_vec();
        backwards.reverse();
        Self::count(&mut self.pooled[hand as usize], placements);
        Self::count(&mut self.pooled[hand as usize], &backwards);

        if self.symmetries != Symmetries::Full {
            return;
        }
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

    fn probability(&self, hand: Hand, context: &Context, finger: Finger) -> f32 {
        let general = match context.backoff() {
            Some(parent) => self.probability(hand, &parent, finger),
            None => 0.2,
        };
        let pooled = self.pooled[hand as usize].shrink(context, finger, general);
        self.hands[hand as usize].shrink(context, finger, pooled)
    }

    pub fn observations(&self) -> u32 {
        self.hands.iter().map(HandCounts::observations).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.observations() == 0
    }

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
            learned: self.learned.clone(),
        };
        let text = serde_json::to_string_pretty(&stored)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }

    pub fn from_json(text: &str) -> anyhow::Result<Self> {
        let stored: StoredModel = serde_json::from_str(text)?;
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
        prior.learned = stored.learned;
        Ok(prior)
    }

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
            habits.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
            for (_, _, line) in habits.into_iter().take(per_hand) {
                lines.push(format!("  {line}"));
            }
        }
        lines
    }
}

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
        self.probability(hand, &context, current.finger)
            .max(1e-4)
            .ln()
    }

    fn chord_cost(&self, hand: Hand, notes: &[u8], fingers: &[Finger]) -> f32 {
        self.learned.as_ref().map_or(0.0, |l| l.chord(hand, notes, fingers))
    }

    fn step_cost(
        &self,
        hand: Hand,
        from: (&[u8], &[Finger]),
        to: (&[u8], &[Finger]),
        seconds: f64,
    ) -> f32 {
        self.learned.as_ref().map_or(0.0, |l| l.step(hand, from, to, seconds))
    }

    fn trigram_cost(
        &self,
        hand: Hand,
        a: (&[u8], &[Finger]),
        b: (&[u8], &[Finger]),
        c: (&[u8], &[Finger]),
    ) -> f32 {
        self.learned.as_ref().map_or(0.0, |l| l.trigram(hand, a, b, c))
    }
}

#[derive(Serialize, Deserialize)]
struct StoredModel {
    version: u32,
    left: HashMap<String, Tally>,
    right: HashMap<String, Tally>,
    pooled_left: HashMap<String, Tally>,
    pooled_right: HashMap<String, Tally>,
    #[serde(default)]
    symmetries: Symmetries,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    learned: Option<crate::learned::Learned>,
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

    #[test]
    fn what_one_hand_is_shown_teaches_the_other() {
        let scale = run(&[(60, 1), (62, 2), (64, 3), (65, 1), (67, 2), (69, 3)]);
        let mut prior = NgramPrior::new();
        for _ in 0..20 {
            prior.observe(Hand::Right, &scale);
        }

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

    #[test]
    fn a_rising_passage_teaches_the_falling_one() {
        let up = run(&[(60, 1), (62, 2), (64, 3), (65, 1), (67, 2)]);
        let mut prior = NgramPrior::new();
        for _ in 0..20 {
            prior.observe(Hand::Right, &up);
        }

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

    #[test]
    fn a_hands_own_evidence_outweighs_the_symmetries() {
        let mut prior = NgramPrior::new();
        let habit = run(&[(60, 1), (62, 2)]);
        for _ in 0..200 {
            prior.observe(Hand::Right, &habit);
        }
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

    #[test]
    fn an_octave_is_already_the_same_passage() {
        let low = run(&[(60, 1), (62, 2), (64, 3), (65, 1)]);
        let high: Vec<Placement> = low
            .iter()
            .map(|p| Placement::new(p.midi + 12, p.finger))
            .collect();

        let mut from_low = NgramPrior::new();
        from_low.observe(Hand::Right, &low);
        let mut from_high = NgramPrior::new();
        from_high.observe(Hand::Right, &high);

        for finger in Finger::ALL {
            let asked_high = Placement::new(76, finger);
            let taught_by_low = from_low.log_probability(
                Hand::Right,
                Some(high[1]),
                Some(high[2]),
                asked_high,
            );
            let asked_low = Placement::new(64, finger);
            let taught_by_high = from_high.log_probability(
                Hand::Right,
                Some(low[1]),
                Some(low[2]),
                asked_low,
            );
            assert!(
                (taught_by_low - taught_by_high).abs() < 1e-6,
                "{finger:?}: {taught_by_low} vs {taught_by_high}"
            );
        }
    }

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
