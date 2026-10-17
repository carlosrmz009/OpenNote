//! Fingering a score: the options, the solution and its explanations, and how the two hands
//! and the four published rule sets are put together.

use on_hand::{Finger, Hand, HandProfile};
use on_score::{Fingering, NoteId, Score};

use crate::biomech::{BiomechModel, BiomechWeights, Grip};
use crate::ruler::Ruler;
use crate::rules::{self, Placement, Rule, RuleScorer, RuleSet, RuleWeights, Trigram};
use crate::spans::SpanModel;

mod events;
mod search;

pub use events::*;
pub(crate) use search::*;

const WITHIN_CHORD_RULES: &[Rule] = &[
    Rule::Stretch,
    Rule::SmallSpan,
    Rule::LargeSpan,
    Rule::Impractical,
    Rule::ThreeToFour,
    Rule::FourOnBlack,
];

const PER_NOTE_RULES: &[Rule] = &[Rule::WeakFinger];

const SHIFTED_VOICE_RULES: &[Rule] = &[Rule::WeakFinger, Rule::Impractical, Rule::RepeatedFinger];

#[derive(Debug, Clone)]
pub struct FingeringOptions {
    pub profile: HandProfile,
    pub rule_set: RuleSet,
    pub span_model: SpanModel,
    pub ruler: Ruler,
    pub rule_weights: RuleWeights,
    pub biomech: BiomechWeights,
    pub rule_scale: f32,
    pub prior_scale: f32,
    pub pattern_scale: f32,
}

impl Default for FingeringOptions {
    fn default() -> Self {
        let profile = HandProfile::default();
        Self {
            span_model: SpanModel::for_hand_size(profile.size()),
            profile,
            rule_set: RuleSet::default(),
            ruler: Ruler::default(),
            rule_weights: RuleWeights::default(),
            biomech: BiomechWeights::default(),
            rule_scale: 1.0,
            prior_scale: 0.0,
            pattern_scale: 1.0,
        }
    }
}

impl FingeringOptions {
    pub fn for_hand(profile: HandProfile) -> Self {
        Self { span_model: SpanModel::for_hand_size(profile.size()), profile, ..Default::default() }
    }
}

pub trait FingeringPrior: Sync {
    fn log_probability(
        &self,
        hand: Hand,
        prev2: Option<Placement>,
        prev: Option<Placement>,
        current: Placement,
    ) -> f32;

    fn chord_cost(&self, _hand: Hand, _notes: &[u8], _fingers: &[Finger]) -> f32 {
        0.0
    }

    fn recall(&self, _score: &Score) -> Vec<(NoteId, Finger)> {
        Vec::new()
    }

    fn suggest(&self, _score: &Score) -> std::collections::HashMap<NoteId, [f32; 5]> {
        Default::default()
    }

    fn step_cost(
        &self,
        _hand: Hand,
        _from: (&[u8], &[Finger]),
        _to: (&[u8], &[Finger]),
        _seconds: f64,
    ) -> f32 {
        0.0
    }

