use std::collections::{BTreeMap, HashMap};

use on_fingering::{FingeringOptions, FingeringPrior};
use on_hand::Hand;
use on_score::{Note, NoteId, Score, TICKS_PER_QUARTER};

use crate::corpus::{by_piece, HandLabel, Piece};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MatchRates {
    pub general: f32,
    pub highest: f32,
    pub soft: f32,
    pub recombined: f32,
    pub ngram: f32,
    pub notes: usize,
    pub pieces: usize,
}

impl std::fmt::Display for MatchRates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "general {:.1}%   highest {:.1}%   soft {:.1}%   recombined {:.1}%   4-gram {:.1}%   \
             ({} notes, {} pieces)",
            self.general * 100.0,
            self.highest * 100.0,
            self.soft * 100.0,
            self.recombined * 100.0,
            self.ngram * 100.0,
            self.notes,
            self.pieces
        )
    }
}

pub fn evaluate(
    pieces: &[Piece],
    options: &FingeringOptions,
    prior: Option<&dyn FingeringPrior>,
) -> MatchRates {
    combine(&per_piece(pieces, options, prior))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PieceRates {
    pub name: String,
    pub group: String,
    pub general: Vec<f32>,
    pub highest: f32,
    pub soft: (usize, usize),
    pub recombined: Option<f32>,
    pub ngram: (usize, usize),
}

pub const NGRAM: usize = 4;

pub fn per_piece(
    pieces: &[Piece],
    options: &FingeringOptions,
    prior: Option<&dyn FingeringPrior>,
) -> Vec<PieceRates> {
    let mut out = Vec::new();
    for (name, annotations) in by_piece(pieces) {
        let Some(ours) = solve(annotations[0], options, prior) else { continue };
        if let Some(rates) = rate_against(name, &ours, &annotations) {
            out.push(rates);
        }
    }
    out
}

pub fn solve(piece: &Piece, options: &FingeringOptions, prior: Option<&dyn FingeringPrior>) -> Option<Vec<Option<u8>>> {
    let (score, keys) = rebuild(piece);
    if keys.is_empty() {
        return None;
    }
    let solution = on_fingering::finger_score_with_prior(&score, options, prior);
    let chosen: HashMap<NoteId, u8> = solution.fingerings.iter().map(|f| (f.note, f.finger.number())).collect();
    Some(keys.iter().map(|id| chosen.get(id).copied()).collect())
}

pub fn rate_against(name: &str, ours: &[Option<u8>], annotations: &[&Piece]) -> Option<PieceRates> {
    let mut this = PieceRates { name: name.to_string(), group: name.to_string(), ..Default::default() };
    let mut best = 0.0f32;
    let mut acceptable: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
    let mut truths: Vec<Vec<Option<u8>>> = Vec::new();

    for annotation in annotations {
        let (mut agreed, mut compared) = (0usize, 0usize);
        let mut row = vec![None; annotation.notes.len()];
        for (index, note) in annotation.notes.iter().enumerate() {
            let Some(Some(mine)) = ours.get(index) else { continue };
            let Some(theirs) = note.finger else { continue };
            compared += 1;
            acceptable.entry(index).or_default().push(theirs);
            row[index] = Some(theirs);
            agreed += usize::from(*mine == theirs);
        }
        if compared == 0 {
            continue;
        }
        truths.push(row);
        let rate = agreed as f32 / compared as f32;
        best = best.max(rate);
        this.general.push(rate);
    }
    if this.general.is_empty() {
        return None;
    }
    this.highest = best;

    let shared: Vec<usize> = (0..truths[0].len())
        .filter(|i| truths.iter().all(|row| row[*i].is_some()))
        .filter(|i| matches!(ours.get(*i), Some(Some(_))))
        .collect();
    if !shared.is_empty() {
        let rows: Vec<Vec<u8>> =
            truths.iter().map(|row| shared.iter().map(|i| row[*i].expect("filtered")).collect()).collect();
        let mine: Vec<u8> = shared.iter().map(|i| ours[*i].expect("filtered")).collect();
        this.recombined = Some(recombined(&rows, &mine));
    }

    for (index, fingers) in acceptable {
        let Some(Some(mine)) = ours.get(index) else { continue };
        this.soft.1 += 1;
        this.soft.0 += usize::from(fingers.contains(mine));
    }

    let notes = &annotations[0].notes;
    for hand in [HandLabel::Left, HandLabel::Right] {
        let line: Vec<usize> = (0..notes.len()).filter(|&i| notes[i].hand == hand).collect();
        for window in line.windows(NGRAM) {
            if window.iter().any(|&i| !matches!(ours.get(i), Some(Some(_)))) {
                continue;
            }
            let full: Vec<&Vec<Option<u8>>> = truths.iter().filter(|row| window.iter().all(|&i| row[i].is_some())).collect();
            if full.is_empty() {
                continue;
            }
            this.ngram.1 += 1;
            this.ngram.0 += usize::from(full.iter().any(|row| window.iter().all(|&i| row[i] == ours[i])));
        }
    }
    Some(this)
}

pub fn human_reference(pieces: &[Piece]) -> Vec<PieceRates> {
    let mut out = Vec::new();
    for (name, annotations) in by_piece(pieces) {
        if annotations.len() < 2 {
            continue;
        }
        for (k, left_out) in annotations.iter().enumerate() {
            let rest: Vec<&Piece> = annotations.iter().enumerate().filter(|(j, _)| *j != k).map(|(_, a)| *a).collect();
            let theirs: Vec<Option<u8>> = left_out.notes.iter().map(|n| n.finger).collect();
            if let Some(rates) = rate_against(&format!("{name}#{k}"), &theirs, &rest) {
                out.push(rates);
            }
        }
    }
    out
}

pub fn per_piece_loo(pieces: &[Piece], options: &FingeringOptions, prior: Option<&dyn FingeringPrior>) -> Vec<PieceRates> {
    let mut out = Vec::new();
    for (name, annotations) in by_piece(pieces) {
        if annotations.len() < 2 {
            continue;
        }
        let Some(ours) = solve(annotations[0], options, prior) else { continue };
        for k in 0..annotations.len() {
            let rest: Vec<&Piece> = annotations.iter().enumerate().filter(|(j, _)| *j != k).map(|(_, a)| *a).collect();
            if let Some(rates) = rate_against(&format!("{name}#{k}"), &ours, &rest) {
                out.push(rates);
            }
        }
    }
    out
}

pub fn assign_groups(rates: &mut [PieceRates], groups: &HashMap<String, String>) {
    for r in rates {
        let base = r.name.split('#').next().unwrap_or(&r.name);
        if let Some(group) = groups.get(base) {
            r.group = group.clone();
        }
    }
}

pub fn combine(pieces: &[PieceRates]) -> MatchRates {
    let general: Vec<f32> = pieces.iter().flat_map(|p| p.general.iter().copied()).collect();
    let highest: Vec<f32> = pieces.iter().map(|p| p.highest).collect();
    let recombined: Vec<f32> = pieces.iter().filter_map(|p| p.recombined).collect();
    let matched: usize = pieces.iter().map(|p| p.soft.0).sum();
    let notes: usize = pieces.iter().map(|p| p.soft.1).sum();
    let runs: usize = pieces.iter().map(|p| p.ngram.1).sum();
    let runs_matched: usize = pieces.iter().map(|p| p.ngram.0).sum();

    MatchRates {
        general: ratio(general.iter().sum(), general.len()),
        highest: ratio(highest.iter().sum(), highest.len()),
        soft: ratio(matched as f32, notes),
        recombined: ratio(recombined.iter().sum(), recombined.len()),
        ngram: ratio(runs_matched as f32, runs),
        notes,
        pieces: pieces.len(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    pub point: f32,
    pub low: f32,
    pub high: f32,
}

impl std::fmt::Display for Interval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:.1}% ({:.1}-{:.1})",
            self.point * 100.0,
            self.low * 100.0,
            self.high * 100.0
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Confidence {
    pub general: Interval,
    pub highest: Interval,
    pub soft: Interval,
    pub recombined: Interval,
    pub rounds: usize,
    pub pieces: usize,
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "general {}   highest {}   soft {}   recombined {}   \
             ({} pieces, {} resamplings)",
            self.general, self.highest, self.soft, self.recombined, self.pieces, self.rounds
        )
    }
}

