//! OpenNote's own benchmark: a manifest of frozen tiers, measured into one scorecard.
//!
//! A tier is a set of fingered pieces with one kind of truth â€” pianists read off video,
//! editors' printed fingerings, several experts on the same music â€” and each is measured
//! the same way: the rules alone and the model on exactly the same notes, every rate of
//! [`crate::eval`], the gain paired piece by piece (whole groups resampled where the
//! pieces of one group are not independent), how comfortable each fingering is to play,
//! and, where a piece has several annotations, how the pianists agree among themselves,
//! which is the ceiling every other number should be read against.
//!
//! The data are built and frozen elsewhere (out/accuracy/make_bench*.py); this only
//! reads them, and the manifest says which hash each file must have, so a benchmark that
//! has quietly changed refuses to run rather than report a number about something else.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use on_fingering::{FingeringOptions, FingeringPrior, Playability};
use serde::{Deserialize, Serialize};

use crate::corpus::{by_piece, Corpus, Piece};
use crate::eval::{combine, human_reference, paired_by, rate_against, Interval, MatchRates, PieceRates};

/// A benchmark: its tiers, and the hash each of their files must have.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub tiers: Vec<TierSpec>,
    /// `piece<TAB>group` lines saying which pianist or score each piece belongs to.
    #[serde(default)]
    pub groups: Option<PathBuf>,
    /// FNV-1a of each data file, by path. A file that no longer matches is refused.
    #[serde(default)]
    pub hashes: BTreeMap<PathBuf, String>,
}

/// One tier of a benchmark.
#[derive(Debug, Clone, Deserialize)]
pub struct TierSpec {
    pub id: String,
    pub title: String,
    /// Corpus directories (`<dir>/normalised/*.jsonl`).
    pub corpora: Vec<PathBuf>,
    /// Readings below this confidence are not scored (video tiers).
    #[serde(default)]
    pub min_confidence: Option<f32>,
    /// Resample whole groups rather than single pieces when pairing.
    #[serde(default)]
    pub cluster: bool,
    /// Measure without the bonus the solver gives the fingerings it was taught (its scale
    /// tables): on scales, with the bonus the engine is only being asked whether it
    /// remembers its own answer, which is a regression check and not a measurement.
    #[serde(default)]
    pub without_taught: bool,
    /// Pieces of this tier whose name starts with one of these must be fingered exactly
    /// as written, or the benchmark gives no headline score at all.
    #[serde(default)]
    pub must_pass: Vec<String>,
    /// Kept back for releases: measured only when asked (`--sealed`), so that it is not
    /// slowly fitted by being looked at after every change.
    #[serde(default)]
    pub sealed: bool,
    /// Run the consistency probes on this tier (see [`probe`]).
    #[serde(default)]
    pub probes: bool,
}

/// Everything measured, for a model and the rules it is built on.
#[derive(Debug, Clone, Serialize)]
pub struct Scorecard {
    pub bench: String,
    pub model: Option<String>,
    pub tiers: Vec<TierCard>,
    pub headline: Headline,
}

/// The OpenNote Score: one number, never shown without what it is made of.
///
/// `100 * G * prod(max(c, 0.01)^w)`. G is 0, and the score withheld with the reason, if
/// a must-pass convention is fingered wrongly or anything is unplayable. Each agreement
/// component is the share of the way from random fingers to perfect agreement,
/// `(m - b)/(1 - b)`, with b the agreement of random fingers (one in five a note, one in
/// 625 for four notes in a row), so a better model can always score higher, past human
/// level. Comfort is, for each kind of awkwardness, the people's own rate over the
/// model's, capped at one, combined by a geometric mean: as comfortable as a pianist, or
/// more, is full marks, and no one kind of awkwardness can be traded for another unseen.
/// The weights are fixed with the benchmark's version and renormalised over the parts
/// that exist; until the expert tier does, the several-pianists tier stands in for it
/// and the score is labelled pre-release.
#[derive(Debug, Clone, Serialize)]
pub struct Headline {
    pub score: Option<f32>,
    pub gated: Option<String>,
    pub label: String,
    /// (component, value 0..1, weight used).
    pub components: Vec<(String, f32, f32)>,
}

