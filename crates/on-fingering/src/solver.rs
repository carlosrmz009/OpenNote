use on_hand::{Finger, Hand, HandProfile};
use on_score::{Fingering, NoteId, Score};

use crate::biomech::{BiomechModel, BiomechWeights, Grip};
use crate::rules::{self, Placement, Rule, RuleScorer, RuleSet, RuleWeights, Trigram};
use crate::ruler::Ruler;
use crate::spans::SpanModel;

const WITHIN_CHORD_RULES: &[Rule] = &[
    Rule::Stretch,
    Rule::SmallSpan,
    Rule::LargeSpan,
    Rule::Impractical,
    Rule::ThreeToFour,
    Rule::FourOnBlack,
];

const PER_NOTE_RULES: &[Rule] = &[Rule::WeakFinger];

const SHIFTED_VOICE_RULES: &[Rule] =
    &[Rule::WeakFinger, Rule::Impractical, Rule::RepeatedFinger];

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
        Self {
            span_model: SpanModel::for_hand_size(profile.size()),
            profile,
            ..Default::default()
        }
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

#[derive(Debug, Clone)]
pub struct Event {
    pub notes: Vec<u8>,
    pub ids: Vec<NoteId>,
    pub struck: Vec<bool>,
    pub onset_seconds: f64,
    pub pinned: Vec<Option<Finger>>,
}

impl Event {
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }
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
        self.rules
            + self.posture
            + self.pattern
            + self.motion
            + self.agreement
            + self.prior
            + self.learned
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
        self.fingerings
            .iter()
            .find(|f| f.note == note)
            .map(|f| f.finger)
    }
}

const AGREEMENT_WEIGHT: f32 = 0.6;

#[derive(Debug, Clone, Default)]
pub struct Agreement {
    votes: std::collections::HashMap<NoteId, [u8; 5]>,
    voters: u8,
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
            finger_score_with_prior(score, &single, prior)
        };
        let answers: Vec<Solution> = if inner_threads() {
            std::thread::scope(|scope| {
                let running: Vec<_> = rules::PUBLISHED
                    .into_iter()
                    .map(|set| scope.spawn(move || ask(set)))
                    .collect();
                running.into_iter().filter_map(|thread| thread.join().ok()).collect()
            })
        } else {
            rules::PUBLISHED.into_iter().map(ask).collect()
        };

        let mut votes: std::collections::HashMap<NoteId, [u8; 5]> =
            std::collections::HashMap::new();
        for answer in &answers {
            for fingering in &answer.fingerings {
                votes.entry(fingering.note).or_default()[fingering.finger.index()] += 1;
            }
        }
        Self { votes, voters: answers.len() as u8 }
    }

    fn bonus(&self, note: NoteId, finger: Finger) -> f32 {
        if self.voters == 0 {
            return 0.0;
        }
        match self.votes.get(&note) {
            Some(counts) => {
                AGREEMENT_WEIGHT * counts[finger.index()] as f32 / self.voters as f32
            }
            None => 0.0,
        }
    }

    pub fn tally(&self) -> (usize, usize) {
        let unanimous = self
            .votes
            .values()
            .filter(|c| c.contains(&self.voters))
            .count();
        (unanimous, self.votes.len() - unanimous)
    }
}