    fn trigram_cost(
        &self,
        _hand: Hand,
        _a: (&[u8], &[Finger]),
        _b: (&[u8], &[Finger]),
        _c: (&[u8], &[Finger]),
    ) -> f32 {
        0.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub hand: Hand,
    pub notes: Vec<u8>,
    pub fingers: Vec<Finger>,
    pub onset_seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub fingers: Vec<Finger>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CostBreakdown {
    pub rules: f32,
    pub posture: f32,
    pub pattern: f32,
    pub motion: f32,
    pub agreement: f32,
    pub prior: f32,
    pub learned: f32,
}

impl CostBreakdown {
    pub fn total(&self) -> f32 {
        self.rules + self.posture + self.pattern + self.motion + self.agreement + self.prior + self.learned
    }
}

#[derive(Debug, Clone)]
pub struct NoteExplanation {
    pub note: NoteId,
    pub finger: Finger,
    pub rules: Vec<(Rule, f32)>,
    pub posture_strain: f32,
    pub reachable: bool,
    pub margin: f32,
    pub cost: CostBreakdown,
}

#[derive(Debug, Clone)]
pub struct Solution {
    pub fingerings: Vec<Fingering>,
    pub cost: f32,
    pub explanations: Vec<NoteExplanation>,
    pub agreement: Option<(usize, usize)>,
    pub path: Vec<Step>,
}

impl Solution {
    pub fn finger_of(&self, note: NoteId) -> Option<Finger> {
        self.fingerings.iter().find(|f| f.note == note).map(|f| f.finger)
    }
}

const AGREEMENT_WEIGHT: f32 = 0.6;

const PASSAGE_WEIGHT: f32 = 2.5;

#[derive(Debug, Clone, Default)]
pub struct Agreement {
    votes: std::collections::HashMap<NoteId, [u8; 5]>,
    voters: u8,
    passages: std::collections::HashMap<NoteId, [f32; 5]>,
}

impl Agreement {
    pub fn across_published(
        score: &Score,
        options: &FingeringOptions,
        prior: Option<&dyn FingeringPrior>,
    ) -> Self {
        let ask = |set| {
            let mut single = options.clone();
            single.rule_set = set;
            finger_as_given(score, &single, prior)
        };
        let answers: Vec<Solution> = if inner_threads() {
            std::thread::scope(|scope| {
                let running: Vec<_> =
                    rules::PUBLISHED.into_iter().map(|set| scope.spawn(move || ask(set))).collect();
                running
                    .into_iter()
                    .map(|thread| thread.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
                    .collect()
            })
        } else {
            rules::PUBLISHED.into_iter().map(ask).collect()
        };

        let mut votes: std::collections::HashMap<NoteId, [u8; 5]> = std::collections::HashMap::new();
        for answer in &answers {
            for fingering in &answer.fingerings {
                votes.entry(fingering.note).or_default()[fingering.finger.index()] += 1;
            }
        }
        Self { votes, voters: answers.len() as u8, passages: Default::default() }
    }

    fn passage(&self, note: NoteId, finger: Finger) -> f32 {
        self.passages.get(&note).map_or(0.0, |share| PASSAGE_WEIGHT * share[finger.index()])
    }

    fn bonus(&self, note: NoteId, finger: Finger) -> f32 {
        if self.voters == 0 {
            return 0.0;
        }
        match self.votes.get(&note) {
            Some(counts) => AGREEMENT_WEIGHT * counts[finger.index()] as f32 / self.voters as f32,
            None => 0.0,
        }
    }

    pub fn tally(&self) -> (usize, usize) {
        let unanimous = self.votes.values().filter(|c| c.contains(&self.voters)).count();
        (unanimous, self.votes.len() - unanimous)
    }
}

pub fn finger_score_consensus(
    score: &Score,
    options: &FingeringOptions,
    prior: Option<&dyn FingeringPrior>,
) -> Solution {
    consensus_with(score, options, prior, Default::default())
}

fn consensus_with(
    score: &Score,
    options: &FingeringOptions,
    prior: Option<&dyn FingeringPrior>,
    passages: std::collections::HashMap<NoteId, [f32; 5]>,
) -> Solution {
    let mut agreement = Agreement::across_published(score, options, prior);
    agreement.passages = passages;
    let tally = agreement.tally();
    let mut combined = options.clone();
    combined.rule_set = RuleSet::Consensus;
    let mut solution = solve_score(score, &combined, prior, Some(&agreement));
    solution.agreement = Some(tally);
    solution
}

static INNER_THREADS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_inner_threads(on: bool) {
    INNER_THREADS.store(on, std::sync::atomic::Ordering::Relaxed);
}

fn inner_threads() -> bool {
    INNER_THREADS.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn finger_score(score: &Score, options: &FingeringOptions) -> Solution {
    finger_score_with_prior(score, options, None)
}

pub fn finger_score_with_prior(
    score: &Score,
    options: &FingeringOptions,
    prior: Option<&dyn FingeringPrior>,
) -> Solution {
    let pins = prior.map(|p| p.recall(score)).unwrap_or_default();
    let recalled;
    let score = if pins.is_empty() {
        score
    } else {
        tracing::info!(
            "{} of {} notes fingered as documented in a known piece",
            pins.len(),
            score.notes.len()
        );
        let mut pinned = score.clone();
        for (id, finger) in pins {
            let note = &mut pinned.notes[id.index()];
            note.given_finger = note.given_finger.or(Some(finger));
        }
        recalled = pinned;
        &recalled
    };
    let mut passages = prior.map(|p| p.suggest(score)).unwrap_or_default();
    passages.retain(|id, _| score.note(*id).given_finger.is_none());
    if !passages.is_empty() {
        tracing::info!(
            "{} of {} notes have a passage pianists commonly finger alike",
            passages.len(),
            score.notes.len()
        );
    }
    if options.rule_set == RuleSet::Consensus {
        return consensus_with(score, options, prior, passages);
    }
    let agreement = Agreement { passages, ..Default::default() };
    solve_score(score, options, prior, Some(&agreement))
}

fn finger_as_given(
    score: &Score,
    options: &FingeringOptions,
    prior: Option<&dyn FingeringPrior>,
) -> Solution {
    if options.rule_set == RuleSet::Consensus {
        return finger_score_consensus(score, options, prior);
    }
    solve_score(score, options, prior, None)
}

fn solve_score(
    score: &Score,
    options: &FingeringOptions,
    prior: Option<&dyn FingeringPrior>,
    agreement: Option<&Agreement>,
) -> Solution {
    let search = |hand| {
        let mut events = build_events(score, hand);
        if events.is_empty() {
            return None;
        }
        let reach = options.span_model.table().0[Finger::Thumb.index()][Finger::Little.index()].max_comf;
        hold_sustained(&mut events, score, reach);
        let mut solver = HandSolver::new(hand, options, prior, agreement);
        solver.find_scales(&events);
        Some(solver.solve(&events))
    };
    let solved: Vec<Option<Solution>> = if inner_threads() {
        std::thread::scope(|scope| {
            let running: Vec<_> =
                Hand::ALL.into_iter().map(|hand| scope.spawn(move || search(hand))).collect();
            running
                .into_iter()
                .map(|thread| thread.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
                .collect()
        })
    } else {
        Hand::ALL.into_iter().map(search).collect()
    };

    let mut fingerings = Vec::new();
    let mut explanations = Vec::new();
    let mut path = Vec::new();
    let mut cost = 0.0;
    for solved in solved.into_iter().flatten() {
        cost += solved.cost;
        fingerings.extend(solved.fingerings);
        explanations.extend(solved.explanations);
        path.extend(solved.path);
    }

    finger_the_twins(score, &mut fingerings);
    fingerings.sort_by_key(|f| f.note);
    explanations.sort_by_key(|e| e.note);
    Solution { fingerings, cost, explanations, agreement: None, path }
}

fn finger_the_twins(score: &Score, fingerings: &mut Vec<Fingering>) {
    let known: std::collections::HashMap<NoteId, Finger> =
        fingerings.iter().map(|f| (f.note, f.finger)).collect();
    let mut added = Vec::new();
    for note in &score.notes {
        if known.contains_key(&note.id) || note.tie.is_continuation() || note.hand.is_none() {
            continue;
        }
        let twin = score.notes.iter().find(|other| {
            other.midi == note.midi
                && other.hand == note.hand
                && other.onset == note.onset
                && known.contains_key(&other.id)
        });
        if let Some(finger) = twin.and_then(|twin| known.get(&twin.id)) {
            added.push(Fingering::new(note.id, *finger));
        }
    }
    fingerings.extend(added);
}

#[cfg(test)]
mod tests;