const RANDOM_NOTE: f32 = 0.2;
const RANDOM_RUN: f32 = 0.0016;

impl Scorecard {
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TierCard {
    pub id: String,
    pub title: String,
    pub pieces: usize,
    pub groups: usize,
    pub notes: usize,
    pub rules: Rates,
    pub model: Option<Rates>,
    /// The model's gain over the rules, paired, in points.
    pub gain: Option<Gains>,
    /// The pianists against one another, where pieces have several annotations.
    pub human: Option<Rates>,
    pub comfort_rules: Comfort,
    pub comfort_model: Option<Comfort>,
    /// The comfort of the people's own fingerings of the same pieces.
    pub comfort_human: Comfort,
    /// Must-pass pieces fingered otherwise than as written.
    pub gate_failures: Vec<String>,
    /// The consistency probes, where the tier runs them.
    pub probes: Option<Probes>,
    /// How long the judged system took, in milliseconds per thousand notes.
    pub ms_per_1000_notes: f32,
    /// The pieces the system being judged (the model, or the rules alone without one)
    /// agreed with least, worst first: where to look.
    pub weakest: Vec<(String, f32)>,
}

/// Agreement, in percent.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Rates {
    pub general: f32,
    pub highest: f32,
    pub soft: f32,
    pub recombined: f32,
    pub ngram: f32,
}

impl From<MatchRates> for Rates {
    fn from(r: MatchRates) -> Self {
        Rates {
            general: r.general * 100.0,
            highest: r.highest * 100.0,
            soft: r.soft * 100.0,
            recombined: r.recombined * 100.0,
            ngram: r.ngram * 100.0,
        }
    }
}

/// A difference in points, with its 95% interval.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Span {
    pub point: f32,
    pub low: f32,
    pub high: f32,
    /// The interval is clear of zero.
    pub real: bool,
}

impl From<Interval> for Span {
    fn from(i: Interval) -> Self {
        Span { point: i.point * 100.0, low: i.low * 100.0, high: i.high * 100.0, real: i.low > 0.0 || i.high < 0.0 }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Gains {
    pub general: Span,
    pub ngram: Span,
    pub recombined: Span,
}

/// How hard a fingering is on the hand: what [`Playability`] measures, as rates.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Comfort {
    /// Transitions no hand could make in time, in percent.
    pub unplayable: f32,
    /// How many there were.
    pub unplayable_count: usize,
    /// Hand position changes per 100 transitions.
    pub hand_moves: f32,
    /// Millimetres travelled per transition.
    pub travel_mm: f32,
    /// Finger pairs held past a comfortable span, in percent.
    pub stretched: f32,
    /// Thumb passes under, fingers passed over the thumb, and crossings without the
    /// thumb, per 100 transitions.
    pub thumb_unders: f32,
    pub finger_overs: f32,
    pub thumbless: f32,
    /// Semitones per finger between successive notes, and within chords.
    pub step_spread: f32,
    pub chord_spread: f32,
    /// Notes the fourth or fifth finger played on a black key, in percent.
    pub weak_on_black: f32,
}

impl From<&Playability> for Comfort {
    fn from(p: &Playability) -> Self {
        Comfort {
            unplayable: p.unplayable_rate() * 100.0,
            unplayable_count: p.unplayable,
            hand_moves: p.change_rate() * 100.0,
            travel_mm: (p.travel_mm / p.transitions.max(1) as f64) as f32,
            stretched: p.stretched_rate() * 100.0,
            thumb_unders: p.thumb_under_rate() * 100.0,
            finger_overs: p.finger_over_rate() * 100.0,
            thumbless: p.thumbless_rate() * 100.0,
            step_spread: p.step_spread(),
            chord_spread: p.chord_spread(),
            weak_on_black: p.weak_on_black_rate() * 100.0,
        }
    }
}

/// How hard the people's own fingerings were on the hand, measured exactly as the
/// engine's are: what "as comfortable as a pianist" has to be read against.
fn human_comfort(pieces: &[Piece], options: &FingeringOptions) -> Playability {
    use on_fingering::playability::{measure, Chord, CHORD_SECONDS};
    let mut total = Playability::default();
    for piece in pieces {
        for (label, hand) in [(crate::corpus::HandLabel::Left, on_hand::Hand::Left), (crate::corpus::HandLabel::Right, on_hand::Hand::Right)] {
            let mut notes: Vec<(f64, on_fingering::Placement)> = piece
                .notes
                .iter()
                .filter(|n| n.hand == label)
                .filter_map(|n| Some((n.onset, on_fingering::Placement::new(n.midi, on_hand::Finger::from_number(n.finger?)?))))
                .collect();
            notes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.midi.cmp(&b.1.midi)));
            let mut chords: Vec<Chord> = Vec::new();
            let mut started = f64::NEG_INFINITY;
            for (onset, placement) in notes {
                match chords.last_mut() {
                    Some(last) if onset - started <= CHORD_SECONDS => last.notes.push(placement),
                    _ => {
                        started = onset;
                        chords.push(Chord::single(onset, placement));
                    }
                }
            }
            total.add(measure(hand, options.span_model.table(), &chords));
        }
    }
    total
}