pub fn paired(ours: &[PieceRates], theirs: &[PieceRates], rounds: usize) -> Interval {
    paired_by(ours, theirs, rounds, false, |r| combine(r).general).expect("the two were measured on different pieces")
}

pub fn paired_by(
    ours: &[PieceRates],
    theirs: &[PieceRates],
    rounds: usize,
    cluster: bool,
    metric: impl Fn(&[PieceRates]) -> f32,
) -> Result<Interval, String> {
    let key = |list: &[PieceRates]| {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        list.iter()
            .map(|r| {
                let n = seen.entry(r.name.as_str()).or_insert(0);
                *n += 1;
                (r.name.clone(), *n)
            })
            .collect::<Vec<_>>()
    };
    let theirs_at: HashMap<(String, usize), usize> = key(theirs).into_iter().enumerate().map(|(i, k)| (k, i)).collect();
    let mut pairs: Vec<(usize, usize)> = Vec::with_capacity(ours.len());
    for (i, k) in key(ours).into_iter().enumerate() {
        match theirs_at.get(&k) {
            Some(&j) => pairs.push((i, j)),
            None => return Err(format!("piece {} was measured for one system only", k.0)),
        }
    }
    if pairs.len() != theirs.len() {
        return Err(format!("{} pieces against {}: measured on different pieces", ours.len(), theirs.len()));
    }
    let units: Vec<Vec<usize>> = if cluster {
        let mut by: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (p, &(i, _)) in pairs.iter().enumerate() {
            by.entry(ours[i].group.as_str()).or_default().push(p);
        }
        by.into_values().collect()
    } else {
        (0..pairs.len()).map(|p| vec![p]).collect()
    };
    let gap = |chosen: &[usize]| {
        let a: Vec<PieceRates> = chosen.iter().map(|&p| ours[pairs[p].0].clone()).collect();
        let b: Vec<PieceRates> = chosen.iter().map(|&p| theirs[pairs[p].1].clone()).collect();
        metric(&a) - metric(&b)
    };
    let all: Vec<usize> = (0..pairs.len()).collect();
    let point = gap(&all);
    let mut draws = Vec::with_capacity(rounds);
    let mut rng: u64 = 0x2545_f491_4f6c_dd1d;
    for _ in 0..rounds.max(1) {
        let mut chosen = Vec::with_capacity(pairs.len());
        for _ in 0..units.len() {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            chosen.extend_from_slice(&units[(rng >> 33) as usize % units.len().max(1)]);
        }
        draws.push(gap(&chosen));
    }
    draws.sort_by(f32::total_cmp);
    let at = |q: f64| draws[((draws.len() - 1) as f64 * q).round() as usize];
    Ok(Interval { point, low: at(0.025), high: at(0.975) })
}

