//! Finding the best fingering for a piece.
//!
//! # Why this is an exact search rather than a heuristic one
//!
//! The total cost of a fingering decomposes into terms that each look at no more
//! than three consecutive chords. That is not an approximation made for tractability
//! — it is the structure of the published models, and it is also the structure of
//! the biomechanics, because what a hand has to do next depends on where it is now
//! and where it is going, not on where it was ten bars ago. Given that structure,
//! the globally cheapest fingering is found by dynamic programming over states of
//! the form *(fingering of the previous chord, fingering of this chord)*.
//!
//! Every chord has at most `C(5, k)` fingerings — ten, for a triad — so the state
//! space is small and the search is exhaustive rather than greedy.
//!
//! # What gets charged
//!
//! * within a chord: how the fingers sit against each other, and the posture the
//!   whole shape demands, from [`crate::biomech`]
//! * between two chords: the work of reconfiguring the hand in the time available
//! * across three chords: the published rules, read along the chord's outer voices
//! * optionally, a statistical prior trained on how pianists actually finger

use on_hand::{Finger, Hand, HandProfile};
use on_score::{Fingering, NoteId, Score};

use crate::biomech::{BiomechModel, BiomechWeights, Grip};
use crate::rules::{self, Placement, Rule, RuleScorer, RuleSet, RuleWeights, Trigram};
use crate::ruler::Ruler;
use crate::spans::SpanModel;

/// Rules charged between adjacent members of the same chord. These are the ones
/// about how a pair of fingers sits, which is exactly what a chord asks of them.
const WITHIN_CHORD_RULES: &[Rule] = &[
    Rule::Stretch,
    Rule::SmallSpan,
    Rule::LargeSpan,
    Rule::Impractical,
    Rule::ThreeToFour,
    Rule::FourOnBlack,
];

/// Rules charged once per note, regardless of its neighbours.
const PER_NOTE_RULES: &[Rule] = &[Rule::WeakFinger];

/// The same, for a voice whose finger did not change while the hand moved.
///
/// [`Rule::Impractical`] charges a finger for playing two different notes in a row,
/// because a line played that way cannot be joined. Between two chords that charge is
/// wrong: the fingers are the corners of a shape rather than voices of a line, and a
/// shape is meant to keep its fingers while the hand carries it somewhere else. Octaves
/// are the plainest case — a run of them is 1-5 all the way, in every edition ever
/// printed — but it is the same for repeated triads, sixths, and any passage a hand
/// plays in position. The travel is charged already, by the motion and posture terms.
///
/// The rule is left in place wherever the finger *did* change, since it is then saying
/// something about the span between two different fingers, which is still true.
const SHIFTED_VOICE_RULES: &[Rule] = &[Rule::WeakFinger, Rule::Impractical];

/// How a fingering search is configured.
#[derive(Debug, Clone)]
pub struct FingeringOptions {
    /// The pianist's hand.
    pub profile: HandProfile,
    /// Which published rule set to score with.
    pub rule_set: RuleSet,
    /// Which span table to use.
    pub span_model: SpanModel,
    /// Whether the rules measure semitones or millimetres.
    pub ruler: Ruler,
    /// Per-rule weights.
    pub rule_weights: RuleWeights,
    /// Weights for the biomechanical terms.
    pub biomech: BiomechWeights,
    /// Overall weight on the rule term relative to the biomechanical one.
    pub rule_scale: f32,
    /// Overall weight on the statistical prior. Zero until a model is trained.
    pub prior_scale: f32,
    /// Weight on the standard chord shapes pianists are taught.
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
    /// Options for a hand of a given size, with the matching span table.
    pub fn for_hand(profile: HandProfile) -> Self {
        Self {
            span_model: SpanModel::for_hand_size(profile.size()),
            profile,
            ..Default::default()
        }
    }
}

