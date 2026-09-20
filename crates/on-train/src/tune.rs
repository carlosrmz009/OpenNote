//! Searching the engine's weights, for as long as there is time to search.
//!
//! The engine is rules and a model of a hand, and how much each rule and each kind of
//! strain counts against the others is a set of numbers somebody chose. Nothing about
//! those numbers says they are the best ones. This tries others — a few at a time, on
//! every core, for as long as it is left running — and keeps a setting only when it
//! agrees with what people actually wrote more often than the one it replaces.
//!
//! It is the same idea as training a network and it is done the way the prior's weights
//! already are: by measuring, not by differentiating. There is nothing to differentiate.
//! The engine makes discrete choices — this finger or that one, this hand or the other
//! — and the only honest question to ask of a setting is how many of those choices it
//! gets right.
//!
//! What it is graded against decides what it learns, and that is the part to be careful
//! with. The examples are divided three ways by a hash of their name:
//!
//! * **train** — what the search climbs. A setting is kept here whenever it scores
//!   higher, which on its own would just memorise the training set.
//! * **held out** — what a setting has to improve as well before it is *promoted*, that
//!   is, written out for use. This is the guard against memorising.
//! * **test** — never consulted. It is reported alongside each promotion so that the
//!   figure somebody reads in the morning is one the search could not have chased.
//!
//! A generation is judged on a few hundred examples drawn afresh, not on the whole of
//! the training third: see [`Examples::sample`] for why that is what makes a dataset of
//! any size usable at all. Promotion is always decided on whole thirds.
//!
//! There are three things to search for, and they differ only in what the answer is
//! compared against. [`Target::Hands`] has real ground truth — a two-staff score says
//! which hand plays what — and there are tens of thousands of scores of it.
//! [`Target::Fingers`] has ground truth too, fingerings a pianist wrote, and almost
//! none of it exists. [`Target::Play`] is for the gap between those two: it grades a
//! fingering by measuring it against the hand model afterwards, which can be done on
//! every score there is, and asks whether a hand could play what the engine wrote and
//! how comfortably rather than whether a pianist would have written the same.
//!
//! Fingering settings, however they are graded, have one more gate. The major scales
//! have a settled answer, and a setting that fingers them worse than the defaults do is
//! rejected however well it does otherwise: agreeing with a measure by forgetting how
//! scales go is not an improvement anyone wants.
//!
//! Everything the search needs to carry on is written after every generation, so it can
//! be stopped at any moment and started again without losing more than one.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use on_fingering::scales::scale_fingerings;
use on_fingering::{finger_score, FingeringOptions, Rule, RuleWeights};
use on_hand::Hand;
use on_score::hands::HandAssignment;
use on_score::{Note, NoteId, Score, SourceRef, TieState, TICKS_PER_QUARTER};
use serde::{Deserialize, Serialize};

use crate::corpus::Piece;
use crate::eval::evaluate;

/// What is being tuned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// Which hand plays each note.
    Hands,
    /// Which finger plays each note, graded against fingerings a pianist wrote.
    Fingers,
    /// Which finger plays each note, graded against what a hand can actually do.
    ///
    /// The same numbers as `Fingers`, judged differently. Fingerings somebody wrote
    /// down barely exist — of a quarter of a million public-domain scores, thirty-five
    /// carry one — so on nearly all music the question cannot be "did it write what a
    /// pianist wrote". It can be "could a hand play what it wrote, and comfortably",
    /// and that has an answer on every score there is.
    Play,
}

impl Target {
    fn name(self) -> &'static str {
        match self {
            Target::Hands => "hands",
            Target::Fingers => "fingers",
            Target::Play => "play",
        }
    }

    /// Every target there is, in the order a report should read them.
    pub const ALL: [Target; 3] = [Target::Hands, Target::Fingers, Target::Play];
}

/// Engine weights by name, as they are saved and loaded.
///
/// One file can carry both targets' weights; each is applied only to the settings it
/// names, and a name the engine no longer has is ignored rather than refused, so a
/// file tuned against an older engine still loads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Weights(pub BTreeMap<String, f32>);

impl Weights {
    /// The engine's own defaults for a target.
    pub fn defaults(target: Target) -> Self {
        Self(knobs(target).into_iter().map(|k| (k.name, k.default)).collect())
    }

    /// Read weights saved by the tuner.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading weights from {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Write them out.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Set the hand-assignment weights this names.
    pub fn apply_to_hands(&self, options: &mut HandAssignment) {
        for (name, value) in &self.0 {
            match name.as_str() {
                "hands.overspan" => options.overspan_weight = *value,
                "hands.travel" => options.travel_weight = *value,
                "hands.register" => options.register_weight = *value,
                "hands.abandon" => options.abandon_weight = *value,
                "hands.pivot_window" => options.pivot_window_seconds = f64::from(*value),
                "hands.one_hand_fraction" => options.one_hand_fraction = *value,
                _ => {}
            }
        }
    }

    /// Set the fingering weights this names.
    pub fn apply_to_fingering(&self, options: &mut FingeringOptions) {
        for (name, value) in &self.0 {
            let value = *value;
            if let Some(rule) = name.strip_prefix("rule.") {
                if let Some(rule) = Rule::ALL.into_iter().find(|r| format!("{r:?}") == rule) {
                    options.rule_weights.set(rule, value);
                }
                continue;
            }
            let strain = &mut options.biomech.strain;
            match name.as_str() {
                "scale.rule" => options.rule_scale = value,
                "scale.pattern" => options.pattern_scale = value,
                "biomech.posture" => options.biomech.posture = value,
                "biomech.motion" => options.biomech.motion = value,
                "strain.joint_deviation" => strain.joint_deviation = value,
                "strain.wrist_deviation" => strain.wrist_deviation = value,
                "strain.limit_barrier" => strain.limit_barrier = value,
                "strain.spread" => strain.spread = value,
                "strain.tendon_coupling" => strain.tendon_coupling = value,
                "strain.thumb_opposition" => strain.thumb_opposition = value,
                "strain.carriage" => strain.carriage = value,
                _ => {}
            }
        }
    }
}

/// One number the search may move, and how far.
struct Knob {
    name: String,
    default: f32,
    min: f32,
    max: f32,
}

