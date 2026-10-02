//! OpenNote's own benchmark: a manifest of frozen tiers, measured into one scorecard.
//!
//! A tier is a set of fingered pieces with one kind of truth — pianists read off video,
//! editors' printed fingerings, several experts on the same music — and each is measured
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
}

/// Everything measured, for a model and the rules it is built on.
#[derive(Debug, Clone, Serialize)]
pub struct Scorecard {
    pub bench: String,
    pub model: Option<String>,
    pub tiers: Vec<TierCard>,
}

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
    /// Hand position changes per 100 transitions.
    pub hand_moves: f32,
    /// Millimetres travelled per transition.
    pub travel_mm: f32,
    /// Finger pairs held past a comfortable span, in percent.
    pub stretched: f32,
}

impl From<&Playability> for Comfort {
    fn from(p: &Playability) -> Self {
        Comfort {
            unplayable: p.unplayable_rate() * 100.0,
            hand_moves: p.change_rate() * 100.0,
            travel_mm: (p.travel_mm / p.transitions.max(1) as f64) as f32,
            stretched: p.stretched_rate() * 100.0,
        }
    }
}

/// FNV-1a of a file's bytes, as hex: what the manifest pins.
pub fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    Ok(format!("{hash:016x}"))
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
    mut progress: impl FnMut(&str),
) -> Result<Scorecard> {
    manifest.verify()?;
    let groups = manifest.groups()?;
    let mut rules_only = options.clone();
    rules_only.prior_scale = 0.0;
    let mut tiers = Vec::new();
    for spec in &manifest.tiers {
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

        let (mut rules, comfort_rules) = measure(&pieces, &rules_only, None, threads);
        crate::eval::assign_groups(&mut rules, &groups);
        let notes = rules.iter().map(|r| r.soft.1).sum();
        let group_count = rules.iter().map(|r| r.group.as_str()).collect::<std::collections::BTreeSet<_>>().len();
        let human = {
            let reference = human_reference(&pieces);
            (!reference.is_empty()).then(|| Rates::from(combine(&reference)))
        };
        let (model, gain, comfort_model) = match prior {
            None => (None, None, None),
            Some(prior) => {
                let (mut with, comfort) = measure(&pieces, options, Some(prior), threads);
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
                (Some(Rates::from(combine(&with))), Some(gains), Some(Comfort::from(&comfort)))
            }
        };
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
        });
    }
    Ok(Scorecard { bench: manifest.name.clone(), model: model_name, tiers })
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

    #[test]
    fn the_hash_is_fnv_1a() {
        let dir = std::env::temp_dir().join("opennote-bench-fnv");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "a").unwrap();
        assert_eq!(hash_file(&file).unwrap(), "af63dc4c8601ec8c");
    }
}
