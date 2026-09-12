//! Fitting the model, and the weights that blend it with the rules.
//!
//! Two things are learned, and they are learned differently.
//!
//! The **prior** is counted, not optimised: every fingered note in the training set
//! is tallied into [`on_fingering::NgramPrior`] and that is the whole of it. There is
//! no learning rate, no stopping criterion and no run that can fail to converge.
//!
//! The **weights** — how far the prior, the published rules and the standard chord
//! shapes are trusted against each other — are three numbers, and they are fitted by
//! trying values and keeping what agreed best with the training set. Coordinate
//! descent over a short list of candidates is enough for three numbers, and it has
//! the property that matters here: every step is a measurement somebody can check,
//! not a gradient somebody has to believe.
//!
//! Both are measured on data that was held back. A model scored on what it was
//! trained on always looks better than it is, and somebody training for the first
//! time should not have to know that to avoid it.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use on_fingering::{FingeringOptions, NgramPrior};
use on_hand::Hand;

use crate::corpus::{by_piece, Piece};
use crate::eval::{evaluate, MatchRates};

/// The candidate weights tried for each term, in order.
///
/// Deliberately coarse. The corpora people will have are small, and picking between
/// 0.62 and 0.64 on a hundred pieces is fitting noise.
const CANDIDATES: [f32; 7] = [0.0, 0.25, 0.5, 1.0, 1.5, 2.0, 3.0];

/// A corpus divided into what is learned from and what is measured on.
pub struct Split {
    /// Pieces the model may learn from.
    pub train: Vec<Piece>,
    /// Pieces held back, which it never sees until it is scored.
    pub test: Vec<Piece>,
}

/// Divide a corpus, keeping every annotation of a piece on the same side.
///
/// Splitting by annotation rather than by piece would put one person's fingering of a
/// piece in the training set and another's in the test set, and the model would be
/// scored on music it had already learned. The division is by a hash of the piece
/// name, so it is the same every time the same corpus is used: two training runs are
/// comparable, which is the whole point of reporting a before and an after.
pub fn split(pieces: &[Piece], held_out: f32) -> Split {
    let held_out = held_out.clamp(0.0, 0.9);
    let mut train = Vec::new();
    let mut test = Vec::new();
    for (name, annotations) in by_piece(pieces) {
        let mut hasher = DefaultHasher::new();
        name.hash(&mut hasher);
        let fraction = (hasher.finish() % 10_000) as f32 / 10_000.0;
        let bucket = if fraction < held_out {
            &mut test
        } else {
            &mut train
        };
        bucket.extend(annotations.into_iter().cloned());
    }
    // A corpus too small to split is better trained on than not trained on. Say so
    // rather than silently producing a model scored against nothing.
    if test.is_empty() && !train.is_empty() {
        tracing::warn!("not enough pieces to hold any back; the score below is on training data");
    }
    Split { train, test }
}

/// What a training run did.
pub struct TrainReport {
    /// The model.
    pub prior: NgramPrior,
    /// The weights it should be used with.
    pub options: FingeringOptions,
    /// How the rules alone scored on the held-out pieces.
    pub before: MatchRates,
    /// How the rules and the model together scored on them.
    pub after: MatchRates,
    /// How many pieces were learned from, and how many held back.
    pub trained_on: usize,
    pub held_out: usize,
    /// Whether the weights were fitted as well as the counts.
    pub tuned: bool,
}

impl TrainReport {
    /// Whether the model actually helped.
    pub fn improved(&self) -> bool {
        self.after.general > self.before.general + 1e-4
    }

    /// Whether using the model would be a step backwards.
    ///
    /// Not simply the opposite of [`Self::improved`]. A model that changes nothing is
    /// perfectly fine to keep and go on training, and telling somebody to throw it
    /// away would be wrong; only one that scores *worse* is a problem.
    pub fn made_things_worse(&self) -> bool {
        self.after.general < self.before.general - 1e-4
    }