/// Whether a fingering stays the same when nothing that should change it does. Shares of
/// notes fingered identically, 0..1.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Probes {
    /// The piece an octave higher or lower (kept in the middle of the keyboard).
    pub transposed: f32,
    /// Every onset moved by up to 10 ms, far inside a chord's 30 ms.
    pub jittered: f32,
    /// Pieces probed.
    pub pieces: usize,
}

impl Probes {
    pub fn score(&self) -> f32 {
        (self.transposed + self.jittered) / 2.0
    }
}

/// How many pieces of a tier the probes take, the first by name: a fixed sample.
const PROBE_PIECES: usize = 100;

/// The consistency probes. An octave changes nothing a hand feels â€” the same keys, the
/// same black and white â€” so the fingering should not move; neither should a timing
/// wobble too small to matter. A fingering that does is responding to noise.
pub fn probe(pieces: &[Piece], options: &FingeringOptions, prior: Option<&dyn FingeringPrior>) -> Probes {
    let mut firsts: Vec<&Piece> = by_piece(pieces).into_values().map(|a| a[0]).collect();
    firsts.truncate(PROBE_PIECES);
    let (mut same_t, mut all_t, mut same_j, mut all_j) = (0usize, 0usize, 0usize, 0usize);
    let mut rng: u64 = 0x9e37_79b9_7f4a_7c15;
    for piece in &firsts {
        let Some(plain) = crate::eval::solve(piece, options, prior) else { continue };
        let low = piece.notes.iter().map(|n| n.midi).min().unwrap_or(60);
        let high = piece.notes.iter().map(|n| n.midi).max().unwrap_or(60);
        let shift: i16 = if high <= 84 { 12 } else if low >= 36 { -12 } else { 0 };
        if shift != 0 {
            let mut moved = (*piece).clone();
            for n in &mut moved.notes {
                n.midi = (i16::from(n.midi) + shift) as u8;
            }
            if let Some(t) = crate::eval::solve(&moved, options, prior) {
                for (a, b) in plain.iter().zip(&t) {
                    if a.is_some() {
                        all_t += 1;
                        same_t += usize::from(a == b);
                    }
                }
            }
        }
        let mut wobbled = (*piece).clone();
        // Whole chords move together, so a chord stays a chord; only the time between
        // them changes, by up to 10 ms.
        let mut last_onset = f64::NEG_INFINITY;
        let mut offset = 0.0;
        for n in &mut wobbled.notes {
            if (n.onset - last_onset).abs() > on_fingering::playability::CHORD_SECONDS {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                offset = ((rng >> 33) as f64 / f64::from(u32::MAX) - 0.5) * 0.020;
                last_onset = n.onset;
            }
            n.onset = (n.onset + offset).max(0.0);
        }
        if let Some(j) = crate::eval::solve(&wobbled, options, prior) {
            for (a, b) in plain.iter().zip(&j) {
                if a.is_some() {
                    all_j += 1;
                    same_j += usize::from(a == b);
                }
            }
        }
    }
    let share = |same: usize, all: usize| if all == 0 { 1.0 } else { same as f32 / all as f32 };
    Probes { transposed: share(same_t, all_t), jittered: share(same_j, all_j), pieces: firsts.len() }
}