pub fn finger_score_consensus(
    score: &Score,
    options: &FingeringOptions,
    prior: Option<&dyn FingeringPrior>,
) -> Solution {
    let agreement = Agreement::across_published(score, options, prior);
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
        let reach = options.span_model.table().0[Finger::Thumb.index()]
            [Finger::Little.index()]
        .max_comf;
        hold_sustained(&mut events, score, reach);
        let mut solver = HandSolver::new(hand, options, prior, agreement);
        solver.find_scales(&events);
        Some(solver.solve(&events))
    };
    let solved: Vec<Option<Solution>> = if inner_threads() {
        std::thread::scope(|scope| {
            let running: Vec<_> =
                Hand::ALL.into_iter().map(|hand| scope.spawn(move || search(hand))).collect();
            running.into_iter().map(|thread| thread.join().ok().flatten()).collect()
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

pub fn build_events(score: &Score, hand: Hand) -> Vec<Event> {
    let mut events: Vec<Event> = Vec::new();
    for chord in score.chords(hand) {
        let mut notes = Vec::new();
        let mut ids = Vec::new();
        let mut pinned = Vec::new();
        for id in &chord.notes {
            let note = score.note(*id);
            if note.tie.is_continuation() {
                continue;
            }
            if notes.contains(&note.midi) {
                continue;
            }
            notes.push(note.midi);
            ids.push(*id);
            pinned.push(note.given_finger);
        }
        if notes.is_empty() {
            continue;
        }
        let struck = vec![true; notes.len()];
        events.push(Event {
            notes,
            ids,
            struck,
            onset_seconds: chord.onset_seconds,
            pinned,
        });
    }
    events
}

const FINGERS: usize = 5;

const SUBSTITUTION_COST: f32 = 800.0;

const RELEASE_SECONDS: f64 = crate::playability::CHORD_SECONDS;

fn hold_sustained(events: &mut [Event], score: &Score, reach: i32) {
    let mut sounding: Vec<Sounding> = Vec::new();
    for index in 0..events.len() {
        let now = events[index].onset_seconds;
        sounding.retain(|s| s.until > now + RELEASE_SECONDS);
        let restruck: Vec<u8> = events[index].notes.clone();
        sounding.retain(|s| !restruck.contains(&s.midi) || events[index].ids.contains(&s.id));

        let struck: Vec<(NoteId, u8)> = events[index]
            .ids
            .iter()
            .zip(&events[index].notes)
            .map(|(id, midi)| (*id, *midi))
            .collect();
        let (struck_low, struck_high) = struck
            .iter()
            .fold((u8::MAX, u8::MIN), |(lo, hi), (_, m)| (lo.min(*m), hi.max(*m)));

        sounding.retain(|s| i32::from(s.high.max(struck_high) - s.low.min(struck_low)) <= reach);
        for s in &mut sounding {
            s.low = s.low.min(struck_low);
            s.high = s.high.max(struck_high);
        }

        let mut held: Vec<(NoteId, u8)> = sounding
            .iter()
            .rev()
            .filter(|s| !struck.iter().any(|(other, _)| *other == s.id))
            .map(|s| (s.id, s.midi))
            .collect();

        let (mut low, mut high) = struck
            .iter()
            .fold((u8::MAX, u8::MIN), |(lo, hi), (_, m)| (lo.min(*m), hi.max(*m)));
        let mut room = FINGERS.saturating_sub(struck.len());
        held.retain(|(_, midi)| {
            let (would_low, would_high) = (low.min(*midi), high.max(*midi));
            if room == 0 || i32::from(would_high - would_low) > reach {
                return false;
            }
            low = would_low;
            high = would_high;
            room -= 1;
            true
        });

        if !held.is_empty() {
            let event = &mut events[index];
            for (id, midi) in held {
                event.ids.push(id);
                event.notes.push(midi);
                event.struck.push(false);
                event.pinned.push(score.note(id).given_finger);
            }
            let mut order: Vec<usize> = (0..event.notes.len()).collect();
            order.sort_by_key(|i| event.notes[*i]);
            event.notes = order.iter().map(|i| event.notes[*i]).collect();
            event.ids = order.iter().map(|i| event.ids[*i]).collect();
            event.struck = order.iter().map(|i| event.struck[*i]).collect();
            event.pinned = order.iter().map(|i| event.pinned[*i]).collect();
        }

        for (id, midi) in struck {
            sounding.push(Sounding {
                id,
                midi,
                until: score.release_seconds(id),
                low: struck_low,
                high: struck_high,
            });
        }
    }
}

struct Sounding {
    id: NoteId,
    midi: u8,
    until: f64,
    low: u8,
    high: u8,
}

struct HandSolver<'a> {
    hand: Hand,
    rules: RuleScorer,
    biomech: BiomechModel,
    options: &'a FingeringOptions,
    prior: Option<&'a dyn FingeringPrior>,
    agreement: Option<&'a Agreement>,
    scales: std::collections::HashMap<usize, crate::scales::Taught>,
}

impl<'a> HandSolver<'a> {
    fn new(
        hand: Hand,
        options: &'a FingeringOptions,
        prior: Option<&'a dyn FingeringPrior>,
        agreement: Option<&'a Agreement>,
    ) -> Self {
        Self {
            hand,
            rules: RuleScorer::new(
                options.rule_set,
                options.rule_weights,
                options.span_model.table(),
                options.ruler,
            ),
            biomech: BiomechModel::new(options.profile.clone(), hand, options.biomech),
            options,
            prior,
            agreement,
            scales: std::collections::HashMap::new(),
        }
    }

    fn find_scales(&mut self, events: &[Event]) {
        let line: Vec<Option<u8>> = events
            .iter()
            .map(|e| if e.len() == 1 { Some(e.notes[0]) } else { None })
            .collect();
        let onsets: Vec<f64> = events.iter().map(|e| e.onset_seconds).collect();
        self.scales = crate::scales::scale_fingerings(self.hand, &line, &onsets);
    }

    fn candidates(&self, event: &Event) -> Vec<Candidate> {
        let k = event.len();
        let mut out = Vec::new();
        if k == 0 || k > 5 {
            if k > 5 {
                out.push(Candidate { fingers: spread_fingers(self.hand, k) });
            }
            return out;
        }

        let mut chosen = Vec::with_capacity(k);
        self.enumerate(event, 0, &mut chosen, &mut out);
        if out.is_empty() {
            out.push(Candidate { fingers: spread_fingers(self.hand, k) });
        }
        out
    }

    fn enumerate(
        &self,
        event: &Event,
        index: usize,
        chosen: &mut Vec<Finger>,
        out: &mut Vec<Candidate>,
    ) {
        if index == event.len() {
            out.push(Candidate { fingers: chosen.clone() });
            return;
        }
        let remaining = event.len() - index - 1;
        for f in Finger::ALL {
            if let Some(required) = event.pinned[index] {
                if f != required {
                    continue;
                }
            }
            if let Some(previous) = chosen.last() {
                let ordered = match self.hand {
                    Hand::Right => f.index() > previous.index(),
                    Hand::Left => f.index() < previous.index(),
                };
                if !ordered {
                    continue;
                }
            }
            let room = match self.hand {
                Hand::Right => 4 - f.index() >= remaining,
                Hand::Left => f.index() >= remaining,
            };
            if !room {
                continue;
            }
            chosen.push(f);
            self.enumerate(event, index + 1, chosen, out);
            chosen.pop();
        }
    }

    fn grip(&self, event: &Event, candidate: &Candidate) -> Grip {
        Grip::new(
            event
                .notes
                .iter()
                .copied()
                .zip(candidate.fingers.iter().copied())
                .collect(),
        )
    }

    fn outer(&self, event: &Event, candidate: &Candidate) -> (Placement, Placement) {
        let last = event.len() - 1;
        (
            Placement::new(event.notes[0], candidate.fingers[0]),
            Placement::new(event.notes[last], candidate.fingers[last]),
        )
    }

    fn baseline_strain(&self, event: &Event, candidates: &[Candidate]) -> f32 {
        candidates
            .iter()
            .map(|c| self.biomech.grip_outcome(&self.grip(event, c)).strain)
            .fold(f32::INFINITY, f32::min)
    }

    fn event_cost(&self, index: usize, event: &Event, candidate: &Candidate, baseline: f32) -> f32 {
        self.event_breakdown(index, event, candidate, baseline).total()
    }

    fn event_breakdown(
        &self,
        index: usize,
        event: &Event,
        candidate: &Candidate,
        baseline: f32,
    ) -> CostBreakdown {
        let mut rules = 0.0;
        let mut agreed = 0.0;
        for i in 0..event.len() {
            let here = Placement::new(event.notes[i], candidate.fingers[i]);
            if let Some(votes) = self.agreement {
                agreed -= votes.bonus(event.ids[i], candidate.fingers[i]);
            }
            rules += self
                .rules
                .score_with(
                    &Trigram { hand: self.hand, prev: None, current: here, next: None },
                    PER_NOTE_RULES,
                )
                .total();
            if i > 0 {
                let below = Placement::new(event.notes[i - 1], candidate.fingers[i - 1]);
                rules += self
                    .rules
                    .score_with(
                        &Trigram {
                            hand: self.hand,
                            prev: Some(below),
                            current: here,
                            next: None,
                        },
                        WITHIN_CHORD_RULES,
                    )
                    .total();
            }
        }
        let mut pattern =
            crate::patterns::chord_bonus(self.hand, &event.notes, &candidate.fingers);
        if let Some(taught) = self.scales.get(&index) {
            if candidate.fingers.first() == Some(&taught.finger) {
                pattern += taught.bonus;
            }
        }
        let pattern = self.options.pattern_scale * pattern;

        let outcome = self.biomech.grip_outcome(&self.grip(event, candidate));
        let mut posture = self.biomech.weights().posture * (outcome.strain - baseline).max(0.0);
        if !outcome.reachable {
            posture += crate::biomech::UNREACHABLE_PENALTY + outcome.shortfall_mm;
        }
        let learned = self.prior.map_or(0.0, |model| {
            model
                .chord_cost(self.hand, &event.notes, &candidate.fingers)
                .clamp(-crate::learned::LIMIT, crate::learned::LIMIT)
        });
        CostBreakdown {
            rules: self.options.rule_scale * rules,
            posture,
            pattern: -pattern,
            motion: 0.0,
            agreement: agreed,
            prior: 0.0,
            learned,
        }
    }

    fn window_cost(
        &self,
        prev: Option<(&Event, &Candidate)>,
        current: (&Event, &Candidate),
        next: Option<(&Event, &Candidate)>,
    ) -> f32 {
        let (low, high) = self.outer(current.0, current.1);
        let prev_outer = prev.map(|(e, c)| self.outer(e, c));
        let next_outer = next.map(|(e, c)| self.outer(e, c));

        let shifting =
            current.0.len() > 1 && prev.is_some_and(|(event, _)| event.len() > 1);

        let mut total = 0.0;
        for (side, current) in [(0usize, low), (1usize, high)] {
            let t = Trigram {
                hand: self.hand,
                prev: prev_outer.map(|o| if side == 0 { o.0 } else { o.1 }),
                current,
                next: next_outer.map(|o| if side == 0 { o.0 } else { o.1 }),
            };
            let held = t.prev.is_some_and(|p| p.finger == t.current.finger);
            let skip = if shifting && held { SHIFTED_VOICE_RULES } else { PER_NOTE_RULES };
            total += 0.5 * self.rules.score_melodic(&t, skip).total();
        }
        self.options.rule_scale * total
    }

    fn prior_cost(
        &self,
        prev2: Option<(&Event, &Candidate)>,
        prev: Option<(&Event, &Candidate)>,
        current: (&Event, &Candidate),
    ) -> f32 {
        let Some(prior) = self.prior else { return 0.0 };
        let learned = match (prev2, prev) {
            (Some(a), Some(b)) => prior
                .trigram_cost(
                    self.hand,
                    (&a.0.notes, &a.1.fingers),
                    (&b.0.notes, &b.1.fingers),
                    (&current.0.notes, &current.1.fingers),
                )
                .clamp(-crate::learned::LIMIT, crate::learned::LIMIT),
            _ => 0.0,
        };
        if self.options.prior_scale == 0.0 {
            return learned;
        }
        let top = |pair: Option<(&Event, &Candidate)>| pair.map(|(e, c)| self.outer(e, c).1);
        let (_, here) = self.outer(current.0, current.1);
        let logp = prior.log_probability(self.hand, top(prev2), top(prev), here);
        learned - self.options.prior_scale * logp
    }

    fn move_cost(&self, from: (&Event, &Candidate), to: (&Event, &Candidate)) -> f32 {
        let seconds = to.0.onset_seconds - from.0.onset_seconds;
        let travel = self.biomech.transition_cost(
            &self.grip(from.0, from.1),
            &self.grip(to.0, to.1),
            seconds,
        );
        let learned = self.prior.map_or(0.0, |model| {
            model
                .step_cost(
                    self.hand,
                    (&from.0.notes, &from.1.fingers),
                    (&to.0.notes, &to.1.fingers),
                    seconds,
                )
                .clamp(-crate::learned::LIMIT, crate::learned::LIMIT)
        });
        travel + SUBSTITUTION_COST * self.substitutions(from, to) as f32 + learned
    }

    fn substitutions(&self, from: (&Event, &Candidate), to: (&Event, &Candidate)) -> usize {
        to.0
            .ids
            .iter()
            .enumerate()
            .filter(|(slot, id)| {
                !to.0.struck[*slot]
                    && from
                        .0
                        .ids
                        .iter()
                        .position(|earlier| earlier == *id)
                        .is_some_and(|was| from.1.fingers[was] != to.1.fingers[*slot])
            })
            .count()
    }

    fn solve(&self, events: &[Event]) -> Solution {
        let all: Vec<Vec<Candidate>> = events.iter().map(|e| self.candidates(e)).collect();
        let baselines: Vec<f32> = events
            .iter()
            .zip(&all)
            .map(|(e, c)| self.baseline_strain(e, c))
            .collect();

        let n = events.len();
        if n == 1 {
            return self.single_event(&events[0], &all[0], baselines[0]);
        }

        let mut best: Vec<Vec<f32>> = Vec::with_capacity(n);
        let mut from: Vec<Vec<usize>> = Vec::with_capacity(n);

        let (c0, c1) = (all[0].len(), all[1].len());
        let mut row = vec![f32::INFINITY; c0 * c1];
        for a in 0..c0 {
            let pa = (&events[0], &all[0][a]);
            let base = self.event_cost(0, &events[0], &all[0][a], baselines[0]);
            for b in 0..c1 {
                let pb = (&events[1], &all[1][b]);
                let mut total = base;
                total += self.event_cost(1, &events[1], &all[1][b], baselines[1]);
                total += self.move_cost(pa, pb);
                total += self.window_cost(None, pa, Some(pb));
                total += self.prior_cost(None, None, pa);
                total += self.prior_cost(None, Some(pa), pb);
                row[a * c1 + b] = total;
            }
        }
        best.push(vec![]);
        best.push(row);
        from.push(vec![]);
        from.push(vec![0; c0 * c1]);

        for i in 2..n {
            let (ca, cb, cc) = (all[i - 2].len(), all[i - 1].len(), all[i].len());
            let mut row = vec![f32::INFINITY; cb * cc];
            let mut back = vec![0usize; cb * cc];
            for b in 0..cb {
                let pb = (&events[i - 1], &all[i - 1][b]);
                for c in 0..cc {
                    let pc = (&events[i], &all[i][c]);
                    let step = self.event_cost(i, &events[i], &all[i][c], baselines[i]) + self.move_cost(pb, pc);
                    for a in 0..ca {
                        let previous = best[i - 1][a * cb + b];
                        if !previous.is_finite() {
                            continue;
                        }
                        let pa = (&events[i - 2], &all[i - 2][a]);
                        let total = previous
                            + step
                            + self.window_cost(Some(pa), pb, Some(pc))
                            + self.prior_cost(Some(pa), Some(pb), pc);
                        let slot = b * cc + c;
                        if total < row[slot] {
                            row[slot] = total;
                            back[slot] = a;
                        }
                    }
                }
            }
            best.push(row);
            from.push(back);
        }

        let (cb, cc) = (all[n - 2].len(), all[n - 1].len());
        let mut best_slot = 0;
        let mut best_cost = f32::INFINITY;
        for b in 0..cb {
            for c in 0..cc {
                let slot = b * cc + c;
                let base = best[n - 1][slot];
                if !base.is_finite() {
                    continue;
                }
                let pb = (&events[n - 2], &all[n - 2][b]);
                let pc = (&events[n - 1], &all[n - 1][c]);
                let total = base + self.window_cost(Some(pb), pc, None);
                if total < best_cost {
                    best_cost = total;
                    best_slot = slot;
                }
            }
        }

        let mut chosen = vec![0usize; n];
        let mut b = best_slot / cc;
        let mut c = best_slot % cc;
        chosen[n - 1] = c;
        chosen[n - 2] = b;
        for i in (2..n).rev() {
            let a = from[i][b * all[i].len() + c];
            chosen[i - 2] = a;
            c = b;
            b = a;
        }

        self.assemble(events, &all, &baselines, &chosen, best_cost)
    }

    fn single_event(&self, event: &Event, candidates: &[Candidate], baseline: f32) -> Solution {
        let mut best = 0;
        let mut best_cost = f32::INFINITY;
        for (i, candidate) in candidates.iter().enumerate() {
            let cost = self.event_cost(0, event, candidate, baseline)
                + self.window_cost(None, (event, candidate), None);
            if cost < best_cost {
                best_cost = cost;
                best = i;
            }
        }
        let events = std::slice::from_ref(event);
        let all = vec![candidates.to_vec()];
        self.assemble(events, &all, &[baseline], &[best], best_cost)
    }

    fn neighbourhood(
        &self,
        events: &[Event],
        all: &[Vec<Candidate>],
        baselines: &[f32],
        chosen: &[usize],
        index: usize,
        here: &Candidate,
    ) -> f32 {
        let at = |j: isize| on_path(events, all, chosen, index, here, j);
        let i = index as isize;
        let centre = (&events[index], here);

        let mut total = self.event_cost(index, &events[index], here, baselines[index]);
        if let Some(prev) = at(i - 1) {
            total += self.move_cost(prev, centre);
        }
        if let Some(next) = at(i + 1) {
            total += self.move_cost(centre, next);
        }
        for j in [i - 1, i, i + 1] {
            if let Some(middle) = at(j) {
                total += self.window_cost(at(j - 1), middle, at(j + 1));
            }
        }
        if events.len() >= 2 {
            for j in [i, i + 1, i + 2] {
                if let Some(current) = at(j) {
                    total += self.prior_cost(at(j - 2), at(j - 1), current);
                }
            }
        }
        total
    }

    fn assemble(
        &self,
        events: &[Event],
        all: &[Vec<Candidate>],
        baselines: &[f32],
        chosen: &[usize],
        cost: f32,
    ) -> Solution {
        let mut fingerings = Vec::new();
        let mut explanations = Vec::new();
        let mut path = Vec::with_capacity(events.len());

        for (i, event) in events.iter().enumerate() {
            let candidate = &all[i][chosen[i]];
            path.push(Step {
                hand: self.hand,
                notes: event.notes.clone(),
                fingers: candidate.fingers.clone(),
                onset_seconds: event.onset_seconds,
            });
            let grip = self.grip(event, candidate);
            let outcome = self.biomech.grip_outcome(&grip);

            let prev = i.checked_sub(1).map(|j| (&events[j], &all[j][chosen[j]]));
            let next = events
                .get(i + 1)
                .map(|e| (e, &all[i + 1][chosen[i + 1]]));
            let (low, high) = self.outer(event, candidate);
            let window = Trigram {
                hand: self.hand,
                prev: prev.map(|(e, c)| self.outer(e, c).1),
                current: high,
                next: next.map(|(e, c)| self.outer(e, c).1),
            };
            let shifting = event.len() > 1 && prev.is_some_and(|(e, _)| e.len() > 1);
            let held = window.prev.is_some_and(|p| p.finger == window.current.finger);
            let skip = if shifting && held { SHIFTED_VOICE_RULES } else { PER_NOTE_RULES };
            let fired = self.rules.score_melodic(&window, skip).fired();
            let _ = low;

            let mut breakdown = self.event_breakdown(i, event, candidate, baselines[i]);
            breakdown.rules += self.window_cost(prev, (event, candidate), next);
            breakdown.motion = prev
                .map(|p| self.move_cost(p, (event, candidate)))
                .unwrap_or(0.0);
            if events.len() >= 2 {
                let before = |back: usize| {
                    i.checked_sub(back).map(|j| (&events[j], &all[j][chosen[j]]))
                };
                breakdown.prior = self.prior_cost(before(2), before(1), (event, candidate));
            }

            let local =
                |c: &Candidate| self.neighbourhood(events, all, baselines, chosen, i, c);
            let chosen_cost = local(candidate);
            let mut margin = f32::INFINITY;
            for (j, other) in all[i].iter().enumerate() {
                if j == chosen[i] {
                    continue;
                }
                margin = margin.min(local(other) - chosen_cost);
            }
            if !margin.is_finite() {
                margin = 0.0;
            }

            for (slot, id) in event.ids.iter().enumerate() {
                if !event.struck[slot] {
                    continue;
                }
                let finger = candidate.fingers[slot];
                fingerings.push(Fingering::new(*id, finger));
                explanations.push(NoteExplanation {
                    note: *id,
                    finger,
                    rules: fired.clone(),
                    posture_strain: outcome.strain,
                    reachable: outcome.reachable,
                    margin,
                    cost: breakdown,
                });
            }
        }

        Solution { fingerings, cost, explanations, agreement: None, path }
    }
}

fn on_path<'a>(
    events: &'a [Event],
    all: &'a [Vec<Candidate>],
    chosen: &[usize],
    index: usize,
    here: &'a Candidate,
    j: isize,
) -> Option<(&'a Event, &'a Candidate)> {
    let j = usize::try_from(j).ok().filter(|j| *j < events.len())?;
    Some((&events[j], if j == index { here } else { &all[j][chosen[j]] }))
}

fn spread_fingers(hand: Hand, k: usize) -> Vec<Finger> {
    (0..k)
        .map(|i| {
            let slot = (i * 4) / (k - 1).max(1);
            let slot = slot.min(4);
            match hand {
                Hand::Right => Finger::from_index(slot),
                Hand::Left => Finger::from_index(4 - slot),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use on_score::{Note, NoteId, Score, SourceRef, TieState, TICKS_PER_QUARTER};

    fn melody(pitches: &[u8], hand: Hand, beats_per_note: f64) -> Score {
        let mut score = Score::default();
        let step = (TICKS_PER_QUARTER as f64 * beats_per_note) as i64;
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

    #[test]
    fn a_key_struck_again_is_not_also_held() {
        let mut score = melody(&[60, 60, 64], Hand::Right, 1.0);
        score.notes[0].duration *= 3;
        score.finalise();
        let mut events = build_events(&score, Hand::Right);
        hold_sustained(&mut events, &score, 14);
        for event in &events {
            let mut keys = event.notes.clone();
            keys.dedup();
            assert_eq!(keys, event.notes, "the same key twice in one chord: {:?}", event.notes);
        }
    }

    fn ticks_for(score: &Score, seconds: f64) -> i64 {
        let note = &score.notes[1];
        (seconds * note.onset as f64 / note.onset_seconds).round() as i64
    }

    #[test]
    fn a_note_let_go_as_the_next_lands_is_not_held() {
        let line = melody(&[60, 62, 64, 65, 67], Hand::Right, 0.5);
        for (overlap, held) in [(0.0, false), (0.002, false), (0.020, false), (0.100, true)] {
            let mut score = line.clone();
            let extra = ticks_for(&score, overlap);
            for note in &mut score.notes {
                note.duration += extra;
            }
            score.finalise();
            let mut events = build_events(&score, Hand::Right);
            hold_sustained(&mut events, &score, 14);
            let widest = events.iter().map(Event::len).max().unwrap_or(0);
            assert_eq!(widest > 1, held, "a note running {overlap} s past the next onset");
        }
    }

    #[test]
    fn a_few_milliseconds_of_timing_change_no_finger() {
        let wobble = [0.009, -0.008, 0.004, -0.010, 0.007, -0.003, 0.010, -0.006];
        for score in passages() {
            let plain = fingers_of(&score, &finger_score(&score, &FingeringOptions::default()));
            let mut moved = score.clone();
            let mut onsets: Vec<i64> = moved.notes.iter().map(|n| n.onset).collect();
            onsets.sort_unstable();
            onsets.dedup();
            for note in &mut moved.notes {
                let chord = onsets.binary_search(&note.onset).unwrap();
                note.onset += ticks_for(&score, wobble[chord % wobble.len()]);
            }
            moved.finalise();
            let wobbled = fingers_of(&moved, &finger_score(&moved, &FingeringOptions::default()));
            assert_eq!(plain, wobbled, "{:?}", score.notes.iter().map(|n| n.midi).collect::<Vec<_>>());
        }
    }

    fn passages() -> Vec<Score> {
        vec![
            melody(&[60, 62, 64, 65, 67, 69, 71, 72, 71, 69, 67, 65, 64, 62, 60], Hand::Right, 0.5),
            melody(&[60, 67, 64, 72, 69, 76, 71, 79, 72], Hand::Right, 0.5),
            melody(&[48, 43, 45, 41, 43, 36, 40, 45, 48], Hand::Left, 0.5),
            melody(&[64, 63, 64, 63, 64, 59, 62, 60, 57], Hand::Right, 0.25),
            octaves(&[48, 50, 52, 53, 55, 53, 52, 50, 48], Hand::Left, 0.5),
        ]
    }

    fn trained_prior() -> crate::NgramPrior {
        let mut prior = crate::NgramPrior::new();
        let finger = |n: u8| Finger::from_number(n).expect("1..=5");
        for (hand, line) in [
            (Hand::Right, [(60, 1), (62, 2), (64, 3), (65, 1), (67, 2), (69, 3), (71, 4), (72, 5)]),
            (Hand::Left, [(48, 5), (50, 4), (52, 3), (53, 2), (55, 1), (57, 3), (59, 2), (60, 1)]),
        ] {
            let placements: Vec<Placement> =
                line.iter().map(|(midi, f)| Placement::new(*midi, finger(*f))).collect();
            prior.observe(hand, &placements);
        }
        prior
    }

    fn runs(score: &Score) -> Vec<Solution> {
        let plain = FingeringOptions::default();
        let prior = trained_prior();
        let with_model = FingeringOptions { prior_scale: 1.5, ..FingeringOptions::default() };
        vec![
            finger_score(score, &plain),
            finger_score_with_prior(score, &with_model, Some(&prior)),
        ]
    }

    #[test]
    fn no_alternative_is_ever_cheaper_than_what_was_chosen() {
        for score in passages() {
            for solution in runs(&score) {
                for e in &solution.explanations {
                    assert!(
                        e.margin >= -1e-3,
                        "note {:?} was given {:?}, and swapping it would have been cheaper by {}",
                        e.note,
                        e.finger,
                        -e.margin
                    );
                }
            }
        }
    }

    #[test]
    fn the_explained_costs_add_up_to_the_total() {
        for score in passages().into_iter().take(4) {
            for solution in runs(&score) {
                let explained: f32 = solution.explanations.iter().map(|e| e.cost.total()).sum();
                let tolerance = 1e-3 * solution.cost.abs().max(1.0);
                assert!(
                    (explained - solution.cost).abs() <= tolerance,
                    "the notes explain {explained} of a total of {}",
                    solution.cost
                );
            }
        }
    }

    fn octaves(pitches: &[u8], hand: Hand, beats_per_note: f64) -> Score {
        let mut score = melody(pitches, hand, beats_per_note);
        let upper: Vec<Note> = score
            .notes
            .iter()
            .enumerate()
            .map(|(i, n)| Note {
                id: NoteId((pitches.len() + i) as u32),
                midi: n.midi + 12,
                ..n.clone()
            })
            .collect();
        score.notes.extend(upper);
        score.finalise();
        score
    }

    fn fingers_of(score: &Score, solution: &Solution) -> Vec<u8> {
        let mut out: Vec<_> = score
            .notes
            .iter()
            .filter_map(|n| solution.finger_of(n.id).map(|f| (n.onset, f.number())))
            .collect();
        out.sort_by_key(|(t, _)| *t);
        out.into_iter().map(|(_, f)| f).collect()
    }

    const C_MAJOR_UP: [u8; 8] = [60, 62, 64, 65, 67, 69, 71, 72];

    #[test]
    fn candidates_respect_finger_order_within_a_chord() {
        let options = FingeringOptions::default();
        for hand in Hand::ALL {
            let solver = HandSolver::new(hand, &options, None, None);
            let event = Event {
                notes: vec![60, 64, 67],
                ids: vec![NoteId(0), NoteId(1), NoteId(2)],
                struck: vec![true; 3],
                onset_seconds: 0.0,
                pinned: vec![None; 3],
            };
            let candidates = solver.candidates(&event);
            assert_eq!(candidates.len(), 10, "{hand:?}");
            for c in &candidates {
                let indices: Vec<_> = c.fingers.iter().map(|f| f.index()).collect();
                let ordered = match hand {
                    Hand::Right => indices.windows(2).all(|w| w[0] < w[1]),
                    Hand::Left => indices.windows(2).all(|w| w[0] > w[1]),
                };
                assert!(ordered, "{hand:?} produced tangled fingers {indices:?}");
            }
        }
    }

    #[test]
    fn a_pinned_finger_is_obeyed() {
        let options = FingeringOptions::default();
        let solver = HandSolver::new(Hand::Right, &options, None, None);
        let event = Event {
            notes: vec![60, 64],
            ids: vec![NoteId(0), NoteId(1)],
            struck: vec![true; 2],
            onset_seconds: 0.0,
            pinned: vec![Some(Finger::Index), None],
        };
        for c in solver.candidates(&event) {
            assert_eq!(c.fingers[0], Finger::Index);
        }
    }

    #[test]
    fn the_right_hand_fingers_a_rising_c_major_scale_the_way_a_teacher_would() {
        let score = melody(&C_MAJOR_UP, Hand::Right, 0.5);
        let solution = finger_score(&score, &FingeringOptions::default());
        assert_eq!(fingers_of(&score, &solution), vec![1, 2, 3, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn the_left_hand_fingers_a_rising_c_major_scale_the_way_a_teacher_would() {
        let score = melody(&C_MAJOR_UP, Hand::Left, 0.5);
        let solution = finger_score(&score, &FingeringOptions::default());
        assert_eq!(fingers_of(&score, &solution), vec![5, 4, 3, 2, 1, 3, 2, 1]);
    }

    #[test]
    fn a_descending_scale_mirrors_the_ascending_one() {
        let mut descending = C_MAJOR_UP;
        descending.reverse();
        let score = melody(&descending, Hand::Right, 0.5);
        let solution = finger_score(&score, &FingeringOptions::default());
        assert_eq!(fingers_of(&score, &solution), vec![5, 4, 3, 2, 1, 3, 2, 1]);
    }

    #[test]
    fn a_five_note_run_within_the_hand_needs_no_thumb_crossing() {
        let score = melody(&[60, 62, 64, 65, 67], Hand::Right, 0.5);
        let solution = finger_score(&score, &FingeringOptions::default());
        assert_eq!(fingers_of(&score, &solution), vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_triad_is_fingered_one_three_five() {
        let mut score = Score::default();
        for (i, midi) in [60u8, 64, 67].iter().enumerate() {
            score.notes.push(Note {
                id: NoteId(i as u32),
                midi: *midi,
                onset: 0,
                duration: TICKS_PER_QUARTER as i64,
                onset_seconds: 0.0,
                duration_seconds: 0.0,
                staff: None,
                voice: None,
                hand: Some(Hand::Right),
                tie: TieState::default(),
                grace: false,
                chord: i > 0,
                velocity: 72,
                given_finger: None,
                source: SourceRef::Midi { track: 0, event: i },
            });
        }
        score.finalise();
        let solution = finger_score(&score, &FingeringOptions::default());
        let mut fingers: Vec<_> = solution.fingerings.iter().map(|f| f.finger.number()).collect();
        fingers.sort_unstable();
        assert_eq!(fingers, vec![1, 3, 5]);
    }

    #[test]
    fn tied_notes_are_not_fingered_twice() {
        let mut score = melody(&[60, 60], Hand::Right, 1.0);
        score.notes[0].tie.start = true;
        score.notes[1].tie.stop = true;
        let solution = finger_score(&score, &FingeringOptions::default());
        assert_eq!(solution.fingerings.len(), 1, "the tie continuation got its own finger");
    }

    #[test]
    fn an_editorial_fingering_is_kept_and_the_rest_fits_around_it() {
        let mut score = melody(&C_MAJOR_UP, Hand::Right, 0.5);
        score.notes[0].given_finger = Some(Finger::Index);
        let solution = finger_score(&score, &FingeringOptions::default());
        let fingers = fingers_of(&score, &solution);
        assert_eq!(fingers[0], 2, "the editorial fingering was overwritten");
        assert_eq!(fingers.len(), C_MAJOR_UP.len());
    }

    #[test]
    fn every_note_gets_exactly_one_finger() {
        let score = melody(&[60, 63, 66, 70, 73, 68, 61, 59], Hand::Right, 0.25);
        let solution = finger_score(&score, &FingeringOptions::default());
        assert_eq!(solution.fingerings.len(), score.notes.len());
        assert_eq!(solution.explanations.len(), score.notes.len());
        let mut ids: Vec<_> = solution.fingerings.iter().map(|f| f.note).collect();
        ids.dedup();
        assert_eq!(ids.len(), score.notes.len());
    }

    #[test]
    fn explanations_name_the_rules_that_fired() {
        let score = melody(&[65, 66, 67], Hand::Right, 0.5);
        let solution = finger_score(&score, &FingeringOptions::default());
        assert!(solution.explanations.iter().all(|e| e.reachable));
        assert!(solution.explanations.iter().all(|e| e.margin >= 0.0));
    }

    #[test]
    fn tempo_changes_the_answer() {
        let wide = [60u8, 84, 60, 84, 60, 84];
        let slow = finger_score(&melody(&wide, Hand::Right, 2.0), &FingeringOptions::default());
        let fast = finger_score(&melody(&wide, Hand::Right, 0.1), &FingeringOptions::default());
        assert_eq!(slow.fingerings.len(), wide.len());
        assert_eq!(fast.fingerings.len(), wide.len());
        assert!(fast.cost > slow.cost, "slow {} fast {}", slow.cost, fast.cost);
    }

    #[test]
    fn a_two_octave_arpeggio_is_fingered_the_way_the_charts_print_it() {
        for (tonic, register) in [(0u8, 60u8), (5, 60), (7, 60), (3, 60)] {
            let base = tonic + register;
            let up: Vec<u8> =
                [0u8, 4, 7, 12, 16, 19, 24].iter().map(|step| base + step).collect();
            let mut pitches = up.clone();
            pitches.extend(up.iter().rev().skip(1));

            let score = melody(&pitches, Hand::Right, 0.5);
            let right = fingers_of(&score, &finger_score(&score, &FingeringOptions::default()));
            assert_eq!(
                right,
                vec![1, 2, 3, 1, 2, 3, 5, 3, 2, 1, 3, 2, 1],
                "right hand, arpeggio on {base}"
            );

            let score = melody(&pitches, Hand::Left, 0.5);
            let left = fingers_of(&score, &finger_score(&score, &FingeringOptions::default()));
            let want = if tonic == 3 {
                vec![5, 3, 2, 1, 3, 2, 1, 2, 3, 1, 2, 3, 5]
            } else {
                vec![5, 4, 2, 1, 4, 2, 1, 2, 4, 1, 2, 4, 5]
            };
            assert_eq!(left, want, "left hand, arpeggio on {base}");
        }
    }

    #[test]
    fn a_chromatic_run_takes_the_third_finger_on_every_black_key() {
        for (hand, seconds) in [(Hand::Right, [0u8, 5]), (Hand::Left, [4, 11])] {
            let score = melody(&(60..=72).collect::<Vec<u8>>(), hand, 0.25);
            let solution = finger_score_consensus(&score, &FingeringOptions::default(), None);
            let fingers = fingers_of(&score, &solution);
            for (i, midi) in (60u8..=72).enumerate().take(12).skip(1) {
                let wanted = match midi % 12 {
                    1 | 3 | 6 | 8 | 10 => 3,
                    p if seconds.contains(&p) => 2,
                    _ => 1,
                };
                assert_eq!(fingers[i], wanted, "{hand:?} at midi {midi}");
            }
        }
    }

    #[test]
    fn a_run_of_octaves_is_the_thumb_and_the_little_finger_all_the_way() {
        let score = octaves(&[72, 74, 76, 77, 79, 81], Hand::Right, 0.5);
        let options = FingeringOptions::default();
        let solution = finger_score_consensus(&score, &options, None);

        let mut by_onset: std::collections::BTreeMap<i64, Vec<u8>> = Default::default();
        for note in &score.notes {
            if let Some(finger) = solution.finger_of(note.id) {
                by_onset.entry(note.onset).or_default().push(finger.number());
            }
        }
        for (onset, mut shape) in by_onset {
            shape.sort_unstable();
            assert_eq!(shape, vec![1, 5], "the octave at tick {onset}");
        }
    }

    #[test]
    fn the_same_score_is_fingered_the_same_way_every_time() {
        let score = melody(&[60, 64, 67, 72, 71, 69, 65, 62, 60], Hand::Right, 0.25);
        let options = FingeringOptions::default();
        let first = fingers_of(&score, &finger_score_consensus(&score, &options, None));
        for _ in 0..8 {
            let again = fingers_of(&score, &finger_score_consensus(&score, &options, None));
            assert_eq!(first, again);
        }

        let agreement = Agreement::across_published(&score, &options, None);
        assert_eq!(agreement.voters, 4, "all four sets should have been heard from");
    }

    #[test]
    fn asking_for_the_default_gets_the_consensus_and_not_a_lookalike() {
        let score = melody(&[63, 65, 69, 66, 62, 65, 62, 66, 60], Hand::Right, 0.25);
        let options = FingeringOptions::default();
        assert_eq!(options.rule_set, RuleSet::Consensus, "the default is the consensus");
        assert_eq!(
            fingers_of(&score, &finger_score(&score, &options)),
            fingers_of(&score, &finger_score_consensus(&score, &options, None))
        );
    }
}