    /// The report, in sentences, for somebody who wants to know if this worked.
    pub fn summary(&self) -> Vec<String> {
        let mut lines = vec![
            format!(
                "Learned from {} pieces, held {} back.",
                self.trained_on, self.held_out
            ),
            format!("  rules only:  {}", self.before),
            format!("  with model:  {}", self.after),
        ];
        let change = (self.after.general - self.before.general) * 100.0;
        lines.push(match change {
            c if c > 0.5 => format!(
                "The model helped: agreement went up {c:.1} points. Keep it."
            ),
            c if c < -0.5 => format!(
                "The model made things worse by {:.1} points. That usually means too \
                 little data — add more pieces before relying on it.",
                -c
            ),
            _ => "The model made almost no difference. That is normal with a small \
                  corpus; add more pieces and train again."
                .to_string(),
        });
        if self.tuned {
            lines.push(format!(
                "Fitted weights: rules {:.2}, patterns {:.2}, model {:.2}.",
                self.options.rule_scale, self.options.pattern_scale, self.options.prior_scale
            ));
        }
        lines
    }
}

/// Train a model, and optionally fit the weights it is used with.
pub fn train(
    pieces: &[Piece],
    base: &FingeringOptions,
    held_out: f32,
    tune_weights: bool,
) -> TrainReport {
    let Split { train, test } = split(pieces, held_out);

    let mut prior = NgramPrior::new();
    for piece in &train {
        for hand in Hand::ALL {
            let placements = piece.placements(hand);
            if placements.len() >= 2 {
                prior.observe(hand, &placements);
            }
        }
    }

    // Scored on what was held back, unless there was nothing to hold back.
    let scored = if test.is_empty() { &train } else { &test };

    let mut plain = base.clone();
    plain.prior_scale = 0.0;
    let before = evaluate(scored, &plain, None);

    let mut options = base.clone();
    if options.prior_scale == 0.0 {
        // A default worth having: enough for the corpus to matter, not enough for it
        // to overrule a rule that says a fingering is unplayable.
        options.prior_scale = 1.0;
    }
    if tune_weights && !train.is_empty() {
        options = fit_weights(&train, &options, &prior);
    }
    let after = evaluate(scored, &options, Some(&prior));

    TrainReport {
        prior,
        options,
        before,
        after,
        trained_on: by_piece(&train).len(),
        held_out: by_piece(&test).len(),
        tuned: tune_weights,
    }
}

/// Try weights for each term in turn, keeping whatever agreed best.
///
/// Measured on the training set: the held-out pieces are for reporting, and using
/// them to choose the weights would make the report about how well the weights were
/// chosen rather than how well the model works.
fn fit_weights(
    train: &[Piece],
    base: &FingeringOptions,
    prior: &NgramPrior,
) -> FingeringOptions {
    let mut best = base.clone();
    let mut score = evaluate(train, &best, Some(prior)).general;

    // Two passes. The first finds roughly the right region for each term; the second
    // lets each one settle now that the others have moved. A third changes nothing on
    // any corpus this is likely to see.
    for _ in 0..2 {
        for term in [Term::Prior, Term::Rules, Term::Patterns] {
            for candidate in CANDIDATES {
                let mut trial = best.clone();
                term.set(&mut trial, candidate);
                // Turning off the rules entirely leaves nothing to stop a physically
                // impossible fingering, however well it agrees with the corpus.
                if trial.rule_scale == 0.0 {
                    continue;
                }
                let rate = evaluate(train, &trial, Some(prior)).general;
                if rate > score + 1e-4 {
                    score = rate;
                    best = trial;
                }
            }
        }
    }
    best
}

/// One of the weights being fitted.
#[derive(Clone, Copy)]
enum Term {
    Prior,
    Rules,
    Patterns,
}