/// FNV-1a of a file's bytes, as hex: what the manifest pins.
pub fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(format!("{:016x}", fnv(&bytes)))
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// A passage where the model and another fingering disagree, for a person to judge
/// blind which is more comfortable (tools/training_suite.py's judge screen).
#[derive(Debug, Clone, Serialize)]
pub struct Excerpt {
    pub tier: String,
    pub piece: String,
    /// The passage's notes, fingers left out.
    pub notes: Vec<crate::corpus::FingeredNote>,
    /// "model" and either "rules" or "people", each a finger per note.
    pub fingerings: BTreeMap<String, Vec<Option<u8>>>,
}

/// The judge screen's file: which model, and its passages.
pub fn excerpts_json(model: Option<String>, excerpts: &[Excerpt]) -> Result<String> {
    Ok(serde_json::to_string(&serde_json::json!({ "model": model, "excerpts": excerpts }))?)
}

/// Notes in a judged passage: a few bars of a casual piece.
const EXCERPT_NOTES: usize = 32;
/// Fewest notes the two fingerings must differ on for a passage to be worth judging.
const EXCERPT_DIFFERENCES: usize = 3;

/// Passages to judge from every dev tier but the conventions: per tier, up to `count`
/// against the rules and `count` against what the people played, one of each per piece
/// at most, each the window where the two disagree most. Pieces are taken in an order
/// fixed by their names' hash, so the same bench always offers the same passages.
pub fn excerpts(manifest: &Manifest, options: &FingeringOptions, prior: &dyn FingeringPrior, count: usize) -> Result<Vec<Excerpt>> {
    let mut rules_only = options.clone();
    rules_only.prior_scale = 0.0;
    let mut out = Vec::new();
    for spec in manifest.tiers.iter().filter(|t| !t.sealed && !t.id.starts_with("T0")) {
        let mut pieces = Vec::new();
        for dir in &spec.corpora {
            pieces.extend(Corpus::at(dir)?.load()?);
        }
        if let Some(floor) = spec.min_confidence {
            pieces = pieces.iter().map(|p| p.trusted(floor)).collect();
        }
        let mut order: Vec<Vec<&Piece>> = by_piece(&pieces).into_values().collect();
        order.sort_by_key(|a| fnv(a[0].piece.as_bytes()));
        let (mut against_rules, mut against_people) = (0, 0);
        for annotations in order {
            if against_rules >= count && against_people >= count {
                break;
            }
            let piece = annotations[0];
            let Some(model) = crate::eval::solve(piece, options, Some(prior)) else { continue };
            let people: Vec<Option<u8>> = piece.notes.iter().map(|n| n.finger).collect();
            let mut others = Vec::new();
            if against_rules < count {
                if let Some(rules) = crate::eval::solve(piece, &rules_only, None) {
                    others.push(("rules", rules));
                }
            }
            if against_people < count {
                others.push(("people", people));
            }
            for (label, other) in others {
                // The window with the most disagreement, among those the other fingers almost whole.
                let best = (0..piece.notes.len().saturating_sub(EXCERPT_NOTES - 1))
                    .step_by(EXCERPT_NOTES / 4)
                    .filter(|&i| other[i..i + EXCERPT_NOTES].iter().filter(|f| f.is_some()).count() >= EXCERPT_NOTES * 9 / 10)
                    .map(|i| (model[i..i + EXCERPT_NOTES].iter().zip(&other[i..i + EXCERPT_NOTES]).filter(|(a, b)| b.is_some() && a != b).count(), i))
                    .max();
                let Some((differences, at)) = best else { continue };
                if differences < EXCERPT_DIFFERENCES {
                    continue;
                }
                let range = at..at + EXCERPT_NOTES;
                let notes = piece.notes[range.clone()].iter().map(|n| crate::corpus::FingeredNote { finger: None, ..*n }).collect();
                let fingerings = BTreeMap::from([("model".to_string(), model[range.clone()].to_vec()), (label.to_string(), other[range].to_vec())]);
                out.push(Excerpt { tier: spec.id.clone(), piece: piece.piece.clone(), notes, fingerings });
                if label == "rules" { against_rules += 1 } else { against_people += 1 }
            }
        }
    }
    Ok(out)
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Manifest> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("reading the manifest {}", path.display()))
    }

    /// Refuse a benchmark whose files are not the ones it was made with.
    pub fn verify(&self) -> Result<()> {
        for (path, want) in &self.hashes {
            let got = hash_file(path)?;
            if &got != want {
                bail!("{} has changed since the benchmark was made ({got}, not {want}): a frozen benchmark is never edited", path.display());
            }
        }
        Ok(())
    }

    fn groups(&self) -> Result<HashMap<String, String>> {
        let mut groups = HashMap::new();
        if let Some(path) = &self.groups {
            let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            for line in text.lines() {
                let mut fields = line.trim_start_matches('\u{feff}').split('\t');
                if let (Some(piece), Some(group)) = (fields.next(), fields.next()) {
                    groups.insert(piece.trim().to_string(), group.trim().to_string());
                }
            }
        }
        Ok(groups)
    }
}