impl Knob {
    /// A hundredth to a hundred times the default. Every weight here is a cost, and a
    /// cost's size only means anything against the others, so the bound is a factor
    /// rather than an amount.
    ///
    /// It was a factor of ten to begin with, on the reasoning that ten either way is
    /// enough to switch a term off or let it dominate. The first long search ended
    /// with five of its six numbers sitting exactly on a bound, which is the search
    /// saying the answer is on the other side of it. A bound a search leans on is a
    /// fence, not a bound.
    fn scaled(name: impl Into<String>, default: f32) -> Self {
        Self { name: name.into(), default, min: default / 100.0, max: default * 100.0 }
    }
}

/// Everything the search may move for a target, sorted by name.
fn knobs(target: Target) -> Vec<Knob> {
    let mut out = match target {
        Target::Hands => {
            let d = HandAssignment::default();
            vec![
                Knob::scaled("hands.overspan", d.overspan_weight),
                Knob::scaled("hands.travel", d.travel_weight),
                Knob::scaled("hands.register", d.register_weight),
                Knob::scaled("hands.abandon", d.abandon_weight),
                // Seconds, not a cost: from a bar to a page.
                Knob { name: "hands.pivot_window".into(), default: d.pivot_window_seconds as f32, min: 0.5, max: 20.0 },
                // A fraction of a hand's reach: from half of it to a little over all.
                Knob { name: "hands.one_hand_fraction".into(), default: d.one_hand_fraction, min: 0.5, max: 1.2 },
            ]
        }
        // The same numbers; only what they are graded against differs.
        Target::Fingers | Target::Play => {
            let d = FingeringOptions::default();
            let rules = RuleWeights::default();
            let s = d.biomech.strain;
            let mut knobs: Vec<Knob> = Rule::ALL
                .into_iter()
                .map(|rule| Knob::scaled(format!("rule.{rule:?}"), rules.get(rule)))
                .collect();
            knobs.extend([
                Knob::scaled("scale.rule", d.rule_scale),
                Knob::scaled("scale.pattern", d.pattern_scale),
                Knob::scaled("biomech.posture", d.biomech.posture),
                Knob::scaled("biomech.motion", d.biomech.motion),
                Knob::scaled("strain.joint_deviation", s.joint_deviation),
                Knob::scaled("strain.wrist_deviation", s.wrist_deviation),
                Knob::scaled("strain.limit_barrier", s.limit_barrier),
                Knob::scaled("strain.spread", s.spread),
                Knob::scaled("strain.tendon_coupling", s.tendon_coupling),
                Knob::scaled("strain.thumb_opposition", s.thumb_opposition),
                Knob::scaled("strain.carriage", s.carriage),
            ]);
            knobs
        }
    };
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Which third an example belongs to: 0 train, 1 held out, 2 test.
///
/// A hash of its name, and one written out here rather than the standard library's,
/// which is free to change between compiler versions. A search that runs for weeks is
/// resumed across upgrades, and an example that moved from test to train in the middle
/// of one would quietly contaminate the only honest number it reports.
fn bucket(name: &str) -> usize {
    match fnv(name) % 10 {
        0 | 1 => 1,
        2 | 3 => 2,
        _ => 0,
    }
}

/// What the search is graded against, already divided.
pub enum Examples {
    /// Scores whose source says which hand plays what, by name.
    Hands([Vec<(String, Score)>; 3]),
    /// Fingered pieces from the corpus.
    Fingers([Vec<Piece>; 3]),
    /// Scores to be fingered and then measured, with the hands already settled.
    Play([Vec<(String, Score)>; 3]),
}

impl Examples {
    /// The same scores, to be judged on whether a hand could play the fingering rather
    /// than on which hand plays what.
    ///
    /// Every note is put in the hand its source says, so what is measured is the
    /// fingering on its own and not the engine's guess at the hands on top of it.
    ///
    /// Each score is cut to `notes` of them, from the middle, unless that is 0.
    /// Fingering a score costs about three hundred times what deciding its hands does —
    /// it is a search over five fingers at every note rather than two hands at every
    /// chord — and that cost is what a generation is made of. A passage from each of
    /// six thousand pieces says more per second than the whole of six hundred, and
    /// fingering has no long-range structure that only whole pieces would show.
    pub fn playing(self, notes: usize) -> Self {
        match self {
            Examples::Hands(mut parts) => {
                for (_, score) in parts.iter_mut().flatten() {
                    if let Some(truth) = on_score::hands::truth_hands(score) {
                        for (note, hand) in score.notes.iter_mut().zip(truth) {
                            note.hand = hand;
                        }
                    }
                    if notes > 0 && score.notes.len() > notes {
                        let from = (score.notes.len() - notes) / 2;
                        score.notes.drain(..from);
                        score.notes.truncate(notes);
                        // A note's identifier is its place in the list, so cutting the
                        // list renumbers every note that is left.
                        score.finalise();
                    }
                }
                Examples::Play(parts)
            }
            other => other,
        }
    }

    /// Fingered pieces, divided by piece so one piece's annotations stay together.
    pub fn fingers(pieces: Vec<Piece>) -> Self {
        let mut parts: [Vec<Piece>; 3] = Default::default();
        for piece in pieces {
            parts[bucket(&piece.piece)].push(piece);
        }
        Examples::Fingers(parts)
    }

    /// Scores with a hand split in the source, found under some paths.
    ///
    /// A downloaded dataset can be far bigger than a search needs, so at most `limit`
    /// are kept. They are taken in the order of a hash of their path rather than
    /// alphabetically, which spreads them over the whole of it instead of taking every
    /// piece by one composer whose name begins with A. Files that cannot be read, or
    /// whose source does not say which hand plays what, are passed over.
    ///
    /// `min_shared` keeps only music where the hands share the keyboard: the share of
    /// notes that fall inside the other hand's usual range, from 0 to 1. A dataset of
    /// hymns and simple arrangements is mostly two hands a long way apart, where any
    /// setting that splits by register is right; learning from that teaches the engine
    /// to split by register, which is exactly wrong where it matters.
    pub fn hands(
        paths: &[PathBuf],
        limit: usize,
        min_shared: f64,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<Self> {
        let mut files = Vec::new();
        for path in paths {
            walk(path, &mut files)?;
        }
        files.sort_by_key(|p| {
            let text = p.to_string_lossy().to_string();
            (fnv(&text), text)
        });
        let mut parts: [Vec<(String, Score)>; 3] = Default::default();
        let mut kept = 0;
        for (tried, file) in files.iter().enumerate() {
            if kept >= limit {
                break;
            }
            progress(tried, kept);
            let is_json = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"));
            let score = if is_json {
                let Ok(score) = read_music_render(file) else { continue };
                score
            } else {
                let Ok(document) = on_score::Document::open(file) else { continue };
                document.score().clone()
            };
            // A handful of notes says nothing about how a search follows a piece.
            if score.notes.len() < 32 {
                continue;
            }
            let Some(truth) = on_score::hands::truth_hands(&score) else { continue };
            if shared(&score, &truth) < min_shared {
                continue;
            }
            let name = file.to_string_lossy().to_string();
            parts[bucket(&name)].push((name, score));
            kept += 1;
        }
        Ok(Examples::Hands(parts))
    }

    /// How many examples are in each third.
    pub fn sizes(&self) -> [usize; 3] {
        match self {
            Examples::Hands(parts) | Examples::Play(parts) => {
                [parts[0].len(), parts[1].len(), parts[2].len()]
            }
            Examples::Fingers(parts) => [parts[0].len(), parts[1].len(), parts[2].len()],
        }
    }

    /// How well a setting does on every example, whichever third it is in.
    ///
    /// For checking a setting against music the search never saw at all — somebody's
    /// own test pieces — rather than for choosing one.
    pub fn agreement(&self, weights: &Weights) -> f64 {
        let (mut good, mut of) = (0.0, 0.0);
        for part in 0..3 {
            let all: Vec<usize> = (0..self.sizes()[part]).collect();
            let (g, o) = self.tally(weights, part, &all);
            good += g;
            of += o;
        }
        good / of.max(1.0)
    }

    /// How well it does on each example on its own, in a fixed order.
    fn each(&self, weights: &Weights) -> Vec<(String, f64)> {
        let mut out = Vec::new();
        for part in 0..3 {
            for index in 0..self.sizes()[part] {
                let (good, of) = self.tally(weights, part, &[index]);
                out.push((self.name_of(part, index), good / of.max(1.0)));
            }
        }
        out
    }

    /// What an example is called, for the guard's report.
    fn name_of(&self, part: usize, index: usize) -> String {
        match self {
            Examples::Hands(parts) | Examples::Play(parts) => parts[part][index].0.clone(),
            Examples::Fingers(parts) => {
                let piece = &parts[part][index];
                format!("{} ({})", piece.piece, piece.annotator)
            }
        }
    }

    /// Which target these examples are for.
    pub fn target(&self) -> Target {
        match self {
            Examples::Hands(_) => Target::Hands,
            Examples::Fingers(_) => Target::Fingers,
            Examples::Play(_) => Target::Play,
        }
    }

    /// How well a setting does on a whole third, from 0 to 1, over every core.
    ///
    /// This is the slow one, and it is only ever asked for when something is about to
    /// be promoted or reported — never inside a generation, which is why it is free to
    /// take the whole machine.
    fn score(&self, weights: &Weights, part: usize, threads: usize) -> f64 {
        let all: Vec<usize> = (0..self.sizes()[part]).collect();
        if all.is_empty() {
            return 0.0;
        }
        let chunk = all.len().div_ceil(threads.max(1)).max(1);
        let (good, of) = std::thread::scope(|scope| {
            let handles: Vec<_> = all
                .chunks(chunk)
                .map(|pick| scope.spawn(move || self.tally(weights, part, pick)))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or((0.0, 0.0)))
                .fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1))
        });
        good / of.max(1.0)
    }

    /// How well it does on `size` of that third, drawn by `seed`.
    ///
    /// This is what lets a dataset of any size be used at all. Every example is worked
    /// through again for every setting tried, so grading each setting against fifteen
    /// thousand scores makes a generation fifteen thousand scores long, and a night is
    /// then a few hundred settings. Grading it against a few hundred drawn afresh each
    /// generation costs what a few hundred cost, and over a night every score in the
    /// dataset is still seen many times over.
    ///
    /// What matters is that every setting in one generation is judged on the *same*
    /// draw: a score on one sample means nothing held against a score on another. The
    /// draw being different next generation is not a problem but the point, since a
    /// setting that wins only because it suits one handful of pieces stops winning as
    /// soon as the handful changes. Nothing is ever *promoted* on a sample; that is
    /// decided on the whole of the third the search never learns from.
    ///
    /// `size` of 0, or more than there are, is all of them.
    fn sample(&self, weights: &Weights, part: usize, seed: u64, size: usize) -> f64 {
        let len = self.sizes()[part];
        if len == 0 {
            return 0.0;
        }
        let (good, of) = self.tally(weights, part, &draw(seed, size, len));
        good / of.max(1.0)
    }

    /// What one pass over some of a third counted: how much went right, out of how
    /// much there was.
    ///
    /// Counts rather than a rate, because passes are split across cores and a rate
    /// cannot be added to another rate. Everything else here is this divided.
    fn tally(&self, weights: &Weights, part: usize, pick: &[usize]) -> (f64, f64) {
        match self {
            Examples::Hands(parts) => {
                let mut options = HandAssignment::default();
                weights.apply_to_hands(&mut options);
                let (mut right, mut judged) = (0.0, 0.0);
                for (_, score) in pick.iter().map(|i| &parts[part][*i]) {
                    if let Some((r, j)) = on_score::hands::agreement(score, &options) {
                        right += r as f64;
                        judged += j as f64;
                    }
                }
                (right, judged)
            }
            Examples::Play(parts) => {
                let options = fingering_options(weights);
                let spans = options.span_model.table();
                let (mut good, mut of) = (0.0, 0.0);
                for (_, score) in pick.iter().map(|i| &parts[part][*i]) {
                    let solution = finger_score(score, &options);
                    let measured = on_fingering::measure_solution(score, &solution, spans);
                    let faults = measured.unplayable as f64
                        + STRETCH_PRICE * measured.stretched as f64;
                    let notes = score.notes.len() as f64;
                    good += (notes - faults).max(0.0);
                    of += notes;
                }
                (good, of)
            }
            Examples::Fingers(parts) => {
                let options = fingering_options(weights);
                let chosen: Vec<Piece> = pick.iter().map(|i| parts[part][*i].clone()).collect();
                let rates = evaluate(&chosen, &options, None);
                // A mean over the pieces in this pass, kept as a total so that passes
                // over different pieces can be added together.
                (f64::from(rates.general) * rates.pieces as f64, rates.pieces as f64)
            }
        }
    }
}

