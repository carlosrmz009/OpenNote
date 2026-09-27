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

use on_fingering::learned::{chord_features, step_features, trigram_features, Learned, Style, WEIGHT_LIMIT};
use on_fingering::{finger_score_with_prior, FingeringOptions, NgramPrior, Playability, Step};
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
    /// Who played each piece, where that is known: a channel, a pianist. Pieces are then
    /// held back a whole group at a time, so what is measured is how well the model does
    /// on players it has never seen, not on more pieces by the ones it learned from.
    /// Pieces without a group are held back on their own.
    pub groups: HashMap<String, String>,
    /// Hold back exactly these groups, rather than a share of them chosen by hash. With
    /// few groups a hash puts them on either side by luck; naming them is the honest way
    /// to ask "how does it do on this pianist, having never seen them?".
    pub test_groups: Vec<String>,
    /// Learn one model for everything — shared weights only, no style on top — rather
    /// than a style per source. The sources still decide which pieces are held back and
    /// how the results are reported; they no longer decide how anything is fingered.
    pub unified: bool,
}

/// What a training run did.
pub struct Report {
    /// The model: an empty n-gram table carrying the learned weights, both styles.
    pub model: NgramPrior,
    /// The held-back pieces of each style fingered by the rules alone.
    pub before: Vec<(Style, MatchRates)>,
    /// The same after each pass, with that pass's averaged weights.
    pub passes: Vec<Vec<(Style, MatchRates)>>,
    /// Which pass was kept.
    pub best: usize,
    /// How many training and held-back pieces there were.
    pub sizes: (usize, usize),
    /// For the pass kept, how much better than the rules alone each style is on its
    /// held-back pieces, paired: the pieces resampled together (see
    /// [`crate::eval::paired`]). This, not the pass-by-pass figure, says whether a gain
    /// is real.
    pub gains: Vec<(Style, crate::eval::Interval)>,
    /// For the pass kept, what each source's held-back pieces cost a hand: fingered by
    /// the rules alone, then with the model. Comfort is judged alongside agreement,
    /// never after it.
    pub comfort: Vec<(Style, Playability, Playability)>,
}

/// The style a piece is fingered in: its source's, or one for everything.
fn used(style: Style, unified: bool) -> Style {
    if unified {
        Style::Unified
    } else {
        style
    }
}

/// What a set of pieces costs a hand, fingered with or without a model: position
/// changes, travel, stretch, and transitions no hand could make in time.
pub fn comfort(
    pieces: &[Piece],
    options: &FingeringOptions,
    model: Option<&NgramPrior>,
    threads: usize,
) -> Playability {
    let chunk = pieces.len().div_ceil(threads.max(1)).max(1);
    let parts: Vec<Playability> = std::thread::scope(|scope| {
        let running: Vec<_> = pieces
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    let mut total = Playability::default();
                    for piece in part {
                        let (score, _) = rebuild(piece);
                        let solution = finger_score_with_prior(
                            &score,
                            options,
                            model.map(|m| m as &dyn on_fingering::FingeringPrior),
                        );
                        total.add(on_fingering::measure_solution(&score, &solution, options.span_model.table()));
                    }
                    total
                })
            })
            .collect();
        running.into_iter().map(|t| t.join().expect("a comfort thread panicked")).collect()
    });
    let mut total = Playability::default();
    for part in parts {
        total.add(part);
    }
    total
}

/// One training piece, ready to finger both ways.
struct Example {
    free: Score,
    pinned: Score,
    style: Style,
}

/// Which style a piece teaches: anything read off a performance video the performance
/// one, and everything written down — an edition's printed fingering — the classical one.
pub fn style_of(piece: &Piece) -> Style {
    if piece.annotator == "video" {
        Style::Performance
    } else {
        Style::Classical
    }
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
    (pins > 0).then_some(Example { free, pinned, style: style_of(piece) })
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
            if let Some(before) = i.checked_sub(2).map(|j| &path[j]).filter(|p| p.hand == step.hand) {
                trigram_features(
                    step.hand,
                    (&before.notes, &before.fingers),
                    (&previous.notes, &previous.fingers),
                    (&step.notes, &step.fingers),
                    &mut scratch,
                );
            }
        }
        for fact in &scratch {
            *counts.entry(*fact).or_insert(0.0) += 1.0;
        }
    }
    counts
}