/// One fingering system measured on a tier: every piece's rates, and the comfort of
/// what it played. Each piece is solved once, on whichever thread has it, with all of
/// its annotations together, so a piece several people fingered is never cut in two.
fn measure(
    pieces: &[Piece],
    options: &FingeringOptions,
    prior: Option<&dyn FingeringPrior>,
    threads: usize,
) -> (Vec<PieceRates>, Playability) {
    let grouped: Vec<Vec<&Piece>> = by_piece(pieces).into_values().collect();
    let chunk = grouped.len().div_ceil(threads.max(1)).max(1);
    let parts: Vec<(Vec<PieceRates>, Playability)> = std::thread::scope(|scope| {
        let running: Vec<_> = grouped
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    let mut rates = Vec::new();
                    let mut comfort = Playability::default();
                    for annotations in part {
                        let first = annotations[0];
                        let (score, keys) = crate::eval::rebuild(first);
                        if keys.is_empty() {
                            continue;
                        }
                        let solution = on_fingering::finger_score_with_prior(&score, options, prior);
                        comfort.add(on_fingering::measure_solution(&score, &solution, options.span_model.table()));
                        let chosen: HashMap<_, u8> = solution.fingerings.iter().map(|f| (f.note, f.finger.number())).collect();
                        let ours: Vec<Option<u8>> = keys.iter().map(|id| chosen.get(id).copied()).collect();
                        if let Some(r) = rate_against(&first.piece, &ours, annotations) {
                            rates.push(r);
                        }
                    }
                    (rates, comfort)
                })
            })
            .collect();
        running.into_iter().map(|t| t.join().expect("a benchmark thread panicked")).collect()
    });
    let mut rates = Vec::new();
    let mut comfort = Playability::default();
    for (r, c) in parts {
        rates.extend(r);
        comfort.add(c);
    }
    rates.sort_by(|a, b| a.name.cmp(&b.name));
    (rates, comfort)
}