/// A statistical model of how pianists finger, blended in alongside the rules.
///
/// Left as a trait so the solver does not depend on how the model is trained or
/// stored. Implementations return a log probability; the solver charges its
/// negative, so a likely fingering costs less.
///
/// `Sync` because the four published rule sets are asked what they think in parallel,
/// and they are all asked the same question of the same prior. A prior is a table of
/// counts read and never written, so this costs its implementors nothing.
pub trait FingeringPrior: Sync {
    /// Log probability of `current` following `prev` and `prev2` in this hand.
    fn log_probability(
        &self,
        hand: Hand,
        prev2: Option<Placement>,
        prev: Option<Placement>,
        current: Placement,
    ) -> f32;
}

/// One chord of one hand: the unit the search steps through.
#[derive(Debug, Clone)]
pub struct Event {
    /// Notes low to high. Everything the hand has down at this instant, not only what
    /// it strikes here.
    pub notes: Vec<u8>,
    /// The score notes they came from, aligned with `notes`.
    pub ids: Vec<NoteId>,
    /// Which of them are struck here, rather than still sounding from earlier.
    pub struck: Vec<bool>,
    /// When the chord sounds.
    pub onset_seconds: f64,
    /// A finger the source already fixed for each note, if any.
    pub pinned: Vec<Option<Finger>>,
}

impl Event {
    /// How many notes are in the chord.
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    /// Whether the chord is empty.
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }
}

/// One way of fingering one chord.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The finger for each note of the chord, aligned with [`Event::notes`].
    pub fingers: Vec<Finger>,
}

/// What each part of the cost function charged for one chord shape.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CostBreakdown {
    /// The published rules, within the chord and across its neighbours.
    pub rules: f32,
    /// Extra discomfort compared with the easiest way to hold this same chord.
    pub posture: f32,
    /// Credit for matching a shape pianists are taught. Negative, being a bonus.
    pub pattern: f32,
    /// Work of moving the hand into this shape from the one before it.
    pub motion: f32,
    /// Credit for a finger the published rule sets independently agreed on. Negative,
    /// being a bonus, and zero unless the search was given their answers.
    pub agreement: f32,
}

impl CostBreakdown {
    /// The total, which is what the search compares.
    pub fn total(&self) -> f32 {
        self.rules + self.posture + self.pattern + self.motion + self.agreement
    }
}

/// Why a particular note ended up with the finger it did.
#[derive(Debug, Clone)]
pub struct NoteExplanation {
    /// The note.
    pub note: NoteId,
    /// The finger chosen.
    pub finger: Finger,
    /// The rules that charged something against the chosen chord shape, worst first.
    pub rules: Vec<(Rule, f32)>,
    /// Discomfort of the posture the chord demands.
    pub posture_strain: f32,
    /// Whether the hand could actually reach the chord.
    pub reachable: bool,
    /// How much worse the best alternative finger for this note would have been.
    /// Small numbers mean the choice was close; large ones mean it was forced.
    pub margin: f32,
    /// Where the cost of this chord shape came from.
    pub cost: CostBreakdown,
}

/// The result of a search.
#[derive(Debug, Clone)]
pub struct Solution {
    /// One fingering per sounding note.
    pub fingerings: Vec<Fingering>,
    /// Total cost of the chosen path.
    pub cost: f32,
    /// Per-note reasoning.
    pub explanations: Vec<NoteExplanation>,
    /// How far the four published sets agreed, as (unanimous, split) notes, when the
    /// consensus was the model used.
    ///
    /// `None` for a single published set, because then no vote was taken. Carried here
    /// so a caller wanting the figure does not have to run the four searches a second
    /// time to get it — which is four fifths of the work of a consensus run.
    pub agreement: Option<(usize, usize)>,
}

impl Solution {
    /// The finger assigned to a note, if it was fingered.
    pub fn finger_of(&self, note: NoteId) -> Option<Finger> {
        self.fingerings
            .iter()
            .find(|f| f.note == note)
            .map(|f| f.finger)
    }
}

/// How strongly a note's finger is worth in agreement bonus, at full agreement.
///
/// Small on purpose. This is a tie-breaker, not a vote: the consensus model has
/// already been asked what it thinks, and this only leans on the answer where the
/// published sets, each reasoning over the whole passage, independently landed in the
/// same place. Set it high enough to override the model and it stops being a search.
const AGREEMENT_WEIGHT: f32 = 0.6;