fn model_of(weights: &HashMap<u64, f32>, style: Style) -> NgramPrior {
    let mut model = NgramPrior::new();
    model.learned = Some(Learned { weights: weights.clone(), style });
    model
}

const STYLES: [Style; 2] = [Style::Performance, Style::Classical];

/// Each style's held-back pieces, measured in that style.
fn measure_styles(
    test: &[(Style, Vec<Piece>)],
    options: &FingeringOptions,
    weights: Option<&HashMap<u64, f32>>,
    threads: usize,
    unified: bool,
) -> Vec<(Style, MatchRates)> {
    test.iter()
        .map(|(style, pieces)| {
            let model = weights.map(|w| model_of(w, used(*style, unified)));
            (*style, measure(pieces, options, model.as_ref(), threads))
        })
        .collect()
}

/// The figure a pass is chosen by: each style's agreement, averaged, so neither can be
/// bought at the other's expense unnoticed.
fn overall(rates: &[(Style, MatchRates)]) -> f32 {
    rates.iter().map(|(_, r)| r.general).sum::<f32>() / rates.len().max(1) as f32
}

fn describe_rates(rates: &[(Style, MatchRates)]) -> String {
    rates.iter().map(|(style, r)| format!("{style:?} {:.1}%", r.general * 100.0)).collect::<Vec<_>>().join(", ")
}

/// Finger pieces on several threads, and put the measurements back together.
fn measure(pieces: &[Piece], options: &FingeringOptions, model: Option<&NgramPrior>, threads: usize) -> MatchRates {
    combine(&measure_pieces(pieces, options, model, threads))
}

/// The same, each piece's measurement kept apart, in the order the pieces were given.
fn measure_pieces(
    pieces: &[Piece],
    options: &FingeringOptions,
    model: Option<&NgramPrior>,
    threads: usize,
) -> Vec<crate::eval::PieceRates> {
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
    parts
}

/// Divide pieces a whole group at a time, by a hash of the group, so it is the same
/// division every run.
fn split_by_group(
    pieces: &[Piece],
    held_out: f32,
    groups: &HashMap<String, String>,
    test_groups: &[String],
) -> crate::Split {
    let mut split = crate::Split { train: Vec::new(), test: Vec::new() };
    for piece in pieces {
        // A piece whose player is known goes with its group; one whose player is not — a
        // written fingering, say — is held back on its own, by the same hash as ever.
        let chosen = match groups.get(&piece.piece) {
            Some(group) if !test_groups.is_empty() => test_groups.iter().any(|g| g == group),
            Some(group) => crate::hash::share(group) < f64::from(held_out),
            None => crate::hash::share(&piece.piece) < f64::from(held_out),
        };
        if chosen {
            split.test.push(piece.clone());
        } else {
            split.train.push(piece.clone());
        }
    }
    split
}

/// A trained model's gain over the rules alone, style by style, on exactly the pieces a
/// training run with this configuration would have held back.
pub fn measure_model(
    pieces: &[Piece],
    options: &FingeringOptions,
    config: &Config,
    weights: &HashMap<u64, f32>,
) -> Vec<(Style, MatchRates, MatchRates, crate::eval::Interval, Playability, Playability)> {
    on_fingering::set_inner_threads(false);
    let mut options = options.clone();
    options.prior_scale = 0.0;
    let split = if config.groups.is_empty() {
        split(pieces, config.held_out)
    } else {
        split_by_group(pieces, config.held_out, &config.groups, &config.test_groups)
    };
    STYLES
        .iter()
        .filter_map(|style| {
            let held: Vec<Piece> = split
                .test
                .iter()
                .filter(|p| style_of(p) == *style)
                .map(|p| p.trusted(config.floor))
                .collect();
            if held.is_empty() {
                return None;
            }
            let model = model_of(weights, used(*style, config.unified));
            let ours = measure_pieces(&held, &options, Some(&model), config.threads);
            let theirs = measure_pieces(&held, &options, None, config.threads);
            let hand_before = comfort(&held, &options, None, config.threads);
            let hand_after = comfort(&held, &options, Some(&model), config.threads);
            Some((
                *style,
                combine(&theirs),
                combine(&ours),
                crate::eval::paired(&ours, &theirs, 1000),
                hand_before,
                hand_after,
            ))
        })
        .collect()
}