/// Measure the rules, and the model if there is one, on every tier.
pub fn run(
    manifest: &Manifest,
    options: &FingeringOptions,
    prior: Option<&dyn FingeringPrior>,
    model_name: Option<String>,
    rounds: usize,
    threads: usize,
    include_sealed: bool,
    mut progress: impl FnMut(&str),
) -> Result<Scorecard> {
    manifest.verify()?;
    let groups = manifest.groups()?;
    let mut tiers = Vec::new();
    for spec in manifest.tiers.iter().filter(|t| include_sealed || !t.sealed) {
        let mut options = options.clone();
        if spec.without_taught {
            options.pattern_scale = 0.0;
        }
        let options = &options;
        let mut rules_only = options.clone();
        rules_only.prior_scale = 0.0;
        let mut pieces = Vec::new();
        for dir in &spec.corpora {
            pieces.extend(Corpus::at(dir)?.load()?);
        }
        if let Some(floor) = spec.min_confidence {
            pieces = pieces.iter().map(|p| p.trusted(floor)).collect();
        }
        if pieces.is_empty() {
            bail!("tier {} has no pieces in {:?}", spec.id, spec.corpora);
        }
        progress(&format!("{}: {} ({} annotations)", spec.id, spec.title, pieces.len()));

        let started = std::time::Instant::now();
        let (mut rules, comfort_rules) = measure(&pieces, &rules_only, None, threads);
        let mut elapsed = started.elapsed();
        crate::eval::assign_groups(&mut rules, &groups);
        let notes: usize = rules.iter().map(|r| r.soft.1).sum();
        let group_count = rules.iter().map(|r| r.group.as_str()).collect::<std::collections::BTreeSet<_>>().len();
        let human = {
            let reference = human_reference(&pieces);
            (!reference.is_empty()).then(|| Rates::from(combine(&reference)))
        };
        let mut model_rates: Option<Vec<PieceRates>> = None;
        let (model, gain, comfort_model) = match prior {
            None => (None, None, None),
            Some(prior) => {
                let started = std::time::Instant::now();
                let (mut with, comfort) = measure(&pieces, options, Some(prior), threads);
                elapsed = started.elapsed();
                crate::eval::assign_groups(&mut with, &groups);
                let pair = |metric: fn(&[PieceRates]) -> f32| -> Result<Span> {
                    paired_by(&with, &rules, rounds, spec.cluster, metric)
                        .map(Span::from)
                        .map_err(|e| anyhow::anyhow!("tier {}: {e}", spec.id))
                };
                let gains = Gains {
                    general: pair(|r| combine(r).general)?,
                    ngram: pair(|r| combine(r).ngram)?,
                    recombined: pair(|r| combine(r).recombined)?,
                };
                let summary = Rates::from(combine(&with));
                model_rates = Some(with);
                (Some(summary), Some(gains), Some(Comfort::from(&comfort)))
            }
        };
        let judged = |rates: &[PieceRates]| {
            let mut w: Vec<(String, f32)> = rates.iter().map(|r| (r.name.clone(), combine(std::slice::from_ref(r)).general * 100.0)).collect();
            w.sort_by(|a, b| a.1.total_cmp(&b.1));
            w.truncate(10);
            w
        };
        let judged_rates = model_rates.as_ref().unwrap_or(&rules);
        let weakest = judged(judged_rates);
        let gate_failures: Vec<String> = judged_rates
            .iter()
            .filter(|r| spec.must_pass.iter().any(|prefix| r.name.starts_with(prefix.as_str())))
            .filter(|r| combine(std::slice::from_ref(r)).general < 1.0)
            .map(|r| r.name.clone())
            .collect();
        let probes = spec.probes.then(|| match prior {
            Some(prior) => probe(&pieces, options, Some(prior)),
            None => probe(&pieces, &rules_only, None),
        });
        let ms_per_1000_notes = (elapsed.as_secs_f64() * 1000.0 * threads as f64 / (notes.max(1) as f64 / 1000.0)) as f32;
        tiers.push(TierCard {
            id: spec.id.clone(),
            title: spec.title.clone(),
            pieces: rules.len(),
            groups: group_count,
            notes,
            rules: Rates::from(combine(&rules)),
            model,
            gain,
            human,
            comfort_rules: Comfort::from(&comfort_rules),
            comfort_model,
            comfort_human: Comfort::from(&human_comfort(&pieces, options)),
            gate_failures,
            probes,
            ms_per_1000_notes,
            weakest,
        });
    }
    let headline = headline(&tiers);
    Ok(Scorecard { bench: manifest.name.clone(), model: model_name, tiers, headline })
}

