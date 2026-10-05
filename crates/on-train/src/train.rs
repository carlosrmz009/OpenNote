use on_fingering::{FingeringOptions, NgramPrior};
use on_hand::Hand;

use crate::corpus::{by_piece, Piece};
use crate::hash::share;
use crate::eval::{evaluate, MatchRates};

const CANDIDATES: [f32; 7] = [0.0, 0.25, 0.5, 1.0, 1.5, 2.0, 3.0];

pub struct Split {
    pub train: Vec<Piece>,
    pub test: Vec<Piece>,
}

pub fn split(pieces: &[Piece], held_out: f32) -> Split {
    let held_out = held_out.clamp(0.0, 0.9);
    let mut train = Vec::new();
    let mut test = Vec::new();
    for (name, annotations) in by_piece(pieces) {
        let bucket = if share(name) < f64::from(held_out) {
            &mut test
        } else {
            &mut train
        };
        bucket.extend(annotations.into_iter().cloned());
    }
    if test.is_empty() && !train.is_empty() {
        tracing::warn!("not enough pieces to hold any back; the score below is on training data");
    }
    Split { train, test }
}

pub struct TrainReport {
    pub prior: NgramPrior,
    pub options: FingeringOptions,
    pub before: MatchRates,
    pub after: MatchRates,
    pub trained_on: usize,
    pub held_out: usize,
    pub tuned: bool,
}

impl TrainReport {
    pub fn improved(&self) -> bool {
        self.after.general > self.before.general + 1e-4
    }

    pub fn made_things_worse(&self) -> bool {
        self.after.general < self.before.general - 1e-4
    }

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
            let placements = piece.voice(hand);
            if placements.len() >= 2 {
                prior.observe(hand, &placements);
            }
        }
    }

    let scored = if test.is_empty() { &train } else { &test };

    let mut plain = base.clone();
    plain.prior_scale = 0.0;
    let before = evaluate(scored, &plain, None);

    let mut options = base.clone();
    if options.prior_scale == 0.0 {
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

fn fit_weights(
    train: &[Piece],
    base: &FingeringOptions,
    prior: &NgramPrior,
) -> FingeringOptions {
    let mut best = base.clone();
    let mut score = evaluate(train, &best, Some(prior)).general;

    for _ in 0..2 {
        for term in [Term::Prior, Term::Rules, Term::Patterns] {
            for candidate in CANDIDATES {
                let mut trial = best.clone();
                term.set(&mut trial, candidate);
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
                    confidence: None,
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

    #[test]
    fn training_on_an_empty_corpus_is_harmless() {
        let report = train(&[], &FingeringOptions::default(), 0.2, false);
        assert!(report.prior.is_empty());
        assert_eq!(report.trained_on, 0);
        assert!(!report.summary().is_empty());
    }

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
        assert!(report.after.general >= report.before.general - 0.01);
    }

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