/// Train, keeping the weights from the pass that did best on the held-back pieces.
pub fn train(pieces: &[Piece], options: &FingeringOptions, config: &Config, mut say: impl FnMut(&str)) -> Report {
    on_fingering::set_inner_threads(false);
    let mut options = options.clone();
    // The learned weights come in through the model; the n-gram table it also carries is
    // empty and is not asked.
    options.prior_scale = 0.0;

    let split = if config.groups.is_empty() {
        split(pieces, config.held_out)
    } else {
        split_by_group(pieces, config.held_out, &config.groups, &config.test_groups)
    };
    let test: Vec<(Style, Vec<Piece>)> = STYLES
        .iter()
        .map(|style| {
            let pieces: Vec<Piece> = split
                .test
                .iter()
                .filter(|p| style_of(p) == *style)
                .map(|p| p.trusted(config.floor))
                .collect();
            (*style, pieces)
        })
        .filter(|(_, pieces)| !pieces.is_empty())
        .collect();
    let examples: Vec<Example> = split.train.iter().filter_map(|p| example(p, config.floor)).collect();
    for style in STYLES {
        say(&format!(
            "{style:?}: {} pieces to learn from, {} held back to measure on.",
            examples.iter().filter(|e| e.style == style).count(),
            test.iter().find(|(s, _)| *s == style).map_or(0, |(_, p)| p.len())
        ));
    }

    let before = measure_styles(&test, &options, None, config.threads, config.unified);
    for (style, rates) in &before {
        say(&format!("rules alone, {style:?}: {rates}"));
    }

    let mut weights: HashMap<u64, f32> = HashMap::new();
    // The running sums that make the average (Daumé's trick): the average of the
    // weights after every batch is `weights - total / batches`.
    let mut total: HashMap<u64, f32> = HashMap::new();
    let mut batches = 1.0f32;
    let mut passes = Vec::new();
    let mut best = (overall(&before), None::<HashMap<u64, f32>>, 0usize);

    for pass in 0..config.epochs {
        // A fresh, repeatable order each pass.
        let mut order: Vec<usize> = (0..examples.len()).collect();
        order.sort_by_key(|i| crate::hash::fnv(&format!("{pass}:{i}")));
        if config.per_epoch > 0 {
            order.truncate(config.per_epoch);
        }
        for batch in order.chunks(config.threads.max(1) * 4) {
            let models: Vec<(Style, NgramPrior)> =
                STYLES.iter().map(|s| (*s, model_of(&weights, used(*s, config.unified)))).collect();
            let unified = config.unified;
            let deltas: Vec<HashMap<u64, f32>> = std::thread::scope(|scope| {
                let running: Vec<_> = batch
                    .chunks(4)
                    .map(|part| {
                        let (models, options, examples) = (&models, &options, &examples);
                        scope.spawn(move || {
                            let mut delta: HashMap<u64, f32> = HashMap::new();
                            for &i in part {
                                let example = &examples[i];
                                let model = &models.iter().find(|(s, _)| *s == example.style).expect("every style has a model").1;
                                let free = finger_score_with_prior(&example.free, options, Some(model));
                                let gold = finger_score_with_prior(&example.pinned, options, Some(model));
                                // A written fingering that forces a stretch the hand cannot
                                // make is an engraving slip, not a lesson.
                                if gold.explanations.iter().any(|e| !e.reachable) {
                                    continue;
                                }
                                // Each piece moves the weights by the same amount however
                                // long it is: by its disagreements, one unit shared among
                                // them, so a long piece full of mistakes cannot outvote the
                                // rest of the batch.
                                let wrong = free.path.iter().zip(&gold.path).filter(|(a, b)| a != b).count();
                                if wrong == 0 {
                                    continue;
                                }
                                let share = 1.0 / wrong as f32;
                                let mut piece: HashMap<u64, f32> = HashMap::new();
                                for (fact, n) in facts(&free.path) {
                                    *piece.entry(fact).or_insert(0.0) += n;
                                }
                                for (fact, n) in facts(&gold.path) {
                                    *piece.entry(fact).or_insert(0.0) -= n;
                                }
                                // Every fact is learned shared, and — unless there is one
                                // model for everything — for this style as well.
                                for (fact, n) in piece {
                                    if n != 0.0 {
                                        *delta.entry(fact).or_insert(0.0) += n * share;
                                        if !unified {
                                            *delta.entry(example.style.tag(fact)).or_insert(0.0) += n * share;
                                        }
                                    }
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
                    let weight = weights.entry(fact).or_insert(0.0);
                    let moved = (*weight + config.rate * n).clamp(-WEIGHT_LIMIT, WEIGHT_LIMIT);
                    let step = moved - *weight;
                    *weight = moved;
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
        let rates = measure_styles(&test, &options, Some(&averaged), config.threads, config.unified);
        say(&format!("after pass {}: {}", pass + 1, describe_rates(&rates)));
        if overall(&rates) > best.0 {
            best = (overall(&rates), Some(averaged), pass + 1);
        }
        passes.push(rates);
    }

    let mut gains = Vec::new();
    let mut hands = Vec::new();
    if let Some(weights) = &best.1 {
        for (style, pieces) in &test {
            let model = model_of(weights, used(*style, config.unified));
            let ours = measure_pieces(pieces, &options, Some(&model), config.threads);
            let theirs = measure_pieces(pieces, &options, None, config.threads);
            gains.push((*style, crate::eval::paired(&ours, &theirs, 1000)));
            hands.push((
                *style,
                comfort(pieces, &options, None, config.threads),
                comfort(pieces, &options, Some(&model), config.threads),
            ));
        }
    }
    let default_style = if config.unified { Style::Unified } else { Style::Performance };
    let model = best.1.map(|w| model_of(&w, default_style)).unwrap_or_default();
    Report {
        model,
        before,
        passes,
        best: best.2,
        sizes: (examples.len(), test.iter().map(|(_, p)| p.len()).sum()),
        gains,
        comfort: hands,
    }
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
        let config = Config { epochs: 6, rate: 1.0, floor: 0.4, held_out: 0.3, per_epoch: 0, threads: 2, groups: HashMap::new(), test_groups: Vec::new(), unified: false };
        let report = train(&pieces, &options, &config, |_| {});
        let learned = report.passes.iter().map(|r| overall(r)).fold(0.0, f32::max);
        let before = overall(&report.before);
        assert!(
            learned > before + 0.2 && learned > 0.95,
            "rules alone {before:.3}, best pass {learned:.3}"
        );
    }

    #[test]
    fn two_styles_keep_opposite_habits_apart() {
        // The same line, fingered one way in performances and another in editions.
        let habit = |name: String, annotator: &str, fingers: [u8; 2]| {
            let mut piece = quirky(&name);
            piece.annotator = annotator.into();
            for note in &mut piece.notes {
                note.finger = Some(if note.midi == 60 { fingers[0] } else if note.midi == 64 { fingers[1] } else { note.finger.unwrap_or(1) });
            }
            piece
        };
        let mut pieces: Vec<Piece> = (0..12).map(|i| habit(format!("seen {i}"), "video", [2, 4])).collect();
        pieces.extend((0..12).map(|i| habit(format!("written {i}"), "edition", [1, 3])));
        let options = FingeringOptions { rule_set: on_fingering::RuleSet::Parncutt, ..Default::default() };
        let config = Config { epochs: 8, rate: 1.0, floor: 0.4, held_out: 0.3, per_epoch: 0, threads: 2, groups: HashMap::new(), test_groups: Vec::new(), unified: false };
        let report = train(&pieces, &options, &config, |_| {});
        let best = &report.passes[report.best.max(1) - 1];
        for (style, rates) in best {
            assert!(rates.general > 0.9, "{style:?} only reached {:.3}: {}", rates.general, describe_rates(best));
        }
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