/// Fold a scorecard's tiers into the OpenNote Score. See [`Headline`].
pub fn headline(tiers: &[TierCard]) -> Headline {
    let judged = |t: &TierCard| t.model.unwrap_or(t.rules);
    let comfort_of = |t: &TierCard| t.comfort_model.unwrap_or(t.comfort_rules);
    let find = |prefix: &str| tiers.iter().find(|t| t.id.starts_with(prefix));
    let agreement = |r: Rates| {
        let general = ((r.general / 100.0 - RANDOM_NOTE) / (1.0 - RANDOM_NOTE)).clamp(0.0, 1.0);
        let run = ((r.ngram / 100.0 - RANDOM_RUN) / (1.0 - RANDOM_RUN)).clamp(0.0, 1.0);
        (general + run) / 2.0
    };
    let mut gated = None;
    for t in tiers {
        if !t.gate_failures.is_empty() {
            gated = Some(format!("{}: {} must-pass pieces wrong (first: {})", t.id, t.gate_failures.len(), t.gate_failures[0]));
            break;
        }
        // Unplayable is the model's fault only where it adds to what the rules alone do on
        // the same music: a part written for one hand that no hand can play is the
        // music's, and is reported, not held against the model.
        let added = comfort_of(t).unplayable_count.saturating_sub(t.comfort_rules.unplayable_count);
        if added > 0 {
            gated = Some(format!("{}: {added} unplayable transitions the rules alone do not have", t.id));
            break;
        }
    }
    let mut components: Vec<(String, f32, f32)> = Vec::new();
    // The several-reference slot: commissioned experts (T1) if a bench has them, otherwise
    // the same songs as several pianists played them (T2). Without either, `v2-pre`.
    let several = find("T1").or_else(|| find("T2"));
    let label = if several.is_some() { "v2" } else { "v2-pre" }.to_string();
    if let Some(t) = several {
        // Several references: the stitched (recombined) rate, and coherence over a run.
        let r = judged(t);
        let c = agreement(Rates { general: r.recombined, ..r });
        components.push((format!("{} agreement", t.id), c, 0.35));
    }
    if let Some(t) = find("T3") {
        components.push((format!("{} agreement", t.id), agreement(judged(t)), 0.20));
        let (h, m) = (t.comfort_human, comfort_of(t));
        let ratio = |human: f32, model: f32| ((human + 0.01) / (model + 0.01)).min(1.0);
        let parts = [
            ratio(h.hand_moves, m.hand_moves),
            ratio(h.stretched, m.stretched),
            ratio(h.thumbless, m.thumbless),
            ratio(h.weak_on_black, m.weak_on_black),
            ratio(h.step_spread, m.step_spread),
        ];
        let geometric = parts.iter().map(|p| p.max(1e-3).ln()).sum::<f32>() / parts.len() as f32;
        components.push(("comfort against the pianists".to_string(), geometric.exp(), 0.25));
    }
    // Written music: one editor per score (T4), and the same piece in several editions
    // (T5, judged like T2 on the stitched rate), sharing one weight.
    let stitched = |t: &TierCard| {
        let r = judged(t);
        agreement(Rates { general: r.recombined, ..r })
    };
    let written: Vec<f32> = [find("T4").map(|t| agreement(judged(t))), find("T5").map(stitched)].into_iter().flatten().collect();
    if !written.is_empty() {
        components.push(("written agreement".to_string(), written.iter().sum::<f32>() / written.len() as f32, 0.10));
    }
    let probed: Vec<f32> = tiers.iter().filter_map(|t| t.probes.map(|p| p.score())).collect();
    if !probed.is_empty() {
        components.push(("consistency".to_string(), probed.iter().sum::<f32>() / probed.len() as f32, 0.10));
    }
    let total: f32 = components.iter().map(|c| c.2).sum();
    for c in &mut components {
        c.2 /= total.max(1e-9);
    }
    let score = (gated.is_none() && !components.is_empty())
        .then(|| 100.0 * components.iter().map(|(_, c, w)| c.max(0.01).powf(*w)).product::<f32>());
    Headline { score, gated, label, components }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_file_is_refused() {
        let dir = std::env::temp_dir().join("opennote-bench-hash");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data.jsonl");
        std::fs::write(&file, "one").unwrap();
        let mut manifest = Manifest { name: "t".into(), tiers: vec![], groups: None, hashes: BTreeMap::new() };
        manifest.hashes.insert(file.clone(), hash_file(&file).unwrap());
        assert!(manifest.verify().is_ok());
        std::fs::write(&file, "two").unwrap();
        assert!(manifest.verify().is_err(), "an edited benchmark must not run");
    }

    fn card(id: &str, general: f32, ngram: f32) -> TierCard {
        let rates = Rates { general, highest: general, soft: general, recombined: general, ngram };
        let comfort = Comfort {
            unplayable: 0.0, unplayable_count: 0, hand_moves: 25.0, travel_mm: 25.0, stretched: 4.0, thumb_unders: 3.0, finger_overs: 3.0,
            thumbless: 0.1, step_spread: 2.0, chord_spread: 2.0, weak_on_black: 1.0,
        };
        TierCard {
            id: id.into(), title: id.into(), pieces: 1, groups: 1, notes: 1, rules: rates, model: None, gain: None,
            human: None, comfort_rules: comfort, comfort_model: None, comfort_human: comfort, gate_failures: vec![],
            probes: None, ms_per_1000_notes: 0.0, weakest: vec![],
        }
    }

    #[test]
    fn the_headline_is_withheld_rather_than_shown_wrong() {
        let mut tiers = vec![card("T3-dev", 70.0, 45.0), card("T4-dev", 57.0, 26.0)];
        let fine = headline(&tiers);
        assert!(fine.score.is_some() && fine.gated.is_none(), "{fine:?}");
        assert_eq!(fine.label, "v2-pre", "no tier with several references");
        assert_eq!(headline(&[card("T2-dev", 60.0, 30.0)]).label, "v2", "several pianists stand in for experts");
        let written = headline(&[card("T4-dev", 50.0, 20.0), card("T5-dev", 70.0, 40.0)]);
        assert_eq!(written.components.len(), 1, "one editor and several editions share the written weight");
        assert!(written.components[0].1 > headline(&[card("T4-dev", 50.0, 20.0)]).components[0].1);
        tiers[0].gate_failures = vec!["t0-major-C-right".into()];
        let gated = headline(&tiers);
        assert!(gated.score.is_none() && gated.gated.is_some());
        tiers[0].gate_failures.clear();
        tiers[1].comfort_rules.unplayable_count = 2;
        assert!(headline(&tiers).score.is_some(), "the music's own impossibilities are not the model's");
        tiers[1].comfort_model = Some(Comfort { unplayable_count: 3, ..tiers[1].comfort_rules });
        assert!(headline(&tiers).score.is_none(), "an unplayable move the model adds gates the score");
    }

    #[test]
    fn better_agreement_scores_higher_and_comfort_is_capped_at_human() {
        let base = headline(&[card("T3-dev", 70.0, 45.0)]).score.unwrap();
        let better = headline(&[card("T3-dev", 75.0, 50.0)]).score.unwrap();
        assert!(better > base, "{better} vs {base}");
        // More comfortable than the pianists earns nothing extra; less costs.
        let mut easier = card("T3-dev", 70.0, 45.0);
        easier.comfort_rules.hand_moves = 10.0;
        assert!((headline(&[easier]).score.unwrap() - base).abs() < 1e-3);
        let mut harder = card("T3-dev", 70.0, 45.0);
        harder.comfort_rules.hand_moves = 50.0;
        assert!(headline(&[harder]).score.unwrap() < base);
    }

    #[test]
    fn the_hash_is_fnv_1a() {
        let dir = std::env::temp_dir().join("opennote-bench-fnv");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "a").unwrap();
        assert_eq!(hash_file(&file).unwrap(), "af63dc4c8601ec8c");
    }
}