/// Which of `len` examples a generation is judged on: `size` of them, chosen by
/// `seed`, or all of them if that is 0 or more than there are.
///
/// Drawn with replacement, which for a few hundred out of thousands is a handful of
/// repeats and no reason to keep a set around to avoid them.
fn draw(seed: u64, size: usize, len: usize) -> Vec<usize> {
    if size == 0 || size >= len {
        return (0..len).collect();
    }
    let mut rng = seed | 1;
    (0..size).map(|_| (uniform(&mut rng) * len as f64) as usize % len).collect()
}

/// The engine's fingering settings with these weights applied.
fn fingering_options(weights: &Weights) -> FingeringOptions {
    let mut options = FingeringOptions::default();
    weights.apply_to_fingering(&mut options);
    options
}

/// What one stretch beyond a comfortable span costs, against one transition a hand
/// could not make at all, counted in notes.
///
/// A quarter. The two are not the same kind of fault: one is a fingering that cannot be
/// performed, the other one that can, tightly. But a fingering that is never impossible
/// and always tight is not one anybody wants either, and without the second term the
/// measure is flat over most music — and a flat measure is a week of searching that
/// finds nothing.
const STRETCH_PRICE: f64 = 0.25;

/// FNV-1a.
fn fnv(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// The share of notes played inside the other hand's usual range.
///
/// "Usual" is the middle eight tenths of it, so one stray low note in the right hand
/// does not make a whole piece count as crossing.
fn shared(score: &Score, truth: &[Option<Hand>]) -> f64 {
    let pitches = |hand: Hand| {
        let mut out: Vec<u8> = score
            .notes
            .iter()
            .zip(truth)
            .filter(|(_, h)| **h == Some(hand))
            .map(|(n, _)| n.midi)
            .collect();
        out.sort_unstable();
        out
    };
    let (right, left) = (pitches(Hand::Right), pitches(Hand::Left));
    if right.is_empty() || left.is_empty() {
        return 0.0;
    }
    let at = |sorted: &[u8], p: f64| sorted[(p * (sorted.len() - 1) as f64) as usize];
    let (right_low, left_high) = (at(&right, 0.1), at(&left, 0.9));
    let inside = right.iter().filter(|p| **p <= left_high).count()
        + left.iter().filter(|p| **p >= right_low).count();
    inside as f64 / (right.len() + left.len()) as f64
}

/// Read a score from PDMX's JSON, which is how that dataset ships.
///
/// PDMX was converted from MuseScore into its authors' own format, which keeps a track
/// per staff and drops the staff numbers — and drops fingerings entirely, so it is of no
/// use for fingering. For hands it is: a two-staff piano score arrives as two tracks,
/// and read as a two-track MIDI would be, the higher-sounding one is the right hand.
fn read_music_render(path: &Path) -> Result<Score> {
    let text = std::fs::read_to_string(path)?;
    let json: serde_json::Value = serde_json::from_str(&text)?;
    if json["absolute_time"].as_bool() == Some(true) {
        bail!("absolute time is not handled");
    }
    let resolution = json["resolution"].as_i64().filter(|r| *r > 0).context("no resolution")?;
    let ticks = |time: &serde_json::Value| {
        time.as_i64().map(|t| t * i64::from(TICKS_PER_QUARTER) / resolution)
    };
    let mut score = Score::default();
    for tempo in json["tempos"].as_array().into_iter().flatten() {
        if let (Some(at), Some(qpm)) = (ticks(&tempo["time"]), tempo["qpm"].as_f64()) {
            if qpm > 0.0 {
                score.tempo.insert(at, (60_000_000.0 / qpm) as u32);
            }
        }
    }
    for (track, part) in json["tracks"].as_array().context("no tracks")?.iter().enumerate() {
        for note in part["notes"].as_array().into_iter().flatten() {
            let (Some(onset), Some(duration), Some(midi)) =
                (ticks(&note["time"]), ticks(&note["duration"]), note["pitch"].as_u64())
            else {
                continue;
            };
            score.notes.push(Note {
                id: NoteId(0),
                midi: u8::try_from(midi).unwrap_or(0),
                onset,
                duration: duration.max(1),
                onset_seconds: 0.0,
                duration_seconds: 0.0,
                staff: None,
                voice: None,
                hand: None,
                tie: TieState::default(),
                grace: note["is_grace"].as_bool().unwrap_or(false),
                chord: false,
                velocity: 64,
                given_finger: None,
                source: SourceRef::Midi { track, event: score.notes.len() },
            });
        }
    }
    score.finalise();
    Ok(score)
}

/// Every score file under a path.
fn walk(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path).with_context(|| format!("reading {}", path.display()))? {
            walk(&entry?.path(), out)?;
        }
        return Ok(());
    }
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "mid" | "midi" | "mxl" | "musicxml" | "xml" | "json") {
        out.push(path.to_path_buf());
    }
    Ok(())
}

