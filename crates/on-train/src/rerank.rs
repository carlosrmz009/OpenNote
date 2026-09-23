//! Teaching the search to finger the way the pianists in the corpus do.
//!
//! The search itself is the model. For every training piece it is run twice: once
//! freely, and once with the fingers the pianist was confidently seen to use pinned in
//! place, which gives the best path the current weights allow that agrees with the
//! pianist. Where the two differ, the facts the free path used are made a little dearer
//! and the facts the pianist's path used a little cheaper — the structured perceptron
//! (Collins, 2002), with its weights averaged over the run, which is what keeps one noisy
//! batch from undoing the rest. Only confident fingers are pinned; the rest of a piece is
//! left for the search to fill in, so a doubtful reading teaches nothing.
//!
//! Progress is measured after each pass on pieces held back from training, and the
//! weights from the best pass are the ones kept. Nothing here looks at PIG.

use std::collections::HashMap;

use on_fingering::learned::{chord_features, step_features, Learned};
use on_fingering::{finger_score_with_prior, FingeringOptions, NgramPrior, Step};
use on_hand::Finger;
use on_score::{NoteId, Score};

use crate::corpus::{HandLabel, Piece};
use crate::eval::{combine, per_piece, rebuild, MatchRates};
use crate::train::split;

/// How a training run is done.
#[derive(Debug, Clone)]
pub struct Config {
    /// Passes over the training pieces.
    pub epochs: usize,
    /// How far one disagreement moves a weight, in the cost function's own units.
    pub rate: f32,
    /// Fingers read with less confidence than this are not pinned.
    pub floor: f32,
    /// The share of pieces held back to measure on.
    pub held_out: f32,
    /// Pieces a pass uses, drawn afresh each pass; 0 for all of them.
    pub per_epoch: usize,
    /// Pieces fingered at once.
    pub threads: usize,
}

/// What a training run did.
pub struct Report {
    /// The model: an empty n-gram table carrying the learned weights.
    pub model: NgramPrior,
    /// The held-back pieces fingered by the rules alone.
    pub before: MatchRates,
    /// The same after each pass, with that pass's averaged weights.
    pub passes: Vec<MatchRates>,
    /// Which pass was kept.
    pub best: usize,
    /// How many training and held-back pieces there were.
    pub sizes: (usize, usize),
}

/// One training piece, ready to finger both ways.
struct Example {
    free: Score,
    pinned: Score,
}

/// A piece with its confident fingers pinned — where they make a shape a hand can hold.
///
/// Within one chord of one hand the fingers must run in pitch order and none may play two
/// keys. A reading that breaks that is a tracking error, and pinning it would force the
/// search into the fallback shape it keeps for impossible chords, so the whole chord is
/// left unpinned instead.
fn example(piece: &Piece, floor: f32) -> Option<Example> {
    let trusted = piece.trusted(floor);
    let (free, keys) = rebuild(&trusted);
    let mut pinned = free.clone();
    let index: HashMap<NoteId, usize> =
        pinned.notes.iter().enumerate().map(|(i, note)| (note.id, i)).collect();

    let mut chords: HashMap<(bool, i64), Vec<(u8, Finger, usize)>> = HashMap::new();
    for (i, note) in trusted.notes.iter().enumerate() {
        let Some(finger) = note.finger.and_then(Finger::from_number) else { continue };
        let Some(&at) = keys.get(i).and_then(|id| index.get(id)) else { continue };
        let right = note.hand == HandLabel::Right;
        chords.entry((right, pinned.notes[at].onset)).or_default().push((note.midi, finger, at));
    }
    let mut pins = 0;
    for ((right, _), mut chord) in chords {
        chord.sort_by_key(|(midi, _, _)| *midi);
        chord.dedup_by_key(|(midi, _, _)| *midi);
        let ordered = chord.windows(2).all(|pair| {
            let (a, b) = (pair[0].1.index(), pair[1].1.index());
            if right { b > a } else { b < a }
        });
        if !ordered {
            continue;
        }
        for (_, finger, at) in chord {
            pinned.notes[at].given_finger = Some(finger);
            pins += 1;
        }
    }
    (pins > 0).then_some(Example { free, pinned })
}

/// Every fact a path used, counted.
fn facts(path: &[Step]) -> HashMap<u64, f32> {
    let mut counts = HashMap::new();
    let mut scratch = Vec::with_capacity(16);
    for (i, step) in path.iter().enumerate() {
        scratch.clear();
        chord_features(step.hand, &step.notes, &step.fingers, &mut scratch);
        if let Some(previous) = i.checked_sub(1).map(|j| &path[j]).filter(|p| p.hand == step.hand) {
            step_features(
                step.hand,
                (&previous.notes, &previous.fingers),
                (&step.notes, &step.fingers),
                step.onset_seconds - previous.onset_seconds,
                &mut scratch,
            );
        }
        for fact in &scratch {
            *counts.entry(*fact).or_insert(0.0) += 1.0;
        }
    }
    counts
}

fn model_of(weights: &HashMap<u64, f32>) -> NgramPrior {
    let mut model = NgramPrior::new();
    model.learned = Some(Learned { weights: weights.clone() });
    model
}

/// Finger pieces on several threads, and put the measurements back together.
fn measure(pieces: &[Piece], options: &FingeringOptions, model: Option<&NgramPrior>, threads: usize) -> MatchRates {
    let chunk = pieces.len().div_ceil(threads.max(1)).max(1);
    let parts: Vec<_> = std::thread::scope(|scope| {
        let running: Vec<_> = pieces
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    per_piece(part, options, model.map(|m| m as &dyn on_fingering::FingeringPrior))
                })
            })
            .collect();
        running.into_iter().flat_map(|t| t.join().expect("a fingering thread panicked")).collect()
    });
    combine(&parts)
}