/// What the published rule sets independently chose, note by note.
///
/// Built by running each set's own search to completion and recording its answer. The
/// counts are then available to a final search as a preference — never as a decision.
/// See [`RuleWeights::consensus`] for why the outputs are not simply voted on.
#[derive(Debug, Clone, Default)]
pub struct Agreement {
    /// For each note, how many sets picked each finger.
    votes: std::collections::HashMap<NoteId, [u8; 5]>,
    /// How many sets voted at all, which is what a count is a fraction of.
    voters: u8,
}

impl Agreement {
    /// Run every published rule set over a score and record what each one chose.
    pub fn across_published(
        score: &Score,
        options: &FingeringOptions,
        prior: Option<&dyn FingeringPrior>,
    ) -> Self {
        // Four searches that do not talk to each other, and together about four fifths
        // of the work of fingering a piece: a couple of thousand notes took seven
        // seconds, of which five and a half were these. One thread each, and the
        // answers tallied afterwards in a fixed order so the result does not depend on
        // which thread finished first.
        let answers: Vec<Solution> = std::thread::scope(|scope| {
            let running: Vec<_> = rules::PUBLISHED
                .into_iter()
                .map(|set| {
                    let mut single = options.clone();
                    single.rule_set = set;
                    // The published sets are being asked what *they* think, so they are
                    // asked without the agreement term — which does not exist yet — but
                    // with everything else the real search uses.
                    scope.spawn(move || finger_score_with_prior(score, &single, prior))
                })
                .collect();
            running.into_iter().filter_map(|thread| thread.join().ok()).collect()
        });

        let mut votes: std::collections::HashMap<NoteId, [u8; 5]> =
            std::collections::HashMap::new();
        for answer in &answers {
            for fingering in &answer.fingerings {
                votes.entry(fingering.note).or_default()[fingering.finger.index()] += 1;
            }
        }
        Self { votes, voters: answers.len() as u8 }
    }

    /// How much to take off a note's cost for choosing `finger`.
    ///
    /// Zero when nobody chose it, and [`AGREEMENT_WEIGHT`] when they all did.
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

    /// How many notes the published sets were unanimous about, and how many they
    /// disagreed on. Reported by `explain`, and the honest measure of how much this
    /// term is doing.
    pub fn tally(&self) -> (usize, usize) {
        let unanimous = self
            .votes
            .values()
            .filter(|c| c.contains(&self.voters))
            .count();
        (unanimous, self.votes.len() - unanimous)
    }
}

/// Finger a whole score with every published rule set at once.
///
/// This is what the application uses. Two things are combined, at the two places where
/// the studies actually differ:
///
/// 1. **Which rules matter.** Every rule any set charges for is charged, scaled by how
///    many of the four endorse it — see [`RuleWeights::consensus`]. One search, so the
///    result is a coherent path rather than a stitched-together one.
/// 2. **What the sets concluded.** Each set is also run on its own, and where they
///    independently agree on a finger the final search is nudged towards it. Only
///    nudged: the agreement is worth a fraction of one rule, so it settles ties and
///    loses arguments.
///
/// The second is the part that cannot be folded into the first. Two rule sets can share
/// every rule and still reach different fingerings, because the rules interact over a
/// whole passage; where four separate searches land on the same finger anyway, that is
/// evidence no static weighting expresses.
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

/// Finger a whole score, both hands.
///
/// Notes that continue a tie keep sounding rather than being struck again, so they
/// are not given a finger of their own; everything else is.
pub fn finger_score(score: &Score, options: &FingeringOptions) -> Solution {
    finger_score_with_prior(score, options, None)
}

/// Finger a whole score, optionally blending in a trained prior.
///
/// [`RuleSet::Consensus`] is not a fifth rule set with weights of its own; it is the
/// other four read together, so asking for it has to actually ask them. Dispatching
/// here rather than leaving each caller to remember is what keeps every entry point
/// measuring and playing the same model: `eval` used to solve with the consensus
/// weights and none of the agreement, and so reported accuracy for a model that
/// nothing actually runs.
pub fn finger_score_with_prior(
    score: &Score,
    options: &FingeringOptions,
    prior: Option<&dyn FingeringPrior>,
) -> Solution {
    if options.rule_set == RuleSet::Consensus {
        // Safe from running away: the consensus asks only the published sets, and none
        // of those is the consensus.
        return finger_score_consensus(score, options, prior);
    }
    solve_score(score, options, prior, None)
}