/// How often the engine fingers the major scales the way they are taught, with the
/// standard patterns and without them. See the `scalebench` example, which this is a
/// compact copy of.
fn scale_agreement(options: &FingeringOptions) -> (f64, f64) {
    const MAJOR: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
    let bare = FingeringOptions { pattern_scale: 0.0, ..options.clone() };
    let mut totals = [(0usize, 0usize); 2];
    for tonic in 0..12u8 {
        for hand in Hand::ALL {
            let base = if hand == Hand::Right { 60 } else { 36 };
            let mut up: Vec<u8> = (0..2)
                .flat_map(|octave| MAJOR.map(|step| base + tonic + step + 12 * octave))
                .collect();
            up.push(base + tonic + 24);
            let mut pitches = up.clone();
            pitches.extend(up.iter().rev().skip(1));
            let optional: Vec<Option<u8>> = pitches.iter().map(|p| Some(*p)).collect();
            let onsets: Vec<f64> = (0..pitches.len()).map(|i| i as f64 * 0.25).collect();
            let want = scale_fingerings(hand, &optional, &onsets);
            let score = melody(&pitches, hand);
            for (slot, options) in [options, &bare].into_iter().enumerate() {
                let solution = finger_score(&score, options);
                let mut got = vec![0u8; pitches.len()];
                for f in &solution.fingerings {
                    got[f.note.0 as usize] = f.finger.number();
                }
                totals[slot].0 += want.iter().filter(|(i, t)| got[**i] == t.finger.number()).count();
                totals[slot].1 += want.len();
            }
        }
    }
    let rate = |(a, b): (usize, usize)| a as f64 / b.max(1) as f64;
    (rate(totals[0]), rate(totals[1]))
}