impl Term {
    fn set(self, options: &mut FingeringOptions, value: f32) {
        match self {
            Term::Prior => options.prior_scale = value,
            Term::Rules => options.rule_scale = value,
            Term::Patterns => options.pattern_scale = value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{FingeredNote, HandLabel};

    fn piece(name: &str, annotator: &str, fingers: &[(u8, u8)]) -> Piece {
        Piece {
            piece: name.into(),
            annotator: annotator.into(),
            source: "test".into(),
            notes: fingers
                .iter()
                .enumerate()
                .map(|(i, (midi, finger))| FingeredNote {
                    duration: crate::corpus::ASSUMED_DURATION,
                    midi: *midi,
                    onset: i as f64 * 0.25,
                    hand: HandLabel::Right,
                    finger: Some(*finger),
                })
                .collect(),
        }
    }

    fn scale(name: &str) -> Vec<Piece> {
        let notes = [
            (60, 1),
            (62, 2),
            (64, 3),
            (65, 1),
            (67, 2),
            (69, 3),
            (71, 4),
            (72, 5),
        ];
        vec![piece(name, "1", &notes), piece(name, "2", &notes)]
    }

    /// Every annotation of a piece has to land on the same side of the split, or the
    /// model is scored on music it has already seen.
    #[test]
    fn annotations_of_one_piece_are_never_separated() {
        let mut pieces = Vec::new();
        for index in 0..40 {
            pieces.extend(scale(&format!("piece-{index}")));
        }
        let split = split(&pieces, 0.3);
        for side in [&split.train, &split.test] {
            let names: std::collections::BTreeSet<&str> =
                side.iter().map(|p| p.piece.as_str()).collect();
            for name in names {
                let here = side.iter().filter(|p| p.piece == name).count();
                let total = pieces.iter().filter(|p| p.piece == name).count();
                assert_eq!(here, total, "{name} was split across the two sides");
            }
        }
        assert!(!split.train.is_empty() && !split.test.is_empty());
    }

    /// The same corpus has to give the same split every time, or a before-and-after
    /// comparison means nothing.
    #[test]
    fn the_split_is_the_same_every_time() {
        let mut pieces = Vec::new();
        for index in 0..30 {
            pieces.extend(scale(&format!("piece-{index}")));
        }
        let first: Vec<String> = split(&pieces, 0.25)
            .test
            .iter()
            .map(|p| p.piece.clone())
            .collect();
        let second: Vec<String> = split(&pieces, 0.25)
            .test
            .iter()
            .map(|p| p.piece.clone())
            .collect();
        assert_eq!(first, second);
    }

    /// Training on nothing is not a crash, and produces an empty model.
    #[test]
    fn training_on_an_empty_corpus_is_harmless() {
        let report = train(&[], &FingeringOptions::default(), 0.2, false);
        assert!(report.prior.is_empty());
        assert_eq!(report.trained_on, 0);
        assert!(!report.summary().is_empty());
    }

    /// A model trained on scales should learn something from them.
    #[test]
    fn training_counts_what_it_was_shown() {
        let mut pieces = Vec::new();
        for index in 0..12 {
            pieces.extend(scale(&format!("piece-{index}")));
        }
        let report = train(&pieces, &FingeringOptions::default(), 0.25, false);
        assert!(!report.prior.is_empty());
        assert!(report.trained_on > 0);
        assert!(report.held_out > 0);
        // These are textbook scales, so the rules already agree with them and the
        // model cannot make that worse.
        assert!(report.after.general >= report.before.general - 0.01);
    }

    /// Fitting the weights must never turn the rules off; nothing else stops the
    /// search choosing a fingering no hand could play.
    #[test]
    fn fitting_the_weights_keeps_the_rules_on() {
        let mut pieces = Vec::new();
        for index in 0..8 {
            pieces.extend(scale(&format!("piece-{index}")));
        }
        let report = train(&pieces, &FingeringOptions::default(), 0.25, true);
        assert!(report.options.rule_scale > 0.0);
        assert!(report.tuned);
    }
}