/// The search itself, with everything it can be given.
fn solve_score(
    score: &Score,
    options: &FingeringOptions,
    prior: Option<&dyn FingeringPrior>,
    agreement: Option<&Agreement>,
) -> Solution {
    // The two hands never look at each other: each collects its own chords, holds its
    // own sustained notes and searches its own path. So they are searched at the same
    // time, and put back together in `Hand::ALL` order afterwards rather than as they
    // finish, which keeps the total cost — a sum of floats, and so an order-dependent
    // one — the same every run.
    let solved: Vec<Option<Solution>> = std::thread::scope(|scope| {
        let running: Vec<_> = Hand::ALL
            .into_iter()
            .map(|hand| {
                scope.spawn(move || {
                    let mut events = build_events(score, hand);
                    if events.is_empty() {
                        return None;
                    }
                    // The comfortable span, not the forced one. `max_prac` is what a
                    // hand can be made to reach for an instant, with everything else
                    // it is doing subordinated to getting there; it is not a shape a
                    // hand sits in while its other fingers go on playing. Holding is
                    // the second thing, so it is measured by the second number — and
                    // the two are far enough apart to matter, fourteen semitones
                    // against sixteen for a medium hand.
                    let reach = options.span_model.table().0[Finger::Thumb.index()]
                        [Finger::Little.index()]
                    .max_comf;
                    hold_sustained(&mut events, score, reach);
                    let mut solver = HandSolver::new(hand, options, prior, agreement);
                    solver.find_scales(&events);
                    Some(solver.solve(&events))
                })
            })
            .collect();
        running.into_iter().map(|thread| thread.join().ok().flatten()).collect()
    });

    let mut fingerings = Vec::new();
    let mut explanations = Vec::new();
    let mut cost = 0.0;
    for solved in solved.into_iter().flatten() {
        cost += solved.cost;
        fingerings.extend(solved.fingerings);
        explanations.extend(solved.explanations);
    }

    finger_the_twins(score, &mut fingerings);
    fingerings.sort_by_key(|f| f.note);
    explanations.sort_by_key(|e| e.note);
    Solution { fingerings, cost, explanations, agreement: None }
}

/// Give the same finger to notes the search collapsed into one.
///
/// A pitch struck twice at the same instant in the same hand is one key press. The
/// search fingers one of them; the other has to end up on the same finger, or the score
/// shows two fingerings on one key and everything downstream — the hand model included
/// — believes the hand is doing something it is not.
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

/// Collect one hand's chords, dropping notes that merely continue a tie.
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
            // One key, one finger. Scores — and MIDI files especially — often carry the
            // same pitch twice at the same instant, from a doubled voice or an
            // overlapping repeat. A pianist presses the key once; asking the search to
            // finger both copies makes it hand them different fingers, because within a
            // chord no two notes may share one. The twin is fingered afterwards, with
            // whatever this one gets.
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

/// How many notes one hand can have down at once.
const FINGERS: usize = 5;

/// What it costs to change the finger on a note that is already down.
///
/// Pianists do this — it is called a substitution, and it is how a hand keeps a legato
/// line going past the end of its reach — but it is a deliberate act, not something to
/// fall into. The cost has to be high for a second reason: it is what makes a held note
/// carry its finger forward from one event to the next, and so from the event that
/// struck it to every event it sounds through. Without that the search is free to give
/// the same held note a different finger each time, and the fingering it reports — the
/// one chosen where the note was struck — goes back to crossing over its neighbours.
const SUBSTITUTION_COST: f32 = 800.0;