/// Does a hand holding a bass octave leave a note above it to the other hand?
///
/// The shape of the opening of a great many romantic pieces: the left hand puts down a
/// bass octave and holds it while a figure runs above, and the figure's lowest note sits
/// between the hands. A hand cannot be in two places, so it belongs to the right hand,
/// and the same case is `a_hand_holding_a_chord_does_not_go_and_fetch_something_else` in
/// `on-score`.
///
/// It is a gate on the search for the same reason the scales are a gate on the fingering
/// search. A dataset of easy music will happily pay for a fraction of a point by letting
/// a hand drop what it is holding, because in easy music that almost never comes up; in
/// the music somebody actually wants help with it comes up constantly, and on a screen
/// it is a hand visibly letting go of keys it is still sounding.
fn holds_what_it_is_holding(weights: &Weights) -> bool {
    let mut options = HandAssignment::default();
    weights.apply_to_hands(&mut options);
    let q = i64::from(TICKS_PER_QUARTER);
    let mut score = Score::default();
    let mut push = |midi: u8, onset: i64, duration: i64| {
        score.notes.push(Note {
            id: NoteId(score.notes.len() as u32),
            midi,
            onset,
            duration,
            onset_seconds: 0.0,
            duration_seconds: 0.0,
            staff: None,
            voice: None,
            hand: None,
            tie: TieState::default(),
            grace: false,
            chord: false,
            velocity: 80,
            source: SourceRef::Midi { track: 0, event: 0 },
            given_finger: None,
        });
    };
    // The bass octave, held right through, and a figure above it whose lowest note the
    // left hand could only reach by letting the octave go.
    push(28, 0, 8 * q);
    push(40, 0, 8 * q);
    for (i, midi) in [52u8, 64, 71, 64, 52, 64, 71, 64].into_iter().enumerate() {
        push(midi, (i as i64 + 1) * q, q / 2);
    }
    score.finalise();
    // No staves on it, so this is the search, which is what is being judged.
    on_score::assign_hands(&mut score, &options);
    score.notes.iter().all(|note| {
        let held = note.midi < 41;
        note.hand == Some(if held { Hand::Left } else { Hand::Right })
    })
}

/// A line of eighth notes in one hand.
fn melody(pitches: &[u8], hand: Hand) -> Score {
    let step = i64::from(TICKS_PER_QUARTER) / 2;
    let mut score = Score::default();
    for (i, midi) in pitches.iter().enumerate() {
        score.notes.push(Note {
            id: NoteId(i as u32),
            midi: *midi,
            onset: i as i64 * step,
            duration: step,
            onset_seconds: 0.0,
            duration_seconds: 0.0,
            staff: None,
            voice: None,
            hand: Some(hand),
            tie: TieState::default(),
            grace: false,
            chord: false,
            velocity: 72,
            given_finger: None,
            source: SourceRef::Midi { track: 0, event: i },
        });
    }
    score.finalise();
    score
}

/// Scores on the three thirds.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Scores {
    pub train: f64,
    pub held_out: f64,
    pub test: f64,
}

/// Everything needed to carry on where a search left off.
#[derive(Serialize, Deserialize)]
struct State {
    target: Target,
    /// The names of the knobs, in the order the vectors below are in.
    names: Vec<String>,
    /// Where the search is, as the logarithm of each weight.
    current: Vec<f64>,
    current_train: f64,
    /// How far a step goes, in the same logarithms.
    step: f64,
    rng: u64,
    generation: u64,
    evaluated: u64,
    /// How long has been spent searching altogether, over every run that has ever
    /// carried this state on. Counted from the work, not from when the process
    /// started, so time with the search stopped is not in it.
    #[serde(default)]
    searched_seconds: f64,
    /// Generations since the search last found anything better.
    stalled: u64,
    /// The last setting that was promoted, and what it scored.
    best: Vec<f64>,
    best_scores: Scores,
    /// What the engine's defaults scored, for comparison.
    baseline: Scores,
    /// The defaults' scale agreement, with and without the taught patterns, which a
    /// fingering setting may not fall below.
    scales: (f64, f64),
}

/// How a search is run.
pub struct TuneConfig {
    /// Where the state, the log and the weights go.
    pub directory: PathBuf,
    /// The weights file to write promotions into, shared by both targets.
    pub weights: PathBuf,
    /// Stop after this long, or run until stopped.
    pub hours: Option<f64>,
    /// Candidates tried at once; one per core by default.
    pub threads: usize,
    /// How many examples each generation is judged on, drawn afresh every time. 0 is
    /// all of them, which is slower by however many times more there are.
    pub batch: usize,
    /// Pieces no promoted setting may do worse on than the defaults do.
    ///
    /// A large dataset is mostly easy music, and a setting can win on it by getting
    /// worse at the hard pieces somebody actually cares about. Each piece here is
    /// checked on its own rather than in a total, so a gain on one cannot pay for a
    /// loss on another.
    pub guard: Option<Examples>,
}

/// How far a guarded piece may fall before a setting is refused, as a fraction.
///
/// A quarter of a point: a note or two on a long piece, which is the noise of which
/// side of a boundary one ambiguous chord lands.
const GUARD_SLACK: f64 = 0.0025;

/// Smallest and largest step, in natural-log units.
const STEP_MIN: f64 = 0.02;
const STEP_MAX: f64 = 1.0;
/// Generations without an improvement before the search goes back to the best it has
/// promoted and takes a big step from there.
const RESTART_AFTER: u64 = 150;