/// Train, keeping the weights from the pass that did best on the held-back pieces.
pub fn train(pieces: &[Piece], options: &FingeringOptions, config: &Config, mut say: impl FnMut(&str)) -> Report {
    on_fingering::set_inner_threads(false);
    let mut options = options.clone();
    // The learned weights come in through the model; the n-gram table it also carries is
    // empty and is not asked.
    options.prior_scale = 0.0;

    let split = split(pieces, config.held_out);
    let test: Vec<Piece> = split.test.iter().map(|p| p.trusted(config.floor)).collect();
    let examples: Vec<Example> = split.train.iter().filter_map(|p| example(p, config.floor)).collect();
    say(&format!(
        "{} pieces to learn from, {} held back to measure on.",
        examples.len(),
        test.len()
    ));

    let before = measure(&test, &options, None, config.threads);
    say(&format!("rules alone:   {before}"));

    let mut weights: HashMap<u64, f32> = HashMap::new();
    // The running sums that make the average (Daumé's trick): the average of the
    // weights after every batch is `weights - total / batches`.
    let mut total: HashMap<u64, f32> = HashMap::new();
    let mut batches = 1.0f32;
    let mut passes = Vec::new();
    let mut best = (before.general, None::<HashMap<u64, f32>>, 0usize);

    for pass in 0..config.epochs {
        // A fresh, repeatable order each pass.
        let mut order: Vec<usize> = (0..examples.len()).collect();
        order.sort_by_key(|i| crate::hash::fnv(&format!("{pass}:{i}")));
        if config.per_epoch > 0 {
            order.truncate(config.per_epoch);
        }
        for batch in order.chunks(config.threads.max(1) * 4) {
            let model = model_of(&weights);
            let deltas: Vec<HashMap<u64, f32>> = std::thread::scope(|scope| {
                let running: Vec<_> = batch
                    .chunks(4)
                    .map(|part| {
                        let (model, options, examples) = (&model, &options, &examples);
                        scope.spawn(move || {
                            let mut delta: HashMap<u64, f32> = HashMap::new();
                            for &i in part {
                                let free = finger_score_with_prior(&examples[i].free, options, Some(model));
                                let gold = finger_score_with_prior(&examples[i].pinned, options, Some(model));
                                if free.path == gold.path {
                                    continue;
                                }
                                for (fact, n) in facts(&free.path) {
                                    *delta.entry(fact).or_insert(0.0) += n;
                                }
                                for (fact, n) in facts(&gold.path) {
                                    *delta.entry(fact).or_insert(0.0) -= n;
                                }
                            }
                            delta
                        })
                    })
                    .collect();
                running.into_iter().map(|t| t.join().expect("a training thread panicked")).collect()
            });
            for delta in deltas {
                for (fact, n) in delta {
                    if n == 0.0 {
                        continue;
                    }
                    let step = config.rate * n;
                    *weights.entry(fact).or_insert(0.0) += step;
                    *total.entry(fact).or_insert(0.0) += batches * step;
                }
            }
            batches += 1.0;
        }
        let averaged: HashMap<u64, f32> = weights
            .iter()
            .map(|(fact, w)| (*fact, w - total.get(fact).copied().unwrap_or(0.0) / batches))
            .filter(|(_, w)| w.abs() > 1e-6)
            .collect();
        let rates = measure(&test, &options, Some(&model_of(&averaged)), config.threads);
        say(&format!("after pass {}: {rates}", pass + 1));
        if rates.general > best.0 {
            best = (rates.general, Some(averaged), pass + 1);
        }
        passes.push(rates);
    }

    let model = best.1.map(|w| model_of(&w)).unwrap_or_default();
    Report { model, before, passes, best: best.2, sizes: (examples.len(), test.len()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::FingeredNote;

    /// A slow line a pianist fingers in a way the rules would not choose, over and over.
    fn quirky(name: &str) -> Piece {
        let line = [(60u8, 2u8), (64, 4), (60, 2), (64, 4), (62, 3), (65, 5)];
        let notes = (0..8)
            .flat_map(|bar| {
                line.iter().enumerate().map(move |(i, (midi, finger))| FingeredNote {
                    midi: *midi,
                    onset: f64::from(bar * 6 + i as i32) * 0.5,
                    duration: 0.4,
                    hand: HandLabel::Right,
                    finger: Some(*finger),
                    confidence: Some(0.9),
                })
            })
            .collect();
        Piece { piece: name.into(), annotator: "video".into(), source: "test".into(), notes }
    }

    #[test]
    fn training_teaches_the_search_a_pianists_habit() {
        let pieces: Vec<Piece> = (0..12).map(|i| quirky(&format!("piece {i}"))).collect();
        let options = FingeringOptions { rule_set: on_fingering::RuleSet::Parncutt, ..Default::default() };
        let config = Config { epochs: 6, rate: 1.0, floor: 0.4, held_out: 0.3, per_epoch: 0, threads: 2 };
        let report = train(&pieces, &options, &config, |_| {});
        let learned = report.passes.iter().map(|r| r.general).fold(0.0, f32::max);
        assert!(
            learned > report.before.general + 0.2 && learned > 0.95,
            "rules alone {:.3}, best pass {:.3}",
            report.before.general,
            learned
        );
    }

    #[test]
    fn a_crossed_reading_is_not_pinned() {
        let mut piece = quirky("crossed");
        piece.notes.truncate(2);
        // Two notes struck together, the lower with the higher finger: not a shape a
        // right hand holds, so neither is pinned.
        piece.notes[1].onset = piece.notes[0].onset;
        piece.notes[0].finger = Some(4);
        piece.notes[1].finger = Some(2);
        assert!(example(&piece, 0.4).is_none());
    }
}