/// Fold what is still sounding into each event.
///
/// Without this the search sees a chord at a time and nothing else, and a note the hand
/// is still holding from two beats ago constrains it not at all. That is how a left hand
/// ends up told to play 5, then 1 above it, then 3 above *that* — three fingers crossed
/// over each other, because no two of them were ever considered together.
///
/// It is also what the teaching says. The standard advice is to work out a fingering
/// with no pedal at all, precisely so that everything the score holds is actually held
/// by a finger; and Balliauw's hard constraint — one finger cannot play two notes
/// sounding at once — only bites if the search knows what is sounding.
///
/// Notes the hand could not still be holding are left out rather than forced in: beyond
/// its reach, or beyond its five fingers, the oldest are dropped. Those are the ones a
/// pianist has already given to the pedal.
fn hold_sustained(events: &mut [Event], score: &Score, reach: i32) {
    // When each note stops sounding, by the event it was struck at.
    let mut sounding: Vec<Sounding> = Vec::new();
    for index in 0..events.len() {
        let now = events[index].onset_seconds;
        sounding.retain(|s| s.until > now + 1e-6);

        let struck: Vec<(NoteId, u8)> = events[index]
            .ids
            .iter()
            .zip(&events[index].notes)
            .map(|(id, midi)| (*id, *midi))
            .collect();
        let (struck_low, struck_high) = struck
            .iter()
            .fold((u8::MAX, u8::MIN), |(lo, hi), (_, m)| (lo.min(*m), hi.max(*m)));

        // A finger cannot follow the hand. Once the hand has ranged further, since it
        // struck a note, than it can span in one shape, that note is not under a finger
        // any more — it has been given to the pedal and the hand has gone somewhere
        // else.
        //
        // Asking that of the note's whole history rather than only of this event is the
        // point. A bass octave under a broken chord passes the pairwise test at every
        // step — the octave is within reach, and so is the reach from its top note up
        // to the arpeggio — while no single hand shape satisfies both at once. Held on
        // that way, the octave came out fingered 5-3, which is not a shape a hand makes,
        // and the note under finger 5 was then drawn with nothing on it at all.
        sounding.retain(|s| i32::from(s.high.max(struck_high) - s.low.min(struck_low)) <= reach);
        for s in &mut sounding {
            s.low = s.low.min(struck_low);
            s.high = s.high.max(struck_high);
        }

        // Everything still down that this event does not strike again, newest first:
        // the oldest are the first a hand lets go of.
        let mut held: Vec<(NoteId, u8)> = sounding
            .iter()
            .rev()
            .filter(|s| !struck.iter().any(|(other, _)| *other == s.id))
            .map(|s| (s.id, s.midi))
            .collect();

        // Only what the hand could still be holding, taken newest first and stopping
        // as soon as one more would not fit. Both tests are on the whole shape rather
        // than on each note separately: five fingers, and a span the hand can reach
        // from its lowest note to its highest. Whatever is left over is being held by
        // the pedal, and the search is right not to plan a finger for it.
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
                // A note the source fixed a finger for keeps that finger for as long as
                // it sounds, not only at the moment it is struck. Dropping the pin here
                // would let the search quietly re-finger an editorial marking the
                // instant anything else was played.
                event.pinned.push(score.note(id).given_finger);
            }
            // Low to high, which everything downstream relies on.
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

/// A note still sounding, and how far the hand has ranged since striking it.
struct Sounding {
    id: NoteId,
    midi: u8,
    /// When it stops sounding.
    until: f64,
    /// The lowest and highest note the hand has struck since — and including — the
    /// event that struck this one. A finger stays on the note only while that fits in
    /// one hand.
    low: u8,
    high: u8,
}

/// Searches one hand's part.
struct HandSolver<'a> {
    hand: Hand,
    rules: RuleScorer,
    biomech: BiomechModel,
    options: &'a FingeringOptions,
    prior: Option<&'a dyn FingeringPrior>,
    /// What the published sets agreed on, if they were asked.
    agreement: Option<&'a Agreement>,
    /// The finger the standard scale fingering gives each event, where the passage
    /// is a scale. Computed once for the whole part, since it depends on a longer
    /// stretch of music than the search window sees.
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

    /// Work out where the standard scale fingerings apply.
    ///
    /// Only single notes take part: a scale is a line, and a passage in octaves or
    /// thirds is fingered by different rules.
    fn find_scales(&mut self, events: &[Event]) {
        let line: Vec<Option<u8>> = events
            .iter()
            .map(|e| if e.len() == 1 { Some(e.notes[0]) } else { None })
            .collect();
        let onsets: Vec<f64> = events.iter().map(|e| e.onset_seconds).collect();
        self.scales = crate::scales::scale_fingerings(self.hand, &line, &onsets);
    }

    /// Every fingering of a chord that this hand could physically arrange.
    ///
    /// Fingers must run in the same direction as the pitches — the right hand's
    /// finger numbers increase upward, the left hand's decrease — which is what
    /// stops the search proposing shapes with the fingers tangled. The genuine
    /// exception, a thumb crossing, happens *between* chords rather than within one,
    /// so nothing is lost by insisting on it here.
    fn candidates(&self, event: &Event) -> Vec<Candidate> {
        let k = event.len();
        let mut out = Vec::new();
        if k == 0 || k > 5 {
            // More notes than fingers: the chord must be rolled or split. Spread the
            // fingers evenly and let the biomechanical term price the result.
            if k > 5 {
                out.push(Candidate { fingers: spread_fingers(self.hand, k) });
            }
            return out;
        }

        let mut chosen = Vec::with_capacity(k);
        self.enumerate(event, 0, &mut chosen, &mut out);
        if out.is_empty() {
            // Every combination was ruled out by a pinned finger that does not fit.
            // Fall back to an even spread so the search always has somewhere to go.
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
        // Remaining fingers must leave room for the notes still to be placed.
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

    /// The grip a candidate describes.
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

    /// The lowest and highest note of a chord, with their fingers. These are the
    /// two voices the horizontal rules follow; for a single note they coincide, so
    /// a melody is scored exactly as the published models score it.
    fn outer(&self, event: &Event, candidate: &Candidate) -> (Placement, Placement) {
        let last = event.len() - 1;
        (
            Placement::new(event.notes[0], candidate.fingers[0]),
            Placement::new(event.notes[last], candidate.fingers[last]),
        )
    }

    /// The least strain any fingering of this chord could be held at.
    ///
    /// The posture term is charged relative to this. Some chords are simply harder
    /// to hold than others, and that is not the fingering's fault; what the search
    /// needs to know is how much worse *this* shape is than the best available one.
    /// Without the subtraction, a single note would be fingered by whichever digit
    /// happens to hang most comfortably rather than by what the passage needs.
    fn baseline_strain(&self, event: &Event, candidates: &[Candidate]) -> f32 {
        candidates
            .iter()
            .map(|c| self.biomech.grip_outcome(&self.grip(event, c)).strain)
            .fold(f32::INFINITY, f32::min)
    }

    /// Cost of the chord itself: how its fingers sit together, and what the whole
    /// posture costs to hold.
    fn event_cost(&self, index: usize, event: &Event, candidate: &Candidate, baseline: f32) -> f32 {
        self.event_breakdown(index, event, candidate, baseline).total()
    }

    /// The same cost, itemised.
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
        CostBreakdown {
            rules: self.options.rule_scale * rules,
            posture,
            pattern: -pattern,
            motion: 0.0,
            agreement: agreed,
        }
    }

    /// Cost of the published rules read across three consecutive chords, along both
    /// outer voices. Averaged, so a single-note passage is charged exactly once.
    fn window_cost(
        &self,
        prev: Option<(&Event, &Candidate)>,
        current: (&Event, &Candidate),
        next: Option<(&Event, &Candidate)>,
    ) -> f32 {
        let (low, high) = self.outer(current.0, current.1);
        let prev_outer = prev.map(|(e, c)| self.outer(e, c));
        let next_outer = next.map(|(e, c)| self.outer(e, c));

        // Whether the hand is carrying a shape from one place to another, rather than
        // playing a line: a chord before and a chord now.
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

    /// Cost of the statistical prior for one chord.
    fn prior_cost(
        &self,
        prev2: Option<(&Event, &Candidate)>,
        prev: Option<(&Event, &Candidate)>,
        current: (&Event, &Candidate),
    ) -> f32 {
        let Some(prior) = self.prior else { return 0.0 };
        if self.options.prior_scale == 0.0 {
            return 0.0;
        }
        let top = |pair: Option<(&Event, &Candidate)>| pair.map(|(e, c)| self.outer(e, c).1);
        let (_, here) = self.outer(current.0, current.1);
        let logp = prior.log_probability(self.hand, top(prev2), top(prev), here);
        -self.options.prior_scale * logp
    }

    /// Cost of moving the hand between two chords.
    fn move_cost(&self, from: (&Event, &Candidate), to: (&Event, &Candidate)) -> f32 {
        let seconds = to.0.onset_seconds - from.0.onset_seconds;
        let travel = self.biomech.transition_cost(
            &self.grip(from.0, from.1),
            &self.grip(to.0, to.1),
            seconds,
        );
        travel + SUBSTITUTION_COST * self.substitutions(from, to) as f32
    }

    /// How many notes are down through both events on a different finger in each.
    fn substitutions(&self, from: (&Event, &Candidate), to: (&Event, &Candidate)) -> usize {
        to.0
            .ids
            .iter()
            .enumerate()
            .filter(|(slot, id)| {
                // Only a note that carries over. One being struck here is a free choice.
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

    /// Run the search.
    fn solve(&self, events: &[Event]) -> Solution {
        let all: Vec<Vec<Candidate>> = events.iter().map(|e| self.candidates(e)).collect();
        let baselines: Vec<f32> = events
            .iter()
            .zip(&all)
            .map(|(e, c)| self.baseline_strain(e, c))
            .collect();

        // Second-order Viterbi. `best[(a, b)]` is the cheapest path that ends with
        // candidate `a` at event i-1 and candidate `b` at event i.
        let n = events.len();
        if n == 1 {
            return self.single_event(&events[0], &all[0], baselines[0]);
        }

        let mut best: Vec<Vec<f32>> = Vec::with_capacity(n);
        let mut from: Vec<Vec<usize>> = Vec::with_capacity(n);

        // Event 0 and 1 seed the recursion.
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

        // Close the final window, whose centre is the last event and whose right
        // neighbour is absent.
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

        // Walk the path back.
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

    /// The degenerate case of a piece with a single chord in this hand.
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

    /// Turn a chosen path into fingerings and explanations.
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

        for (i, event) in events.iter().enumerate() {
            let candidate = &all[i][chosen[i]];
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
            // Must match what the search actually charged, or the explanation
            // describes a different cost function from the one that decided.
            let shifting = event.len() > 1 && prev.is_some_and(|(e, _)| e.len() > 1);
            let held = window.prev.is_some_and(|p| p.finger == window.current.finger);
            let skip = if shifting && held { SHIFTED_VOICE_RULES } else { PER_NOTE_RULES };
            let fired = self.rules.score_melodic(&window, skip).fired();
            let _ = low;

            // How much worse the runner-up would have been, holding the rest of the
            // path fixed. A small margin means the choice was nearly arbitrary and
            // worth flagging; a large one means it was forced.
            let mut breakdown = self.event_breakdown(i, event, candidate, baselines[i]);
            breakdown.rules += self.window_cost(prev, (event, candidate), next);
            breakdown.motion = prev
                .map(|p| self.move_cost(p, (event, candidate)))
                .unwrap_or(0.0);
            let local = |c: &Candidate| {
                self.event_cost(i, event, c, baselines[i])
                    + self.window_cost(prev, (event, c), next)
                    + prev.map(|p| self.move_cost(p, (event, c))).unwrap_or(0.0)
                    + next.map(|n| self.move_cost((event, c), n)).unwrap_or(0.0)
            };
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
                // A note is fingered at the event that strikes it. It appears again in
                // every later event that it is still sounding through, where it
                // constrains the hand but is not being chosen for.
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

        Solution { fingerings, cost, explanations, agreement: None }
    }
}

/// Spread the five fingers over more notes than there are fingers, which happens
/// only in chords meant to be rolled.
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

    /// Build a single-line score in one hand from pitches, one per beat.
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

    /// The same, but every note doubled an octave above.
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
            // Choosing 3 of 5 fingers in a fixed order.
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
        // Force the opening C onto finger 2 and see the rest adapt.
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
        // Thumb forced onto a black key by a chromatic run.
        let score = melody(&[65, 66, 67], Hand::Right, 0.5);
        let solution = finger_score(&score, &FingeringOptions::default());
        assert!(solution.explanations.iter().all(|e| e.reachable));
        assert!(solution.explanations.iter().all(|e| e.margin >= 0.0));
    }

    #[test]
    fn tempo_changes_the_answer() {
        // A wide leap is fingered differently when there is no time to move.
        let wide = [60u8, 84, 60, 84, 60, 84];
        let slow = finger_score(&melody(&wide, Hand::Right, 2.0), &FingeringOptions::default());
        let fast = finger_score(&melody(&wide, Hand::Right, 0.1), &FingeringOptions::default());
        // Both must still be complete and reachable.
        assert_eq!(slow.fingerings.len(), wide.len());
        assert_eq!(fast.fingerings.len(), wide.len());
        // The fast version should cost more, because the same distance has to be
        // covered in a fraction of the time.
        assert!(fast.cost > slow.cost, "slow {} fast {}", slow.cost, fast.cost);
    }

    #[test]
    fn a_two_octave_arpeggio_is_fingered_the_way_the_charts_print_it() {
        // Nothing in the code knows about arpeggios: there is no table of them the way
        // there is for scales and for the chromatic. The published fingering falls out
        // of the hand model on its own, which is the best evidence there is that the
        // model is shaped right — so it is worth noticing if it ever stops.
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

            // The left hand plays the same shape from the other end.
            let score = melody(&pitches, Hand::Left, 0.5);
            let left = fingers_of(&score, &finger_score(&score, &FingeringOptions::default()));
            assert_eq!(
                left,
                vec![5, 3, 2, 1, 3, 2, 1, 2, 3, 1, 2, 3, 5],
                "left hand, arpeggio on {base}"
            );
        }
    }

    #[test]
    fn a_chromatic_run_takes_the_third_finger_on_every_black_key() {
        // Third finger on the black keys, thumb on the white ones, and the second
        // finger where two white keys sit side by side. Left to itself the model
        // alternates the thumb and the second finger the whole way up, which crosses
        // nothing over anything and is exactly why nobody plays it that way.
        for (hand, seconds) in [(Hand::Right, [0u8, 5]), (Hand::Left, [4, 11])] {
            let score = melody(&(60..=72).collect::<Vec<u8>>(), hand, 0.25);
            let solution = finger_score_consensus(&score, &FingeringOptions::default(), None);
            let fingers = fingers_of(&score, &solution);
            // The ends of a run are the search's to settle, as with the major scales.
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
        // The least controversial fingering in the repertoire: octaves are 1-5, every
        // one of them, in every edition. Two separate readings of the published rules
        // used to break it, and both came from measuring a finger against itself —
        // whose span the table records as exactly zero — so that a hand carrying a
        // fixed shape up the keyboard looked like a hand repeatedly reorganising and
        // repeatedly breaking a line.
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
        // The four published sets are asked in parallel, one thread each. Their answers
        // are tallied afterwards in a fixed order rather than as they arrive, so which
        // thread finishes first cannot change what comes out — and it would be a quiet
        // sort of wrong if it could, since nothing about the fingering would look
        // broken, it would just be a different fingering on Tuesday.
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
        // The consensus is the four published sets read together. Solving with its
        // weights but without asking them is a fifth model that nothing plays, and it
        // is what `eval` was quietly measuring.
        // A line the two disagree about: with the four sets asked, the sixth note takes
        // 2 rather than 3.
        let score = melody(&[63, 65, 69, 66, 62, 65, 62, 66, 60], Hand::Right, 0.25);
        let options = FingeringOptions::default();
        assert_eq!(options.rule_set, RuleSet::Consensus, "the default is the consensus");
        assert_eq!(
            fingers_of(&score, &finger_score(&score, &options)),
            fingers_of(&score, &finger_score_consensus(&score, &options, None))
        );
    }
}