/// Search, printing what happens through `say`, until the time is up.
///
/// A (1 + λ) evolution strategy: from where it is, try λ random variations at once,
/// move to the best if it is at least as good, and lengthen the step after a success
/// and shorten it after a failure. That is all, and for a few dozen numbers graded by a
/// score with plateaus in it, it is what works — anything that needs a gradient has
/// nothing to work with here.
pub fn run(examples: &Examples, config: &TuneConfig, mut say: impl FnMut(&str)) -> Result<()> {
    let target = examples.target();
    let knobs = knobs(target);
    let names: Vec<String> = knobs.iter().map(|k| k.name.clone()).collect();
    let lows: Vec<f64> = knobs.iter().map(|k| f64::from(k.min).ln()).collect();
    let highs: Vec<f64> = knobs.iter().map(|k| f64::from(k.max).ln()).collect();
    let weights_of = |x: &[f64]| {
        Weights(names.iter().cloned().zip(x.iter().map(|v| v.exp() as f32)).collect())
    };

    let directory = config.directory.join(target.name());
    std::fs::create_dir_all(&directory)?;
    let state_path = directory.join("state.json");
    let log_path = directory.join("log.csv");
    let sizes = examples.sizes();
    if sizes[0] == 0 || sizes[1] == 0 {
        bail!(
            "need examples to learn from and to hold back; have {} and {}. Add more.",
            sizes[0],
            sizes[1]
        );
    }
    say(&format!(
        "{} examples to learn from, {} held back, {} kept for the final test.",
        sizes[0], sizes[1], sizes[2]
    ));
    let batch = if config.batch == 0 { sizes[0] } else { config.batch.min(sizes[0]) };
    if batch < sizes[0] {
        say(&format!("Judging each generation on {batch} of them, drawn afresh each time."));
    }

    let threads = config.threads.max(1);
    let defaults: Vec<f64> = knobs.iter().map(|k| f64::from(k.default).ln()).collect();
    let resumed = std::fs::read_to_string(&state_path)
        .ok()
        .and_then(|text| serde_json::from_str::<State>(&text).ok())
        .filter(|state| state.target == target && state.names == names);
    let mut state = match resumed {
        Some(mut state) => {
            // The examples may not be the ones it was started on — a bigger download, a
            // different limit — so everything it compares against is measured again.
            state.current_train = examples.score(&weights_of(&state.current), 0, threads);
            state.baseline = score_all(examples, &weights_of(&defaults), threads);
            state.best_scores = score_all(examples, &weights_of(&state.best), threads);
            say(&format!("Carrying on from generation {}.", state.generation));
            state
        }
        None => {
            let baseline = score_all(examples, &weights_of(&defaults), threads);
            let scales = if target == Target::Hands {
                (0.0, 0.0)
            } else {
                scale_agreement(&FingeringOptions::default())
            };
            State {
                target,
                names: names.clone(),
                current: defaults.clone(),
                current_train: baseline.train,
                step: 0.3,
                rng: fnv(target.name()) | 1,
                generation: 0,
                evaluated: 0,
                searched_seconds: 0.0,
                stalled: 0,
                best: defaults.clone(),
                best_scores: baseline,
                baseline,
                scales,
            }
        }
    };
    let guard_floor = config.guard.as_ref().map(|guard| guard.each(&weights_of(&defaults)));
    if let Some(floor) = &guard_floor {
        say(&format!("{} guarded pieces, none of which may get worse.", floor.len()));
    }
    let mut guarded = 0u64;
    say(&format!(
        "The engine's defaults: {:.2}% learning, {:.2}% held back, {:.2}% test.",
        state.baseline.train * 100.0,
        state.baseline.held_out * 100.0,
        state.baseline.test * 100.0
    ));

    let new_log = !log_path.exists();
    let mut log = std::fs::OpenOptions::new().create(true).append(true).open(&log_path)?;
    if new_log {
        writeln!(log, "time,generation,evaluated,step,current_train,best_train,best_held_out,best_test,event")?;
    }

    let started = Instant::now();
    let deadline = config.hours.map(|h| std::time::Duration::from_secs_f64(h * 3600.0));
    let mut last_said = Instant::now();
    let mut since = Instant::now();
    loop {
        if deadline.is_some_and(|d| started.elapsed() >= d) {
            say(&format!(
                "Time is up, after {:.2} hours of searching altogether.",
                state.searched_seconds / 3600.0
            ));
            break;
        }

        // Which examples this generation is judged on. Every candidate in it gets the
        // same ones, and next generation gets different ones.
        let draw = state.rng ^ state.generation.wrapping_mul(0x9e37_79b9_7f4a_7c15);

        // Each child moves a few of the knobs, not all of them. With dozens of knobs,
        // moving every one at once almost always breaks something that was right.
        let children: Vec<Vec<f64>> = (0..threads)
            .map(|_| {
                let share = (3.0 / names.len() as f64).max(0.15);
                let mut child = state.current.clone();
                let mut moved = false;
                while !moved {
                    for (i, value) in child.iter_mut().enumerate() {
                        if uniform(&mut state.rng) < share {
                            *value = (*value + state.step * gaussian(&mut state.rng))
                                .clamp(lows[i], highs[i]);
                            moved = true;
                        }
                    }
                }
                child
            })
            .collect();
        // Where the search stands and the best it has found are judged on this
        // generation's draw alongside the children, because that is the only way their
        // scores can be compared with the children's at all.
        let running: Vec<&Vec<f64>> =
            [&state.current, &state.best].into_iter().chain(children.iter()).collect();
        let scored: Vec<f64> = std::thread::scope(|scope| {
            let handles: Vec<_> = running
                .iter()
                .map(|point| {
                    let weights = weights_of(point);
                    scope.spawn(move || examples.sample(&weights, 0, draw, batch))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or(f64::MIN)).collect()
        });
        let (here, best_here) = (scored[0], scored[1]);
        let scored = &scored[2..];
        state.generation += 1;
        state.evaluated += children.len() as u64;
        state.searched_seconds += since.elapsed().as_secs_f64();
        since = Instant::now();

        let (index, &train) = scored
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .expect("at least one child");
        let mut event = "";
        state.current_train = here;
        if train > here + 1e-12 {
            state.current = children[index].clone();
            state.current_train = train;
            state.step = (state.step * 1.5).min(STEP_MAX);
            state.stalled = 0;
        } else {
            // A tie is taken too. The score has wide plateaus, and the only way across
            // one is to be allowed to wander on it.
            if train >= here - 1e-12 {
                state.current = children[index].clone();
                state.current_train = train;
            }
            state.step *= 0.9;
            state.stalled += 1;
        }
        if state.step < STEP_MIN || state.stalled >= RESTART_AFTER {
            state.current = state.best.clone();
            state.current_train = state.best_scores.train;
            state.step = 0.5;
            state.stalled = 0;
            event = "restart";
        }

        // Promote only what does better on the pieces it never learned from as well.
        // Beating the best on this generation's draw is only what makes it worth the
        // asking; everything below this line is measured on whole thirds, so a promotion
        // is never something a lucky sample bought.
        if state.current_train > best_here + 1e-12 {
            let weights = weights_of(&state.current);
            let held_out = examples.score(&weights, 1, threads);
            let sane = target != Target::Hands || holds_what_it_is_holding(&weights);
            let scales_kept = target == Target::Hands || {
                let mut options = FingeringOptions::default();
                weights.apply_to_fingering(&mut options);
                let (taught, model) = scale_agreement(&options);
                taught >= state.scales.0 - 1e-9 && model >= state.scales.1 - 0.01
            };
            let better = held_out > state.best_scores.held_out + 1e-12 && scales_kept && sane;
            let kept = better
                && match (&config.guard, &guard_floor) {
                    (Some(guard), Some(floor)) => {
                        let now = guard.each(&weights);
                        let held = now.iter().zip(floor).all(|((_, is), (_, was))| *is >= was - GUARD_SLACK);
                        if !held {
                            guarded += 1;
                            event = "guarded";
                        }
                        held
                    }
                    _ => true,
                };
            if kept {
                state.best = state.current.clone();
                state.best_scores = Scores {
                    train: examples.score(&weights, 0, threads),
                    held_out,
                    test: examples.score(&weights, 2, threads),
                };
                let mut file = Weights::load(&config.weights).unwrap_or_default();
                file.0.extend(weights.0);
                // What it took to find these, kept with them. Names beginning with an
                // underscore are not weights and are ignored when they are applied.
                let name = target.name();
                file.0.insert(
                    format!("_{name}.searched_hours"),
                    (state.searched_seconds / 3600.0) as f32,
                );
                file.0.insert(format!("_{name}.settings_tried"), state.evaluated as f32);
                file.0.insert(format!("_{name}.examples"), sizes.iter().sum::<usize>() as f32);
                file.save(&config.weights)?;
                event = "promoted";
                say(&format!(
                    "Generation {}: better. {:.2}% learning, {:.2}% held back, {:.2}% test \
                     (defaults {:.2}%). Saved to {}.",
                    state.generation,
                    state.best_scores.train * 100.0,
                    held_out * 100.0,
                    state.best_scores.test * 100.0,
                    state.baseline.test * 100.0,
                    config.weights.display()
                ));
            }
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        writeln!(
            log,
            "{now},{},{},{:.4},{:.6},{:.6},{:.6},{:.6},{event}",
            state.generation,
            state.evaluated,
            state.step,
            state.current_train,
            state.best_scores.train,
            state.best_scores.held_out,
            state.best_scores.test
        )?;
        // Written to one side and moved over, so being stopped halfway through a write
        // cannot leave a state file that will not load.
        let partial = directory.join("state.json.partial");
        std::fs::write(&partial, serde_json::to_string(&state)?)?;
        std::fs::rename(&partial, &state_path)?;

        if last_said.elapsed().as_secs() >= 600 {
            last_said = Instant::now();
            let refused = if guard_floor.is_some() {
                format!(" {guarded} better settings refused for doing worse on a guarded piece.")
            } else {
                String::new()
            };
            say(&format!(
                "{:.2} hours searched, {} settings tried; \
                 best so far {:.2}% held back (defaults {:.2}%).{refused}",
                state.searched_seconds / 3600.0,
                state.evaluated,
                state.best_scores.held_out * 100.0,
                state.baseline.held_out * 100.0
            ));
        }
    }
    Ok(())
}

/// What the searches under a directory have done, in sentences.
///
/// The hours are what was actually spent searching, added up over every run there has
/// ever been, and they keep counting as long as the state files are kept.
pub fn report(directory: &Path) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    let mut hours = 0.0;
    let mut tried = 0u64;
    for target in Target::ALL {
        let path = directory.join(target.name()).join("state.json");
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let state: State = serde_json::from_str(&text)
            .with_context(|| format!("reading {}", path.display()))?;
        hours += state.searched_seconds / 3600.0;
        tried += state.evaluated;
        lines.push(format!(
            "{}: {:.2} hours, {} settings tried over {} generations.",
            target.name(),
            state.searched_seconds / 3600.0,
            state.evaluated,
            state.generation
        ));
        let measure = match target {
            Target::Play => "music a hand can play comfortably, on the third it never \
                             learned from",
            _ => "agreement with the people who wrote the music down, on the third it \
                  never learned from",
        };
        lines.push(format!(
            "  {measure}: {:.2}%, against {:.2}% for the engine's defaults at the time.",
            state.best_scores.test * 100.0,
            state.baseline.test * 100.0
        ));
    }
    if lines.is_empty() {
        lines.push(format!("Nothing has been searched under {} yet.", directory.display()));
    } else {
        lines.push(format!("Altogether: {hours:.2} hours, {tried} settings tried."));
    }
    Ok(lines)
}

fn score_all(examples: &Examples, weights: &Weights, threads: usize) -> Scores {
    Scores {
        train: examples.score(weights, 0, threads),
        held_out: examples.score(weights, 1, threads),
        test: examples.score(weights, 2, threads),
    }
}

/// SplitMix64, uniform on [0, 1).
fn uniform(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// Standard normal, by Box-Muller.
fn gaussian(state: &mut u64) -> f64 {
    let u = uniform(state).max(f64::MIN_POSITIVE);
    let v = uniform(state);
    (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_survive_a_round_trip_and_set_what_they_name() {
        let mut weights = Weights::defaults(Target::Fingers);
        weights.0.insert("strain.spread".into(), 3.25);
        weights.0.insert("rule.WeakFinger".into(), 0.5);
        weights.0.extend(Weights::defaults(Target::Hands).0);
        weights.0.insert("hands.register".into(), 1.5);
        weights.0.insert("not.a.knob".into(), 9.0);

        let path = std::env::temp_dir().join("opennote-weights-test.json");
        weights.save(&path).unwrap();
        let back = Weights::load(&path).unwrap();
        assert_eq!(back, weights);

        let mut fingering = FingeringOptions::default();
        back.apply_to_fingering(&mut fingering);
        assert_eq!(fingering.biomech.strain.spread, 3.25);
        assert_eq!(fingering.rule_weights.get(Rule::WeakFinger), 0.5);
        let mut hands = HandAssignment::default();
        back.apply_to_hands(&mut hands);
        assert_eq!(hands.register_weight, 1.5);
    }

    #[test]
    fn the_defaults_change_nothing() {
        let mut fingering = FingeringOptions::default();
        Weights::defaults(Target::Fingers).apply_to_fingering(&mut fingering);
        let plain = FingeringOptions::default();
        assert_eq!(fingering.rule_weights, plain.rule_weights);
        assert_eq!(fingering.biomech, plain.biomech);
        assert_eq!(fingering.rule_scale, plain.rule_scale);
        assert_eq!(fingering.pattern_scale, plain.pattern_scale);

        let mut hands = HandAssignment::default();
        Weights::defaults(Target::Hands).apply_to_hands(&mut hands);
        let plain = HandAssignment::default();
        assert_eq!(hands.overspan_weight, plain.overspan_weight);
        assert_eq!(hands.pivot_window_seconds, plain.pivot_window_seconds);
        assert_eq!(hands.one_hand_fraction, plain.one_hand_fraction);
    }

    #[test]
    fn an_example_stays_in_the_same_third() {
        // FNV-1a's published test vector. If this moves, every search that is resumed
        // has just had its test set changed under it.
        assert_eq!(fnv("a"), 0xaf63_dc4c_8601_ec8c);
        let spread: Vec<usize> = (0..1000).map(|i| bucket(&format!("piece-{i}"))).collect();
        let held = spread.iter().filter(|b| **b == 1).count();
        let test = spread.iter().filter(|b| **b == 2).count();
        assert!((150..250).contains(&held) && (150..250).contains(&test), "{held} {test}");
    }

    /// The gate has to pass what the engine does now and refuse what breaks it, or it
    /// is not a gate. The weights refused here are the ones an hour and a half of
    /// searching against PDMX actually produced.
    #[test]
    fn a_setting_that_drops_held_notes_is_refused() {
        assert!(holds_what_it_is_holding(&Weights::defaults(Target::Hands)));
        let mut greedy = Weights::defaults(Target::Hands);
        greedy.0.insert("hands.abandon".into(), 0.041);
        greedy.0.insert("hands.overspan".into(), 0.4);
        greedy.0.insert("hands.travel".into(), 0.0004);
        greedy.0.insert("hands.register".into(), 6.0);
        greedy.0.insert("hands.pivot_window".into(), 1.31);
        greedy.0.insert("hands.one_hand_fraction".into(), 0.5);
        assert!(!holds_what_it_is_holding(&greedy));
    }

    /// A few scores of the same shape, to count over.
    fn some_music() -> Vec<(String, Score)> {
        (0..7u8)
            .map(|i| {
                let pitches: Vec<u8> = (0..40).map(|n| 55 + i + (n * 5) % 19).collect();
                (format!("piece-{i}"), melody(&pitches, Hand::Right))
            })
            .collect()
    }

    /// Splitting a pass over the cores has to count exactly what one core counts, or
    /// the figure reported in the morning depends on the machine it ran on.
    #[test]
    fn a_pass_split_across_cores_counts_the_same() {
        let examples = Examples::Play([some_music(), Vec::new(), Vec::new()]);
        let weights = Weights::defaults(Target::Play);
        let one = examples.score(&weights, 0, 1);
        assert!((0.0..=1.0).contains(&one), "{one}");
        for threads in [2, 3, 8, 64] {
            assert_eq!(one, examples.score(&weights, 0, threads), "on {threads} threads");
        }
    }

    /// Every setting in a generation has to be judged on the same draw, and the next
    /// generation on a different one. Otherwise the comparison between two settings is
    /// a comparison between two sets of music.
    #[test]
    fn a_generations_draw_is_the_same_for_every_setting_and_new_next_time() {
        assert_eq!(draw(11, 300, 9000), draw(11, 300, 9000));
        assert_ne!(draw(11, 300, 9000), draw(12, 300, 9000));
        assert!(draw(11, 300, 9000).iter().all(|i| *i < 9000));
        // Asking for as many as there are, or for none, is all of them in order.
        assert_eq!(draw(11, 0, 5), vec![0, 1, 2, 3, 4]);
        assert_eq!(draw(11, 9, 5), vec![0, 1, 2, 3, 4]);
        // Over many generations every example is drawn, rather than the same tenth of
        // them every time.
        let mut seen = std::collections::BTreeSet::new();
        for seed in 0..200 {
            seen.extend(draw(seed, 20, 300));
        }
        assert_eq!(seen.len(), 300, "only {} of 300 were ever drawn", seen.len());
    }

    /// The playing target cuts each score down, and takes it from the middle rather
    /// than the opening, which is where a piece is least like itself.
    #[test]
    fn playing_keeps_the_middle_of_a_score() {
        let pitches: Vec<u8> = (0..100).map(|n| 40 + n / 2).collect();
        let score = melody(&pitches, Hand::Right);
        let examples = Examples::Hands([vec![("piece".into(), score)], Vec::new(), Vec::new()]);
        let Examples::Play(parts) = examples.playing(10) else { panic!("not the playing target") };
        let kept = &parts[0][0].1.notes;
        assert_eq!(kept.len(), 10);
        assert_eq!(kept[0].midi, pitches[45]);
        // A note's identifier is where it sits in the list, and everything that looks a
        // note up goes through that, so the cut has to renumber them.
        assert!(kept.iter().enumerate().all(|(i, n)| n.id == NoteId(i as u32)));
    }

    #[test]
    fn the_random_numbers_are_the_ones_a_resumed_search_expects() {
        let mut a = 42u64;
        let mut b = 42u64;
        let first: Vec<f64> = (0..5).map(|_| gaussian(&mut a)).collect();
        let second: Vec<f64> = (0..5).map(|_| gaussian(&mut b)).collect();
        assert_eq!(first, second);
        let mean = (0..20_000).map(|_| gaussian(&mut a)).sum::<f64>() / 20_000.0;
        assert!(mean.abs() < 0.05, "{mean}");
    }
}