pub fn confidence(pieces: &[PieceRates], rounds: usize) -> Confidence {
    let point = combine(pieces);
    let mut draws: Vec<MatchRates> = Vec::with_capacity(rounds);
    let mut rng: u64 = 0x2545_f491_4f6c_dd1d;
    for _ in 0..rounds.max(1) {
        let drawn: Vec<PieceRates> = (0..pieces.len())
            .map(|_| {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                pieces[(rng >> 33) as usize % pieces.len().max(1)].clone()
            })
            .collect();
        draws.push(combine(&drawn));
    }

    let band = |of: fn(&MatchRates) -> f32, middle: f32| {
        let mut values: Vec<f32> = draws.iter().map(of).collect();
        values.sort_by(f32::total_cmp);
        let at = |p: f64| values[((values.len() - 1) as f64 * p) as usize];
        Interval { point: middle, low: at(0.025), high: at(0.975) }
    };

    Confidence {
        general: band(|r| r.general, point.general),
        highest: band(|r| r.highest, point.highest),
        soft: band(|r| r.soft, point.soft),
        recombined: band(|r| r.recombined, point.recombined),
        rounds: rounds.max(1),
        pieces: pieces.len(),
    }
}

pub fn recombined(truths: &[Vec<u8>], ours: &[u8]) -> f32 {
    let notes = ours.len();
    if truths.is_empty() || notes == 0 {
        return 0.0;
    }
    debug_assert!(truths.iter().all(|t| t.len() == notes));

    const RECOMBINATION: f32 = 1.0;
    const SUBSTITUTION: f32 = 1.0;

    let mut cost: Vec<f32> = truths
        .iter()
        .map(|t| if t[0] == ours[0] { 0.0 } else { SUBSTITUTION })
        .collect();

    for n in 1..notes {
        let previous = cost.clone();
        for (g, truth) in truths.iter().enumerate() {
            let mut best = previous[g];
            for (h, other) in truths.iter().enumerate() {
                if h != g && other[n] == truth[n] {
                    best = best.min(previous[h] + RECOMBINATION);
                }
            }
            cost[g] = best + if truth[n] == ours[n] { 0.0 } else { SUBSTITUTION };
        }
    }

    let error = cost.iter().copied().fold(f32::INFINITY, f32::min);
    ((notes as f32 - error) / notes as f32).max(0.0)
}

fn ratio(total: f32, count: usize) -> f32 {
    if count == 0 {
        0.0
    } else {
        total / count as f32
    }
}

pub fn rebuild(piece: &Piece) -> (Score, Vec<NoteId>) {
    let mut score = Score { tempo: on_score::TempoMap::constant_bpm(60.0), ..Default::default() };
    let mut keys = Vec::with_capacity(piece.notes.len());
    for note in &piece.notes {
        let id = NoteId(score.notes.len() as u32);
        keys.push(id);
        score.notes.push(Note {
            id,
            midi: note.midi,
            onset: (note.onset * f64::from(TICKS_PER_QUARTER)).round() as i64,
            duration: (note.duration * f64::from(TICKS_PER_QUARTER)).round() as i64,
            onset_seconds: note.onset,
            duration_seconds: note.duration,
            staff: Some(match note.hand {
                HandLabel::Right => 1,
                HandLabel::Left => 2,
            }),
            voice: Some(1),
            hand: Some(Hand::from(note.hand)),
            grace: false,
            chord: false,
            velocity: 80,
            tie: Default::default(),
            given_finger: None,
            source: on_score::SourceRef::Midi { track: 0, event: 0 },
        });
    }
    score.finalise();
    (score, keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::FingeredNote;

    fn rates(general: f32) -> PieceRates {
        PieceRates { general: vec![general], highest: general, ..Default::default() }
    }

    #[test]
    fn a_system_against_itself_differs_by_nothing() {
        let pieces: Vec<PieceRates> = (0..20).map(|i| rates(0.5 + i as f32 / 100.0)).collect();
        let gap = paired(&pieces, &pieces, 500);
        assert_eq!((gap.point, gap.low, gap.high), (0.0, 0.0, 0.0));
    }

    #[test]
    fn a_small_gain_on_every_piece_is_real_even_when_the_pieces_vary_widely() {
        let theirs: Vec<PieceRates> = (0..20).map(|i| rates(0.3 + i as f32 * 0.03)).collect();
        let ours: Vec<PieceRates> = (0..20).map(|i| rates(0.32 + i as f32 * 0.03)).collect();
        let gap = paired(&ours, &theirs, 500);
        assert!(gap.low > 0.0 && (gap.point - 0.02).abs() < 1e-4, "{gap}");
    }

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

    #[test]
    fn following_one_annotation_all_the_way_is_free() {
        let truths = vec![vec![1, 2, 3, 1, 2], vec![1, 3, 4, 1, 2]];
        assert!((recombined(&truths, &[1, 2, 3, 1, 2]) - 1.0).abs() < 1e-6);
        assert!((recombined(&truths, &[1, 3, 4, 1, 2]) - 1.0).abs() < 1e-6);
    }

    fn scored(rate: f32, notes: usize) -> PieceRates {
        PieceRates {
            general: vec![rate],
            highest: rate,
            soft: ((rate * notes as f32) as usize, notes),
            recombined: Some(rate),
            ..Default::default()
        }
    }

    #[test]
    fn resampling_brackets_the_answer_and_narrows_with_more_pieces() {
        let spread = |n: usize| -> Vec<PieceRates> {
            (0..n).map(|i| scored(0.4 + (i % 7) as f32 * 0.05, 100)).collect()
        };

        let few = confidence(&spread(12), 400);
        let many = confidence(&spread(400), 400);

        for band in [few.general, few.highest, few.soft, many.general] {
            assert!(band.low <= band.point && band.point <= band.high, "{band}");
        }
        let width = |b: Interval| b.high - b.low;
        assert!(
            width(many.general) < width(few.general) / 2.0,
            "400 pieces gave {} and 12 gave {}",
            many.general,
            few.general
        );
    }

    #[test]
    fn the_same_pieces_give_the_same_interval() {
        let pieces: Vec<PieceRates> = (0..40).map(|i| scored(0.3 + (i % 5) as f32 * 0.1, 80)).collect();
        assert_eq!(confidence(&pieces, 200), confidence(&pieces, 200));
    }

    #[test]
    fn a_coherent_stitch_beats_a_shuffle() {
        let a = vec![1, 2, 5, 3, 3];
        let b = vec![4, 4, 5, 2, 2];
        let truths = vec![a.clone(), b.clone()];

        let coherent = vec![1, 2, 5, 2, 2];
        let shuffled = vec![4, 2, 5, 3, 2];

        for note in 0..5 {
            assert!(coherent[note] == a[note] || coherent[note] == b[note]);
            assert!(shuffled[note] == a[note] || shuffled[note] == b[note]);
        }

        let stitched = recombined(&truths, &coherent);
        let shuffle = recombined(&truths, &shuffled);
        assert!(
            stitched > shuffle,
            "one crossing scored {stitched}, four scored {shuffle}"
        );
        assert!((stitched - 0.8).abs() < 1e-6, "{stitched}");
    }

    #[test]
    fn a_stitch_costs_what_a_mistake_costs() {
        let truths = vec![vec![1, 2, 3, 2, 1], vec![1, 4, 3, 5, 1]];
        let one_error = recombined(&truths, &[1, 2, 3, 2, 5]);
        let one_switch = recombined(&truths, &[1, 2, 3, 5, 1]);
        assert!((one_error - one_switch).abs() < 1e-6, "{one_error} vs {one_switch}");
        assert!((one_error - 0.8).abs() < 1e-6, "{one_error}");
    }

    #[test]
    fn one_annotation_gives_the_plain_match_rate() {
        let truths = vec![vec![1, 2, 3, 4]];
        assert!((recombined(&truths, &[1, 2, 3, 4]) - 1.0).abs() < 1e-6);
        assert!((recombined(&truths, &[1, 2, 3, 1]) - 0.75).abs() < 1e-6);
        assert!((recombined(&truths, &[5, 5, 5, 5]) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn recombining_nothing_is_zero() {
        assert_eq!(recombined(&[], &[1, 2]), 0.0);
        assert_eq!(recombined(&[vec![]], &[]), 0.0);
    }

    #[test]
    fn the_recombined_rate_sits_between_the_highest_and_the_soft() {
        let notes: Vec<(u8, u8)> = vec![(60, 1), (62, 2), (64, 3), (65, 1), (67, 2)];
        let other: Vec<(u8, u8)> = vec![(60, 1), (62, 3), (64, 4), (65, 1), (67, 2)];
        let pieces = vec![
            piece("c-major", "1", &notes),
            piece("c-major", "2", &other),
        ];
        let rates = evaluate(&pieces, &FingeringOptions::default(), None);
        assert!(rates.recombined >= rates.highest - 1e-6, "{rates}");
        assert!(rates.soft >= rates.recombined - 1e-6, "{rates}");
    }

    #[test]
    fn a_textbook_scale_agrees_completely() {
        let scale = piece(
            "c-major",
            "1",
            &[
                (60, 1),
                (62, 2),
                (64, 3),
                (65, 1),
                (67, 2),
                (69, 3),
                (71, 4),
                (72, 5),
            ],
        );
        let rates = evaluate(&[scale], &FingeringOptions::default(), None);
        assert_eq!(rates.pieces, 1);
        assert!(rates.general > 0.99, "{rates}");
        assert!((rates.soft - rates.general).abs() < 1e-6, "{rates}");
    }

    #[test]
    fn the_three_rates_are_ordered() {
        let notes: Vec<(u8, u8)> = vec![(60, 1), (62, 2), (64, 3), (65, 1), (67, 2)];
        let other: Vec<(u8, u8)> = vec![(60, 1), (62, 3), (64, 4), (65, 1), (67, 2)];
        let pieces = vec![
            piece("c-major", "1", &notes),
            piece("c-major", "2", &other),
        ];
        let rates = evaluate(&pieces, &FingeringOptions::default(), None);
        assert!(rates.soft >= rates.highest - 1e-6, "{rates}");
        assert!(rates.highest >= rates.general - 1e-6, "{rates}");
        assert_eq!(rates.pieces, 1);
    }

    #[test]
    fn an_empty_set_reports_zero() {
        let rates = evaluate(&[], &FingeringOptions::default(), None);
        assert_eq!(rates, MatchRates::default());
    }

    fn named(name: &str, group: &str, general: f32) -> PieceRates {
        PieceRates { name: name.into(), group: group.into(), general: vec![general], highest: general, ..Default::default() }
    }

    #[test]
    fn pieces_are_paired_by_name_not_by_order() {
        let ours: Vec<PieceRates> = (0..30).map(|i| named(&format!("p{i}"), "g", 0.5 + i as f32 / 100.0)).collect();
        let theirs: Vec<PieceRates> = (0..30).map(|i| named(&format!("p{i}"), "g", 0.48 + i as f32 / 100.0)).collect();
        let mut shuffled = theirs.clone();
        shuffled.reverse();
        let general = |r: &[PieceRates]| combine(r).general;
        let a = paired_by(&ours, &theirs, 500, false, general).unwrap();
        let b = paired_by(&ours, &shuffled, 500, false, general).unwrap();
        assert!((a.point - 0.02).abs() < 1e-5 && (b.point - 0.02).abs() < 1e-5, "{a:?} {b:?}");
        assert!(b.low > 0.0, "the same pairs, whatever the order: {b:?}");
    }

    #[test]
    fn a_piece_measured_for_one_system_only_is_an_error() {
        let ours = vec![named("a", "g", 0.5), named("b", "g", 0.5)];
        let theirs = vec![named("a", "g", 0.5), named("c", "g", 0.5)];
        assert!(paired_by(&ours, &theirs, 10, false, |r| combine(r).general).is_err());
        assert!(paired_by(&ours, &ours[..1], 10, false, |r| combine(r).general).is_err());
    }

    #[test]
    fn resampling_whole_groups_does_not_understate_the_width() {
        let mut ours = Vec::new();
        let mut theirs = Vec::new();
        for g in 0..10 {
            let gain = if g % 2 == 0 { 0.04 } else { -0.02 };
            for v in 0..10 {
                let name = format!("g{g}v{v}");
                theirs.push(named(&name, &format!("g{g}"), 0.6));
                ours.push(named(&name, &format!("g{g}"), 0.6 + gain));
            }
        }
        let general = |r: &[PieceRates]| combine(r).general;
        let loose = paired_by(&ours, &theirs, 2000, false, general).unwrap();
        let grouped = paired_by(&ours, &theirs, 2000, true, general).unwrap();
        assert!(grouped.high - grouped.low > loose.high - loose.low, "{grouped:?} vs {loose:?}");
    }

    #[test]
    fn four_notes_in_a_row_count_only_as_one_pianist_played_them() {
        let a = piece("x", "1", &[(60, 1), (62, 2), (64, 3), (65, 4), (67, 5)]);
        let b = piece("x", "2", &[(60, 1), (62, 3), (64, 2), (65, 4), (67, 5)]);
        let mixed = [Some(1), Some(2), Some(2), Some(4), Some(5)];
        let rates = rate_against("x", &mixed, &[&a, &b]).unwrap();
        assert_eq!(rates.soft, (5, 5), "every note matches somebody");
        assert_eq!(rates.ngram, (0, 2), "no run of four is anybody's");
        let following_a = [Some(1), Some(2), Some(3), Some(4), Some(5)];
        assert_eq!(rate_against("x", &following_a, &[&a, &b]).unwrap().ngram, (2, 2));
    }

    #[test]
    fn a_pianist_is_measured_against_the_others_exactly_as_the_model_is() {
        let pieces = vec![
            piece("x", "1", &[(60, 1), (62, 2), (64, 3), (65, 4)]),
            piece("x", "2", &[(60, 1), (62, 2), (64, 3), (65, 4)]),
            piece("x", "3", &[(60, 1), (62, 3), (64, 4), (65, 5)]),
        ];
        let human = human_reference(&pieces);
        assert_eq!(human.len(), 3, "each pianist left out once");
        assert_eq!(human[0].name, "x#0");
        assert_eq!(human[0].highest, 1.0);
        assert_eq!(human[0].general, vec![1.0, 0.25]);
        let model = per_piece_loo(&pieces, &FingeringOptions::default(), None);
        let names: Vec<&str> = model.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["x#0", "x#1", "x#2"]);
        assert!(paired_by(&model, &human, 50, false, |r| combine(r).general).is_ok());
    }
}
