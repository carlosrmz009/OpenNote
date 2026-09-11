//! The published ergonomic rules, as weighted penalties over three consecutive notes.
//!
//! Parncutt's model scores a fingering by walking a window of three notes at a time
//! and charging points for each recognisable source of awkwardness: a stretch beyond
//! what a finger pair can comfortably span, the thumb landing on a black key, the
//! ring finger being asked to work alone. The score of a whole passage is the sum
//! over its windows, which is what makes an exact dynamic program possible.
//!
//! Four rule sets are implemented:
//!
//! * [`RuleSet::Parncutt`] — the original twelve rules of Parncutt, Sloboda, Clarke,
//!   Raekallio & Desain (1997)
//! * [`RuleSet::Jacobs`] — Jacobs' (2001) revisions: only finger 4 counts as weak,
//!   large spans are charged less, and the three-four-five rule is dropped
//! * [`RuleSet::Balliauw`] — Balliauw et al.'s reformulation, which replaces the two
//!   position-change rules and adds a hard penalty for the physically impossible
//! * [`RuleSet::Badgerow`] — Badgerow's pianist-reviewed revision, adding rules about
//!   alternating weak fingers and pivoting off a black-key thumb
//!
//! Ported from the reference implementations in `pydactyl` (David A. Randolph, MIT
//! licensed), which are the closest thing to a canonical transcription of the papers.
//!
//! These rules are one term of the cost function, not the whole of it. They know
//! about semitones and key colour; they know nothing about how long the note lasts
//! or what the hand is actually doing, which is what [`crate::biomech`] adds.

use on_hand::keyboard::{is_black, is_white};
use on_hand::{Finger, Hand};
use serde::{Deserialize, Serialize};

use crate::ruler::Ruler;
use crate::spans::SpanTable;

/// One note with the finger that plays it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// MIDI pitch.
    pub midi: u8,
    /// The finger playing it.
    pub finger: Finger,
}

impl Placement {
    /// A note and its finger.
    pub fn new(midi: u8, finger: Finger) -> Self {
        Self { midi, finger }
    }
}

/// Three consecutive notes of one hand, with the cost attributed to the middle one.
///
/// The first and last may be absent at the ends of a passage, in which case the
/// rules that need them contribute nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trigram {
    /// Which hand is playing.
    pub hand: Hand,
    /// The note before.
    pub prev: Option<Placement>,
    /// The note being charged.
    pub current: Placement,
    /// The note after.
    pub next: Option<Placement>,
}

impl Trigram {
    /// Whether the window is one finger carrying the hand somewhere else.
    ///
    /// When all three notes take the same finger, the hand has kept its shape and
    /// simply moved: the top or the bottom of a run of octaves, a repeated chord
    /// walking up the keyboard, a tremolo. Nothing about the hand was reorganised, and
    /// the travel is charged by the motion and posture terms rather than here.
    ///
    /// The rules that ask this are the ones that measure the window's outer notes
    /// against the span of their two fingers, and a finger spans nothing with itself —
    /// the table records exactly zero either way. Read literally that makes every step
    /// of a settled hand a position change beyond what is comfortable. Parncutt
    /// enumerates fingerings in which consecutive notes take different fingers, so the
    /// case cannot arise for him; here it arises constantly.
    ///
    /// It has to be all three. A thumb, another finger, and the thumb again is the
    /// opposite case — the hand genuinely relocates between the first note and the
    /// last, which is what these rules exist to catch.
    fn hand_only_travelled(&self) -> bool {
        matches!(
            (self.prev, self.next),
            (Some(prev), Some(next))
                if prev.finger == self.current.finger && self.current.finger == next.finger
        )
    }

    /// A trigram with both neighbours present.
    pub fn new(hand: Hand, prev: Placement, current: Placement, next: Placement) -> Self {
        Self { hand, prev: Some(prev), current, next: Some(next) }
    }
}

/// Every rule any of the supported models uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rule {
    /// Interval beyond what the finger pair comfortably spans.
    Stretch,
    /// Interval narrower than the finger pair relaxes to.
    SmallSpan,
    /// Interval wider than the finger pair relaxes to.
    LargeSpan,
    /// Whether the hand has to change position, and how completely.
    PositionChangeCount,
    /// How far the hand has to move when it does change position.
    PositionChangeSize,
    /// Use of a weak finger.
    WeakFinger,
    /// Fingers 3, 4 and 5 all used within one window.
    ThreeFourFive,
    /// Finger 3 immediately followed by finger 4.
    ThreeToFour,
    /// Finger 4 on a black key next to finger 3 on a white one.
    FourOnBlack,
    /// Thumb on a black key.
    ThumbOnBlack,
    /// Little finger on a black key.
    FiveOnBlack,
    /// Thumb passing under, or a finger crossing over it.
    ThumbPassing,
    /// Balliauw's replacement for the position-change count.
    BalliauwPositionChange,
    /// Balliauw's replacement for the position-change size.
    BalliauwPositionComfort,
    /// Reusing a finger across a position change.
    RepeatFingerOnPositionChange,
    /// An interval the finger pair simply cannot span.
    Impractical,
    /// Two different fingers on the same repeated pitch.
    NonRepeatingFinger,
    /// Badgerow: alternating between two adjacent weak fingers.
    AlternationPairing,
    /// Badgerow: the same pitch taken by two different fingers around a neighbour.
    AlternationFingerChange,
    /// Badgerow: pivoting from a black-key thumb onto a weak finger.
    BlackThumbPivot,
}

impl Rule {
    /// Every rule, in a fixed order that the weight vector depends on.
    pub const ALL: [Rule; 20] = [
        Rule::Stretch,
        Rule::SmallSpan,
        Rule::LargeSpan,
        Rule::PositionChangeCount,
        Rule::PositionChangeSize,
        Rule::WeakFinger,
        Rule::ThreeFourFive,
        Rule::ThreeToFour,
        Rule::FourOnBlack,
        Rule::ThumbOnBlack,
        Rule::FiveOnBlack,
        Rule::ThumbPassing,
        Rule::BalliauwPositionChange,
        Rule::BalliauwPositionComfort,
        Rule::RepeatFingerOnPositionChange,
        Rule::Impractical,
        Rule::NonRepeatingFinger,
        Rule::AlternationPairing,
        Rule::AlternationFingerChange,
        Rule::BlackThumbPivot,
    ];

    /// Index into the weight vector.
    pub fn index(self) -> usize {
        self as usize
    }

    /// Short tag, matching the names used in the literature and in `pydactyl`.
    pub fn tag(self) -> &'static str {
        match self {
            Rule::Stretch => "str",
            Rule::SmallSpan => "sma",
            Rule::LargeSpan => "lar",
            Rule::PositionChangeCount => "pcc",
            Rule::PositionChangeSize => "pcs",
            Rule::WeakFinger => "wea",
            Rule::ThreeFourFive => "345",
            Rule::ThreeToFour => "3t4",
            Rule::FourOnBlack => "bl4",
            Rule::ThumbOnBlack => "bl1",
            Rule::FiveOnBlack => "bl5",
            Rule::ThumbPassing => "pa1",
            Rule::BalliauwPositionChange => "bpc",
            Rule::BalliauwPositionComfort => "bpf",
            Rule::RepeatFingerOnPositionChange => "brp",
            Rule::Impractical => "bim",
            Rule::NonRepeatingFinger => "bnr",
            Rule::AlternationPairing => "aap",
            Rule::AlternationFingerChange => "aaf",
            Rule::BlackThumbPivot => "abp",
        }
    }

    /// A one-line description, used when explaining a fingering choice.
    pub fn describe(self) -> &'static str {
        match self {
            Rule::Stretch => "the interval is wider than that finger pair spans comfortably",
            Rule::SmallSpan => "those fingers are closer together than they rest",
            Rule::LargeSpan => "those fingers are further apart than they rest",
            Rule::PositionChangeCount => "the hand has to shift position",
            Rule::PositionChangeSize => "the hand has to shift a long way",
            Rule::WeakFinger => "it uses a weak finger",
            Rule::ThreeFourFive => "fingers 3, 4 and 5 are all in play at once",
            Rule::ThreeToFour => "finger 3 hands over directly to finger 4",
            Rule::FourOnBlack => "finger 4 is on a black key beside finger 3 on a white one",
            Rule::ThumbOnBlack => "the thumb is on a black key",
            Rule::FiveOnBlack => "the little finger is on a black key",
            Rule::ThumbPassing => "the thumb passes under, or a finger crosses over it",
            Rule::BalliauwPositionChange => "the hand has to shift position",
            Rule::BalliauwPositionComfort => "the shift takes the hand outside a comfortable span",
            Rule::RepeatFingerOnPositionChange => "the same finger is reused across a shift",
            Rule::Impractical => "that finger pair cannot span this interval at all",
            Rule::NonRepeatingFinger => "a repeated note changes finger",
            Rule::AlternationPairing => "it alternates between two adjacent weak fingers",
            Rule::AlternationFingerChange => "a repeated pitch is taken by a different finger",
            Rule::BlackThumbPivot => "it pivots from a black-key thumb onto a weak finger",
        }
    }
}

/// Number of rules, and so the length of the weight vector.
pub const RULE_COUNT: usize = Rule::ALL.len();

/// Which published rule set to score with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RuleSet {
    /// Parncutt et al. (1997), the original twelve.
    Parncutt,
    /// Jacobs (2001).
    Jacobs,
    /// Balliauw et al.
    Balliauw,
    /// Badgerow's revision.
    Badgerow,
    /// All four at once. The default, and what the application uses.
    #[default]
    Consensus,
}

/// What one finger playing two different notes in a row is charged, before the rule's
/// own weight is applied.
///
/// A flat amount rather than one that grows with the interval. Whether the two notes are
/// a semitone or an octave apart, the finger has to let go of the first to reach the
/// second, and that is the whole of what is wrong with it.
const SAME_FINGER_STEP: f32 = 3.0;

/// Rules about what a hand can physically do, rather than about what is comfortable.
///
/// These keep their full weight in the consensus however few of the published sets
/// list them: a majority cannot vote an interval into being reachable.
const HARD_CONSTRAINTS: [Rule; 1] = [Rule::Impractical];

/// The published sets, which [`RuleSet::Consensus`] is built from.
pub const PUBLISHED: [RuleSet; 4] =
    [RuleSet::Parncutt, RuleSet::Jacobs, RuleSet::Balliauw, RuleSet::Badgerow];

impl RuleSet {
    /// The rules this set applies.
    pub fn rules(self) -> &'static [Rule] {
        use Rule::*;
        match self {
            RuleSet::Parncutt => &[
                Stretch,
                SmallSpan,
                LargeSpan,
                PositionChangeCount,
                PositionChangeSize,
                WeakFinger,
                ThreeFourFive,
                ThreeToFour,
                FourOnBlack,
                ThumbOnBlack,
                FiveOnBlack,
                ThumbPassing,
            ],
            // Jacobs drops the three-four-five rule, softens large spans, and
            // considers only finger 4 weak.
            RuleSet::Jacobs => &[
                Stretch,
                SmallSpan,
                LargeSpan,
                PositionChangeCount,
                PositionChangeSize,
                WeakFinger,
                ThreeToFour,
                FourOnBlack,
                ThumbOnBlack,
                FiveOnBlack,
                ThumbPassing,
            ],
            RuleSet::Balliauw => &[
                Stretch,
                SmallSpan,
                LargeSpan,
                WeakFinger,
                ThreeFourFive,
                ThreeToFour,
                FourOnBlack,
                ThumbOnBlack,
                FiveOnBlack,
                ThumbPassing,
                BalliauwPositionChange,
                BalliauwPositionComfort,
                RepeatFingerOnPositionChange,
                Impractical,
                NonRepeatingFinger,
            ],
            RuleSet::Badgerow => &[
                Stretch,
                SmallSpan,
                LargeSpan,
                PositionChangeCount,
                PositionChangeSize,
                WeakFinger,
                ThreeFourFive,
                ThreeToFour,
                FourOnBlack,
                ThumbOnBlack,
                FiveOnBlack,
                ThumbPassing,
                Impractical,
                AlternationPairing,
                AlternationFingerChange,
                BlackThumbPivot,
            ],
            // Everything any of them charges for. Which of these matter, and by how
            // much, is not decided here — it is decided by the weights, which scale
            // each rule by how many of the four published sets endorse it. See
            // [`RuleWeights::consensus`].
            RuleSet::Consensus => &[
                Stretch,
                SmallSpan,
                LargeSpan,
                PositionChangeCount,
                PositionChangeSize,
                WeakFinger,
                ThreeFourFive,
                ThreeToFour,
                FourOnBlack,
                ThumbOnBlack,
                FiveOnBlack,
                ThumbPassing,
                BalliauwPositionChange,
                BalliauwPositionComfort,
                RepeatFingerOnPositionChange,
                Impractical,
                NonRepeatingFinger,
                AlternationPairing,
                AlternationFingerChange,
                BlackThumbPivot,
            ],
        }
    }

    /// How many of the published sets include a rule.
    ///
    /// This is the whole basis of the consensus: a rule that every author charges for
    /// is one the field agrees about, and a rule only one author charges for is one
    /// author's opinion. Weighting by the count keeps both, in proportion.
    pub fn endorsements(rule: Rule) -> usize {
        PUBLISHED
            .iter()
            .filter(|set| set.rules().contains(&rule))
            .count()
    }

    /// Whether only finger 4 counts as weak, as Jacobs argued.
    ///
    /// The consensus follows the other three: Jacobs is alone in excusing the little
    /// finger, and a majority of one out of four is not a consensus.
    fn weak_is_only_four(self) -> bool {
        matches!(self, RuleSet::Jacobs)
    }

    /// Penalty for a thumb pass that also changes key colour.
    fn bad_level_change_cost(self) -> f32 {
        match self {
            RuleSet::Balliauw => 2.0,
            RuleSet::Consensus => 2.75,
            _ => 3.0,
        }
    }

    /// Penalty per semitone for a large span not involving the thumb.
    fn large_span_penalty(self) -> f32 {
        match self {
            RuleSet::Jacobs => 1.0,
            RuleSet::Consensus => 1.75,
            _ => 2.0,
        }
    }

    /// Penalty per semitone for a small span not involving the thumb.
    fn small_span_penalty(self) -> f32 {
        match self {
            RuleSet::Balliauw => 1.0,
            RuleSet::Consensus => 1.75,
            _ => 2.0,
        }
    }
}

/// Weight on each rule.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RuleWeights(pub [f32; RULE_COUNT]);

impl Default for RuleWeights {
    fn default() -> Self {
        let mut w = [1.0; RULE_COUNT];
        // Asking a finger pair to do something it physically cannot is charged
        // heavily, so the solver treats it as very nearly a hard constraint while
        // still always being able to return an answer.
        w[Rule::Impractical.index()] = 10.0;
        RuleWeights(w)
    }
}

impl RuleWeights {
    /// Weights for [`RuleSet::Consensus`]: each rule scaled by how many of the four
    /// published sets endorse it.
    ///
    /// This is where the four studies are actually combined. They are not four
    /// independent opinions about which finger to use — they are four variations on one
    /// model, and where they differ they differ about *which rules matter*: Jacobs
    /// drops Parncutt's three-four-five rule, Balliauw replaces the position-change
    /// rules with his own and adds a practicality test, Badgerow keeps Parncutt's and
    /// adds three rules about alternation. So the honest way to combine them is to
    /// charge every rule any of them charges for, in proportion to how many of them do.
    ///
    /// Combining their *outputs* instead — running four searches and taking the
    /// majority finger for each note — is the obvious approach and is unsound. A
    /// fingering is a path, not a set of independent choices: if one set says 1-2-3 and
    /// another says 2-3-4, a per-note majority can return 1-3-4, which neither set
    /// considered and which may be unplayable. Whatever the sets agree on is still
    /// worth knowing, and it is used, but as a nudge inside a single coherent search
    /// rather than as the answer. See `solver::Agreement`.
    pub fn by_endorsement(self) -> Self {
        let mut weights = self;
        for rule in Rule::ALL {
            // Not everything here is an opinion. `Impractical` says a finger pair
            // cannot span an interval at all, and is weighted ten times over precisely
            // so the search treats it as very nearly a hard constraint; scaling it down
            // because only two of the four authors happened to write it down defeats
            // the whole point of it. Parncutt does not list it because his rules were
            // written for melodic fragments where it does not arise, not because he
            // thought a hand could do it.
            if HARD_CONSTRAINTS.contains(&rule) {
                continue;
            }
            let share = RuleSet::endorsements(rule) as f32 / PUBLISHED.len() as f32;
            // A rule no published set uses cannot be reached anyway, so a zero here
            // would be harmless; a floor is kept so the arithmetic never silences a
            // rule the union deliberately includes.
            let share = if share == 0.0 { 1.0 } else { share };
            weights.0[rule.index()] *= share;
        }
        weights
    }
}

impl RuleWeights {
    /// Weight on one rule.
    pub fn get(&self, rule: Rule) -> f32 {
        self.0[rule.index()]
    }

    /// Set the weight on one rule.
    pub fn set(&mut self, rule: Rule, weight: f32) {
        self.0[rule.index()] = weight;
    }
}

/// The per-rule penalties charged against one trigram.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuleCosts {
    /// Weighted cost of each rule, indexed by [`Rule::index`].
    pub per_rule: [f32; RULE_COUNT],
}

impl Default for RuleCosts {
    fn default() -> Self {
        Self { per_rule: [0.0; RULE_COUNT] }
    }
}

impl RuleCosts {
    /// The sum of every rule's contribution.
    pub fn total(&self) -> f32 {
        self.per_rule.iter().sum()
    }

    /// The rules that actually fired, worst first.
    pub fn fired(&self) -> Vec<(Rule, f32)> {
        let mut out: Vec<_> = Rule::ALL
            .iter()
            .filter(|r| self.per_rule[r.index()] > 0.0)
            .map(|r| (*r, self.per_rule[r.index()]))
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        out
    }
}

/// Rules that describe two fingers holding an interval at once. They only mean
/// something when the hand could actually be in both places at the same time.
const SPAN_RULES: &[Rule] = &[
    Rule::Stretch,
    Rule::SmallSpan,
    Rule::LargeSpan,
    Rule::Impractical,
];

/// Scores trigrams against a rule set.
#[derive(Debug, Clone)]
pub struct RuleScorer {
    set: RuleSet,
    weights: RuleWeights,
    spans: &'static SpanTable,
    ruler: Ruler,
    /// The widest interval any finger pair can hold, in semitones. Beyond this the
    /// hand is travelling rather than stretching.
    reach: f32,
}

impl RuleScorer {
    /// Build a scorer.
    ///
    /// A consensus scorer scales its weights by how many of the published sets endorse
    /// each rule. That happens here rather than at the call site so the weighting can
    /// never be left off, or left on for a set it does not belong to.
    pub fn new(set: RuleSet, weights: RuleWeights, spans: &'static SpanTable, ruler: Ruler) -> Self {
        let reach = spans.0[Finger::Thumb.index()][Finger::Little.index()].max_prac as f32;
        let weights = match set {
            RuleSet::Consensus => weights.by_endorsement(),
            _ => weights,
        };
        Self { set, weights, spans, ruler, reach }
    }

    /// Whether two consecutive notes are further apart than the hand can span.
    ///
    /// This matters more than it looks. The published rules were written for legato
    /// melodic fragments, where consecutive notes lie under one hand position, and
    /// their span rules describe two fingers holding an interval. Applied to a leap
    /// of two octaves they report an impossible stretch — but nobody is stretching:
    /// the hand has let go and moved. Scoring a leap as a stretch produces penalties
    /// in the dozens and drowns out every other consideration.
    ///
    /// So beyond the hand's reach, the span rules fall silent and the cost of the
    /// leap is carried by the position-change rules and by the biomechanical
    /// transition term, which is what actually describes moving a hand.
    fn is_leap(&self, t: &Trigram) -> bool {
        match t.prev {
            Some(prev) => self.distance(prev.midi, t.current.midi).abs() > self.reach,
            None => false,
        }
    }

    /// The weights in force.
    pub fn weights(&self) -> &RuleWeights {
        &self.weights
    }

    /// Replace the weights, which is what tuning against a corpus does.
    pub fn set_weights(&mut self, weights: RuleWeights) {
        self.weights = weights;
    }

    /// Score a trigram, returning the per-rule breakdown.
    pub fn score(&self, t: &Trigram) -> RuleCosts {
        let mut costs = RuleCosts::default();
        for rule in self.set.rules() {
            let raw = self.raw(*rule, t);
            if raw != 0.0 {
                costs.per_rule[rule.index()] = raw * self.weights.get(*rule);
            }
        }
        costs
    }

    /// Total weighted cost of a trigram.
    pub fn total(&self, t: &Trigram) -> f32 {
        self.score(t).total()
    }

    /// Score a trigram against only the named rules, and only those the active rule
    /// set includes.
    ///
    /// Polyphony needs this: a chord's fingers are scored against each other, and
    /// the chord as a whole is scored against its neighbours, and the two passes
    /// must not both charge for the same thing.
    pub fn score_with(&self, t: &Trigram, only: &[Rule]) -> RuleCosts {
        let mut costs = RuleCosts::default();
        for rule in self.set.rules() {
            if !only.contains(rule) {
                continue;
            }
            let raw = self.raw(*rule, t);
            if raw != 0.0 {
                costs.per_rule[rule.index()] = raw * self.weights.get(*rule);
            }
        }
        costs
    }

    /// Score a trigram of consecutive notes, as opposed to notes played together.
    ///
    /// Identical to [`Self::score_excluding`] except that it drops the span rules
    /// across a leap, for the reason given on [`Self::is_leap`].
    pub fn score_melodic(&self, t: &Trigram, skip: &[Rule]) -> RuleCosts {
        let leap = self.is_leap(t);
        let mut costs = RuleCosts::default();
        for rule in self.set.rules() {
            if skip.contains(rule) || (leap && SPAN_RULES.contains(rule)) {
                continue;
            }
            let raw = self.raw(*rule, t);
            if raw != 0.0 {
                costs.per_rule[rule.index()] = raw * self.weights.get(*rule);
            }
        }
        costs
    }

    /// Score a trigram against every active rule except the named ones.
    pub fn score_excluding(&self, t: &Trigram, skip: &[Rule]) -> RuleCosts {
        let mut costs = RuleCosts::default();
        for rule in self.set.rules() {
            if skip.contains(rule) {
                continue;
            }
            let raw = self.raw(*rule, t);
            if raw != 0.0 {
                costs.per_rule[rule.index()] = raw * self.weights.get(*rule);
            }
        }
        costs
    }

    /// Distance between two pitches, in semitone equivalents.
    fn distance(&self, from: u8, to: u8) -> f32 {
        self.ruler.distance(from, to)
    }

    /// The unweighted penalty a rule charges.
    fn raw(&self, rule: Rule, t: &Trigram) -> f32 {
        match rule {
            Rule::Stretch => self.stretch(t),
            Rule::SmallSpan => self.small_span(t),
            Rule::LargeSpan => self.large_span(t),
            Rule::PositionChangeCount => self.position_change_count(t),
            Rule::PositionChangeSize => self.position_change_size(t),
            Rule::WeakFinger => self.weak_finger(t),
            Rule::ThreeFourFive => self.three_four_five(t),
            Rule::ThreeToFour => self.three_to_four(t),
            Rule::FourOnBlack => self.four_on_black(t),
            Rule::ThumbOnBlack => self.thumb_on_black(t),
            Rule::FiveOnBlack => self.five_on_black(t),
            Rule::ThumbPassing => self.thumb_passing(t),
            Rule::BalliauwPositionChange => self.balliauw_position_change(t),
            Rule::BalliauwPositionComfort => self.balliauw_position_comfort(t),
            Rule::RepeatFingerOnPositionChange => self.repeat_finger_on_position_change(t),
            Rule::Impractical => self.impractical(t),
            Rule::NonRepeatingFinger => self.non_repeating_finger(t),
            Rule::AlternationPairing => self.alternation_pairing(t),
            Rule::AlternationFingerChange => self.alternation_finger_change(t),
            Rule::BlackThumbPivot => self.black_thumb_pivot(t),
        }
    }

    /// Rule 1. Two points per semitone beyond the pair's comfortable span.
    fn stretch(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        let d = self.distance(prev.midi, t.current.midi);
        let span = self.spans.get(t.hand, prev.finger, t.current.finger);
        if d > span.max_comf as f32 {
            2.0 * (d - span.max_comf as f32)
        } else if d < span.min_comf as f32 {
            2.0 * (span.min_comf as f32 - d)
        } else {
            0.0
        }
    }

    /// Rule 2. Fingers crowded closer than they rest, or reaching across each other.
    ///
    /// One point per semitone when the thumb is involved, two otherwise. When the
    /// fingers are in their natural order this measures crowding; when they are
    /// crossed — a thumb passing under, or a finger reaching over it — the same
    /// arithmetic measures how far the crossing has to reach, which is what makes
    /// the search cross where a pianist crosses rather than wherever it likes.
    fn small_span(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        let (a, b) = (prev.finger, t.current.finger);
        if a == b {
            return 0.0;
        }
        let penalty = if a == Finger::Thumb || b == Finger::Thumb {
            1.0
        } else {
            self.set.small_span_penalty()
        };
        let d = self.distance(prev.midi, t.current.midi);
        let span = self.spans.get(t.hand, a, b);

        // Which end of the relaxed range is the tight one depends on whether the
        // finger numbers ascend or descend, and on which hand: the right hand's
        // finger numbers run up the keyboard, the left hand's run down.
        let ascending_fingers = a.index() < b.index();
        let tight_at_minimum = match t.hand {
            Hand::Right => ascending_fingers,
            Hand::Left => !ascending_fingers,
        };
        if tight_at_minimum {
            (span.min_rel as f32 - d).max(0.0) * penalty
        } else {
            (d - span.max_rel as f32).max(0.0) * penalty
        }
    }

    /// Rule 3. Fingers stretched wider than they rest, as the paper's text describes
    /// it rather than as its formal statement does.
    fn large_span(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        let (a, b) = (prev.finger, t.current.finger);
        let penalty = if a == Finger::Thumb || b == Finger::Thumb {
            1.0
        } else {
            self.set.large_span_penalty()
        };
        let Some(max_rel) = self.outward_max_rel(t, prev) else {
            return 0.0;
        };
        let d = self.distance(prev.midi, t.current.midi).abs();
        (d - max_rel).max(0.0) * penalty
    }

    /// The relaxed span of the pair, but only when the hand is opening out rather
    /// than crossing over itself.
    ///
    /// A hand lays its fingers along the keyboard in a fixed order: the right hand's
    /// thumb is its leftmost finger, the left hand's thumb its rightmost. So the two
    /// hands need opposite tests here, and the reference implementation's left-hand
    /// branch asks for rising finger numbers with rising pitch — the right hand's
    /// pattern — which means this rule never fires for a left hand in its natural
    /// shape. Corrected here, and covered by a test.
    fn outward_max_rel(&self, t: &Trigram, prev: Placement) -> Option<f32> {
        let (a, b) = (prev.finger, t.current.finger);
        if a == b || prev.midi == t.current.midi {
            return None;
        }
        let fingers_ascend = a.index() < b.index();
        let pitches_ascend = prev.midi < t.current.midi;
        let outward = match t.hand {
            Hand::Right => fingers_ascend == pitches_ascend,
            Hand::Left => fingers_ascend != pitches_ascend,
        };
        if !outward {
            return None;
        }
        // Read the pair from the finger on the lower note to the one on the higher,
        // so the threshold applies to a positive interval.
        let (low, high) = if pitches_ascend { (a, b) } else { (b, a) };
        Some(self.spans.get(t.hand, low, high).max_rel as f32)
    }

    /// Rule 4. Whether the hand changes position across the window: one point for a
    /// half change, two for a full one.
    ///
    /// Both this rule and the next ask how far the window's outer notes reach compared
    /// with what their two fingers comfortably straddle, which is the right question
    /// only when those are two different fingers; see [`Trigram::hand_only_travelled`]
    /// for when they are not.
    fn position_change_count(&self, t: &Trigram) -> f32 {
        let (Some(prev), Some(next)) = (t.prev, t.next) else {
            return 0.0;
        };
        if t.hand_only_travelled() {
            return 0.0;
        }
        let d13 = self.distance(prev.midi, next.midi);
        let span = self.spans.get(t.hand, prev.finger, next.finger);
        let thumb_between = t.current.finger == Finger::Thumb
            && is_between(t.current.midi, prev.midi, next.midi);

        if d13 > span.max_comf as f32 {
            if thumb_between && d13 > span.max_prac as f32 {
                2.0
            } else {
                1.0
            }
        } else if d13 < span.min_comf as f32 {
            if thumb_between && d13 < span.min_prac as f32 {
                2.0
            } else {
                1.0
            }
        } else {
            0.0
        }
    }

    /// Rule 5. How far outside the comfortable span the position change reaches.
    fn position_change_size(&self, t: &Trigram) -> f32 {
        let (Some(prev), Some(next)) = (t.prev, t.next) else {
            return 0.0;
        };
        if t.hand_only_travelled() {
            return 0.0;
        }
        let d13 = self.distance(prev.midi, next.midi);
        let span = self.spans.get(t.hand, prev.finger, next.finger);
        if d13 > span.max_comf as f32 {
            d13 - span.max_comf as f32
        } else if d13 < span.min_comf as f32 {
            span.min_comf as f32 - d13
        } else {
            0.0
        }
    }

    /// Rule 6. One point for using a weak finger.
    fn weak_finger(&self, t: &Trigram) -> f32 {
        let weak = if self.set.weak_is_only_four() {
            t.current.finger == Finger::Ring
        } else {
            t.current.finger.is_weak()
        };
        if weak {
            1.0
        } else {
            0.0
        }
    }

    /// Rule 7. One point when 3, 4 and 5 all appear in the window.
    fn three_four_five(&self, t: &Trigram) -> f32 {
        let mut seen = [false; 5];
        for p in [t.prev, Some(t.current), t.next].into_iter().flatten() {
            seen[p.finger.index()] = true;
        }
        if seen[Finger::Middle.index()] && seen[Finger::Ring.index()] && seen[Finger::Little.index()]
        {
            1.0
        } else {
            0.0
        }
    }

    /// Rule 8. One point when finger 3 hands straight over to finger 4.
    fn three_to_four(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        if prev.finger == Finger::Middle && t.current.finger == Finger::Ring {
            1.0
        } else {
            0.0
        }
    }

    /// Rule 9. Finger 4 on a black key immediately beside finger 3 on a white one.
    fn four_on_black(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        let a = prev.finger == Finger::Ring
            && is_black(prev.midi)
            && t.current.finger == Finger::Middle
            && is_white(t.current.midi);
        let b = prev.finger == Finger::Middle
            && is_white(prev.midi)
            && t.current.finger == Finger::Ring
            && is_black(t.current.midi);
        if a || b {
            1.0
        } else {
            0.0
        }
    }

    /// Rule 10. The thumb on a black key, and worse if it has to get there from, or
    /// leave for, a white one.
    fn thumb_on_black(&self, t: &Trigram) -> f32 {
        if t.current.finger != Finger::Thumb || is_white(t.current.midi) {
            return 0.0;
        }
        let mut cost = 1.0;
        if t.prev.is_some_and(|p| is_white(p.midi)) {
            cost += 2.0;
        }
        if t.next.is_some_and(|n| is_white(n.midi)) {
            cost += 2.0;
        }
        cost
    }

    /// Rule 11. The little finger on a black key, unless its neighbours are black
    /// too and the hand is already up among them.
    fn five_on_black(&self, t: &Trigram) -> f32 {
        if t.current.finger != Finger::Little || is_white(t.current.midi) {
            return 0.0;
        }
        let prev_black = t.prev.map(|p| is_black(p.midi));
        let next_black = t.next.map(|n| is_black(n.midi));
        if prev_black == Some(true) && next_black == Some(true) {
            return 0.0;
        }
        let mut cost = 0.0;
        if prev_black == Some(false) {
            cost += 2.0;
        }
        if next_black == Some(false) {
            cost += 2.0;
        }
        cost
    }

    /// Rule 12. The thumb passing under a finger, or a finger crossing over it. Free
    /// when both notes are the same colour, expensive when the thumb has to climb.
    ///
    /// Both notes have to involve the thumb and something else. The reference
    /// implementation tests only the direction and one of the two fingers, so it
    /// charges a thumb-to-thumb succession as a pass — but a thumb moving from one
    /// note to another with no finger over it is just the hand moving, and charging
    /// it distorts the fingering of whatever chord follows.
    fn thumb_passing(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        if prev.finger == t.current.finger {
            return 0.0;
        }
        let (m1, m2) = (prev.midi, t.current.midi);
        let same_level = is_black(m1) == is_black(m2);
        let bad = self.set.bad_level_change_cost();

        // The right hand crosses over its thumb going down and passes under going
        // up; the left hand does the opposite.
        let (crossing_over, passing_under) = match t.hand {
            Hand::Right => (m2 < m1, m2 > m1),
            Hand::Left => (m2 > m1, m2 < m1),
        };

        if prev.finger == Finger::Thumb && crossing_over {
            return if same_level {
                1.0
            } else if is_black(m1) {
                bad
            } else {
                0.0
            };
        }
        if t.current.finger == Finger::Thumb && passing_under {
            return if same_level {
                1.0
            } else if is_black(m2) {
                bad
            } else {
                0.0
            };
        }
        0.0
    }

    /// Balliauw's position change: one point for leaving the comfortable span, two
    /// if the thumb is passing between notes it cannot practically reach across, and
    /// one more if the window returns to the same pitch on a different finger.
    fn balliauw_position_change(&self, t: &Trigram) -> f32 {
        let (Some(prev), Some(next)) = (t.prev, t.next) else {
            return 0.0;
        };
        // Same reason as the Parncutt pair these replace: three notes on one finger is
        // a hand that kept its shape and moved, and a finger spans exactly zero against
        // itself, so reading that as a comfort window makes every step of a settled
        // hand a position change. A run of octaves collected it as noise.
        if t.hand_only_travelled() {
            return 0.0;
        }
        let d13 = self.distance(prev.midi, next.midi);
        let span = self.spans.get(t.hand, prev.finger, next.finger);
        let mut cost = 0.0;
        if d13 < span.min_comf as f32 || d13 > span.max_comf as f32 {
            cost += 1.0;
            let ascending_through = prev.midi < t.current.midi && t.current.midi < next.midi;
            if ascending_through
                && t.current.finger == Finger::Thumb
                && (d13 < span.min_prac as f32 || d13 > span.max_prac as f32)
            {
                cost += 1.0;
            }
        }
        if prev.midi == next.midi && prev.finger != next.finger {
            cost += 1.0;
        }
        cost
    }

    /// Balliauw's position comfort, replacing Parncutt's position-change size.
    fn balliauw_position_comfort(&self, t: &Trigram) -> f32 {
        let (Some(prev), Some(next)) = (t.prev, t.next) else {
            return 0.0;
        };
        // Same reason as the Parncutt pair these replace: three notes on one finger is
        // a hand that kept its shape and moved, and a finger spans exactly zero against
        // itself, so reading that as a comfort window makes every step of a settled
        // hand a position change. A run of octaves collected it as noise.
        if t.hand_only_travelled() {
            return 0.0;
        }
        let d13 = self.distance(prev.midi, next.midi);
        let span = self.spans.get(t.hand, prev.finger, next.finger);
        if d13 > span.max_comf as f32 {
            d13 - span.max_comf as f32
        } else if d13 < span.min_comf as f32 {
            // From the bound that was crossed, not the far one. Measured against
            // `max_comf`, a too-narrow interval is charged the whole width of the
            // window and then drops to nothing the instant it reaches `min_comf`: for
            // a pair whose window is two to eight semitones that is a seven-point step
            // between two fingerings a semitone apart, which is more than an impossible
            // stretch costs. Every sibling rule measures from the bound it crossed.
            span.min_comf as f32 - d13
        } else {
            0.0
        }
    }

    /// Reusing the same finger for different notes across a position change.
    fn repeat_finger_on_position_change(&self, t: &Trigram) -> f32 {
        let (Some(prev), Some(next)) = (t.prev, t.next) else {
            return 0.0;
        };
        if prev.midi != next.midi
            && prev.finger == next.finger
            && self.position_change_count(t) > 0.0
        {
            1.0
        } else {
            0.0
        }
    }

    /// An interval the finger pair cannot span at all. Weighted heavily, so the
    /// solver treats it as very nearly a hard constraint without ever being unable
    /// to return an answer.
    fn impractical(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        // One finger cannot be in two places. Every other pair of fingers spans some
        // range and the charge is how far past it the interval reaches, which is the
        // right shape for a stretch — but a finger against itself spans exactly nothing,
        // so that arithmetic charges a semitone step almost nothing at all. It is not a
        // small stretch, it is a different thing: the finger has to leave one key to
        // reach the other, and a line played that way is not legato however close the
        // two notes are.
        //
        // Repeating a pitch is the exception and is what a finger is *supposed* to do
        // there, so it is left alone.
        if prev.finger == t.current.finger {
            return if prev.midi == t.current.midi { 0.0 } else { SAME_FINGER_STEP };
        }
        let span = self.spans.get(t.hand, prev.finger, t.current.finger);
        let (lo, hi) = (span.min_prac as f32, span.max_prac as f32);

        // The two ends of a span mean different things, and only one of them is a
        // distance.
        //
        // The end far from zero says how far the hand can stretch, which is a question
        // about millimetres and is asked in them. The end *near* zero says the two
        // fingers have to be on different keys, which is a question about keys and has
        // to be asked by counting them. Which end is which depends on the pair: from
        // the second finger to the third the bounds run 1 to 5, and from the third to
        // the second they run -5 to -1, so the adjacency bound is the lower one going
        // up and the upper one coming down.
        //
        // Asking the near bound in millimetres was a mistake, and an expensive one. A
        // semitone averages 13.7 mm but is only 11.75 mm from a white key to the black
        // one beside it, so an ordinary semitone measured 0.86 of one and fell outside
        // a bound of 1. This rule carries ten times the weight of the others precisely
        // so the search treats it as very nearly forbidden — so playing G with the
        // second finger and A flat with the third, which is what every edition of every
        // flat-key scale asks for, was scored as a thing a hand cannot do. Real music is
        // full of semitones taken by adjacent fingers with one of the two keys black,
        // and every one of them was charged.
        let far = self.distance(prev.midi, t.current.midi);
        let near = Ruler::Chromatic.distance(prev.midi, t.current.midi);
        // Two semitones catches every separation bound in the tables and no stretch
        // bound: they run 1 or 2 where they are minima and -1 or -2 where they are
        // maxima, and the next value in either direction is 3 away from zero. It has to
        // reach 2, because a whole tone is 23.5 mm wherever it falls and that is 1.71
        // semitone-widths — so a pair told it needs two of them, which is the second
        // finger against the fifth, was charged for every whole tone in the piece.
        let adjacency = |bound: f32| bound.abs() <= 2.0;

        let above = if adjacency(hi) { near } else { far };
        if above > hi {
            return above - hi;
        }
        let below = if adjacency(lo) { near } else { far };
        if below < lo {
            return lo - below;
        }
        0.0
    }

    /// A repeated pitch that changes finger for no reason.
    fn non_repeating_finger(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        if prev.midi == t.current.midi && prev.finger != t.current.finger {
            1.0
        } else {
            0.0
        }
    }

    /// Badgerow: trilling between two adjacent weak fingers.
    fn alternation_pairing(&self, t: &Trigram) -> f32 {
        let (Some(prev), Some(next)) = (t.prev, t.next) else {
            return 0.0;
        };
        let pattern = (
            prev.finger.number(),
            t.current.finger.number(),
            next.finger.number(),
        );
        if matches!(pattern, (3, 4, 3) | (4, 3, 4) | (4, 5, 4) | (5, 4, 5)) {
            1.0
        } else {
            0.0
        }
    }

    /// Badgerow: coming back to the same pitch on a different finger.
    fn alternation_finger_change(&self, t: &Trigram) -> f32 {
        let (Some(prev), Some(next)) = (t.prev, t.next) else {
            return 0.0;
        };
        if prev.midi == next.midi && prev.finger != next.finger {
            1.0
        } else {
            0.0
        }
    }

    /// Badgerow: pivoting between a thumb on a black key and a weak finger on a
    /// white one, which forces the hand to rock in and out at the same time as it
    /// stretches.
    ///
    /// The reference implementation charges `digit_2 - 1` here, which evaluates to
    /// zero in the branches where digit 2 *is* the thumb. Taking the evident intent,
    /// this charges by the weak finger's number in both directions.
    fn black_thumb_pivot(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        if prev.midi == t.current.midi {
            return 0.0;
        }
        let away_from_thumb = match t.hand {
            Hand::Right => t.current.midi < prev.midi,
            Hand::Left => t.current.midi > prev.midi,
        };
        if away_from_thumb {
            if prev.finger == Finger::Thumb
                && is_black(prev.midi)
                && t.current.finger.is_weak()
                && is_white(t.current.midi)
            {
                return (t.current.finger.number() - 1) as f32;
            }
        } else if t.current.finger == Finger::Thumb
            && is_black(t.current.midi)
            && prev.finger.is_weak()
            && is_white(prev.midi)
        {
            return (prev.finger.number() - 1) as f32;
        }
        0.0
    }
}

/// Whether `mid` lies strictly between the other two, in either direction.
fn is_between(mid: u8, a: u8, b: u8) -> bool {
    (a < mid && mid < b) || (b < mid && mid < a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_consensus_charges_every_rule_any_published_set_charges() {
        for set in PUBLISHED {
            for rule in set.rules() {
                assert!(
                    RuleSet::Consensus.rules().contains(rule),
                    "{rule:?} is in {set:?} but not in the consensus"
                );
            }
        }
    }

    #[test]
    fn a_rule_every_set_endorses_outweighs_one_only_a_single_set_uses() {
        // Stretch is in all four; the alternation rules are Badgerow's alone.
        assert_eq!(RuleSet::endorsements(Rule::Stretch), 4);
        assert_eq!(RuleSet::endorsements(Rule::AlternationPairing), 1);

        let weights = RuleWeights::default().by_endorsement();
        let shared = weights.get(Rule::Stretch);
        let lone = weights.get(Rule::AlternationPairing);
        assert!(
            shared > lone * 3.5,
            "a unanimous rule should carry about four times a lone one: {shared} vs {lone}"
        );
    }

    #[test]
    fn the_consensus_numbers_sit_between_the_sets_that_disagree() {
        // Jacobs halves the large-span penalty and the other three do not, so the
        // consensus has to land between them rather than picking a side.
        let jacobs = RuleSet::Jacobs.large_span_penalty();
        let others = RuleSet::Parncutt.large_span_penalty();
        let consensus = RuleSet::Consensus.large_span_penalty();
        assert!(
            jacobs < consensus && consensus < others,
            "{jacobs} < {consensus} < {others}"
        );
    }
    use crate::spans::PARNCUTT;

    fn scorer(set: RuleSet) -> RuleScorer {
        RuleScorer::new(set, RuleWeights::default(), &PARNCUTT, Ruler::Chromatic)
    }

    fn trigram(hand: Hand, a: (u8, u8), b: (u8, u8), c: (u8, u8)) -> Trigram {
        let place = |(m, f): (u8, u8)| Placement::new(m, Finger::from_number(f).unwrap());
        Trigram::new(hand, place(a), place(b), place(c))
    }

    #[test]
    fn the_stretch_rule_reproduces_the_papers_worked_example() {
        // Fingers 2 then 5 playing D4 to C5: ten semitones against a maximum
        // comfortable eight, at two points per semitone.
        let s = scorer(RuleSet::Parncutt);
        let t = Trigram {
            hand: Hand::Right,
            prev: Some(Placement::new(62, Finger::Index)),
            current: Placement::new(72, Finger::Little),
            next: None,
        };
        assert_eq!(s.stretch(&t), 4.0);
    }

    #[test]
    fn a_comfortable_interval_costs_nothing_to_stretch() {
        let s = scorer(RuleSet::Parncutt);
        // Thumb to index, a major third up: well inside the relaxed range.
        let t = Trigram {
            hand: Hand::Right,
            prev: Some(Placement::new(60, Finger::Thumb)),
            current: Placement::new(64, Finger::Index),
            next: None,
        };
        assert_eq!(s.stretch(&t), 0.0);
        assert_eq!(s.small_span(&t), 0.0);
        assert_eq!(s.large_span(&t), 0.0);
    }

    #[test]
    fn the_thumb_is_penalised_on_black_keys_between_white_ones() {
        let s = scorer(RuleSet::Parncutt);
        // Thumb on F#, arriving from F and leaving for G: one point plus two each side.
        let t = trigram(Hand::Right, (65, 2), (66, 1), (67, 2));
        assert_eq!(s.thumb_on_black(&t), 5.0);

        // Same thumb, but among the black keys, so only the base point.
        let t = trigram(Hand::Right, (61, 2), (66, 1), (68, 2));
        assert_eq!(s.thumb_on_black(&t), 1.0);

        // On a white key it is free.
        let t = trigram(Hand::Right, (65, 2), (67, 1), (69, 2));
        assert_eq!(s.thumb_on_black(&t), 0.0);
    }

    #[test]
    fn the_little_finger_on_a_black_key_is_free_only_among_black_keys() {
        let s = scorer(RuleSet::Parncutt);
        let among_black = trigram(Hand::Right, (61, 2), (66, 5), (68, 2));
        assert_eq!(s.five_on_black(&among_black), 0.0);
        let among_white = trigram(Hand::Right, (60, 2), (66, 5), (67, 2));
        assert_eq!(s.five_on_black(&among_white), 4.0);
    }

    #[test]
    fn the_thumb_passing_rule_knows_which_hand_it_is() {
        let s = scorer(RuleSet::Parncutt);
        // Right hand ascending, thumb passing under from E to F: same colour, one point.
        let t = Trigram {
            hand: Hand::Right,
            prev: Some(Placement::new(64, Finger::Middle)),
            current: Placement::new(65, Finger::Thumb),
            next: None,
        };
        assert_eq!(s.thumb_passing(&t), 1.0);

        // The same shape in the left hand is not a thumb pass at all: the left hand
        // passes its thumb going down.
        let t = Trigram { hand: Hand::Left, ..t };
        assert_eq!(s.thumb_passing(&t), 0.0);

        // Left hand descending onto the thumb: one point.
        let t = Trigram {
            hand: Hand::Left,
            prev: Some(Placement::new(65, Finger::Middle)),
            current: Placement::new(64, Finger::Thumb),
            next: None,
        };
        assert_eq!(s.thumb_passing(&t), 1.0);
    }

    #[test]
    fn the_thumb_moving_to_another_note_by_itself_is_not_a_pass() {
        let s = scorer(RuleSet::Parncutt);
        // Thumb on C4, then thumb on G3: the hand has moved, but nothing was
        // crossed and nothing passed under anything.
        let t = Trigram {
            hand: Hand::Left,
            prev: Some(Placement::new(60, Finger::Thumb)),
            current: Placement::new(55, Finger::Thumb),
            next: None,
        };
        assert_eq!(s.thumb_passing(&t), 0.0);
    }

    #[test]
    fn passing_the_thumb_onto_a_black_key_costs_more() {
        let s = scorer(RuleSet::Parncutt);
        // Right hand ascending from E to F#, thumb landing on the black key.
        let t = Trigram {
            hand: Hand::Right,
            prev: Some(Placement::new(64, Finger::Middle)),
            current: Placement::new(66, Finger::Thumb),
            next: None,
        };
        assert_eq!(s.thumb_passing(&t), 3.0);
    }

    #[test]
    fn jacobs_considers_only_the_ring_finger_weak() {
        let parncutt = scorer(RuleSet::Parncutt);
        let jacobs = scorer(RuleSet::Jacobs);
        let little = trigram(Hand::Right, (60, 1), (67, 5), (69, 4));
        assert_eq!(parncutt.weak_finger(&little), 1.0);
        assert_eq!(jacobs.weak_finger(&little), 0.0);

        let ring = trigram(Hand::Right, (60, 1), (67, 4), (69, 5));
        assert_eq!(parncutt.weak_finger(&ring), 1.0);
        assert_eq!(jacobs.weak_finger(&ring), 1.0);
    }

    #[test]
    fn three_four_five_fires_only_when_all_three_are_present() {
        let s = scorer(RuleSet::Parncutt);
        assert_eq!(s.three_four_five(&trigram(Hand::Right, (60, 3), (62, 4), (64, 5))), 1.0);
        assert_eq!(s.three_four_five(&trigram(Hand::Right, (60, 5), (62, 3), (64, 4))), 1.0);
        assert_eq!(s.three_four_five(&trigram(Hand::Right, (60, 2), (62, 3), (64, 4))), 0.0);
    }

    #[test]
    fn impossible_spans_are_charged_heavily() {
        let s = scorer(RuleSet::Balliauw);
        // Fingers 4 then 5 asked to span an octave: nowhere near practical.
        let t = Trigram {
            hand: Hand::Right,
            prev: Some(Placement::new(60, Finger::Ring)),
            current: Placement::new(72, Finger::Little),
            next: None,
        };
        assert!(s.impractical(&t) > 0.0);
        let costs = s.score(&t);
        assert!(
            costs.per_rule[Rule::Impractical.index()] >= 10.0,
            "an impossible span should dominate: {costs:?}"
        );
    }

    #[test]
    fn a_repeated_note_keeping_its_finger_is_free() {
        let s = scorer(RuleSet::Balliauw);
        let same = Trigram {
            hand: Hand::Right,
            prev: Some(Placement::new(60, Finger::Middle)),
            current: Placement::new(60, Finger::Middle),
            next: None,
        };
        assert_eq!(s.non_repeating_finger(&same), 0.0);
        let changed = Trigram {
            current: Placement::new(60, Finger::Index),
            ..same
        };
        assert_eq!(s.non_repeating_finger(&changed), 1.0);
    }

    #[test]
    fn badgerow_penalises_trilling_on_weak_finger_pairs() {
        let s = scorer(RuleSet::Badgerow);
        assert_eq!(s.alternation_pairing(&trigram(Hand::Right, (60, 4), (62, 5), (60, 4))), 1.0);
        assert_eq!(s.alternation_pairing(&trigram(Hand::Right, (60, 1), (62, 2), (60, 1))), 0.0);
    }

    #[test]
    fn every_rule_set_scores_a_plain_scale_step_cheaply() {
        // C to D with fingers 1 then 2 in the right hand is about as ordinary as
        // piano playing gets; no rule set should object.
        for set in [RuleSet::Parncutt, RuleSet::Jacobs, RuleSet::Balliauw, RuleSet::Badgerow] {
            let s = scorer(set);
            let t = trigram(Hand::Right, (60, 1), (62, 2), (64, 3));
            let total = s.total(&t);
            assert!(total < 1.5, "{set:?} charged {total} for a plain scale step");
        }
    }

    #[test]
    fn weights_scale_the_penalties_they_name() {
        let mut weights = RuleWeights::default();
        weights.set(Rule::WeakFinger, 3.0);
        let s = RuleScorer::new(RuleSet::Parncutt, weights, &PARNCUTT, Ruler::Chromatic);
        let t = trigram(Hand::Right, (60, 1), (67, 5), (69, 3));
        assert_eq!(s.score(&t).per_rule[Rule::WeakFinger.index()], 3.0);
    }

    #[test]
    fn the_breakdown_names_what_fired() {
        let s = scorer(RuleSet::Parncutt);
        let t = trigram(Hand::Right, (65, 2), (66, 1), (67, 2));
        let fired = s.score(&t).fired();
        assert!(fired.iter().any(|(r, _)| *r == Rule::ThumbOnBlack));
        // Sorted worst first.
        assert!(fired.windows(2).all(|w| w[0].1 >= w[1].1));
    }

    #[test]
    fn a_hand_that_only_travels_has_not_changed_position() {
        // C, D, E played by the little finger throughout: the top of a run of octaves.
        // The hand slides two whole tones and keeps its shape, which is travel, not a
        // reorganisation. Reading the span of a finger against itself — zero either way
        // — as a comfortable range makes every step look like a position change.
        let costs = scorer(RuleSet::Parncutt).score(&trigram(Hand::Right, (72, 5), (74, 5), (76, 5)));
        assert_eq!(costs.per_rule[Rule::PositionChangeCount.index()], 0.0);
        assert_eq!(costs.per_rule[Rule::PositionChangeSize.index()], 0.0);
    }

    #[test]
    fn a_thumb_that_comes_back_has_changed_position() {
        // F, E, D taken 1-2-1, descending. The same finger at both ends of the window,
        // but the thumb has had to leave one key and find another three semitones away
        // while another finger played in between: the hand really did relocate, and
        // that is the case the rule exists for.
        let costs = scorer(RuleSet::Parncutt).score(&trigram(Hand::Right, (65, 1), (64, 2), (62, 1)));
        assert!(costs.per_rule[Rule::PositionChangeCount.index()] > 0.0);
    }

    #[test]
    fn three_different_fingers_still_report_a_position_change() {
        // C, D and the A flat above, taken 5-4-1: the thumb has to come right round.
        let costs = scorer(RuleSet::Parncutt).score(&trigram(Hand::Right, (72, 5), (74, 4), (88, 1)));
        assert!(costs.per_rule[Rule::PositionChangeCount.index()] > 0.0);
        assert!(costs.per_rule[Rule::PositionChangeSize.index()] > 0.0);
    }

    #[test]
    fn an_ordinary_semitone_between_adjacent_fingers_is_not_impossible() {
        // The physical ruler is the one that ships, and it measures the keyboard rather
        // than counting semitones. G to A flat is 11.75 mm, which is 0.86 of an average
        // semitone — so a bound written as "at least 1" was failed by the most ordinary
        // interval in flat-key playing, and `Impractical` carries ten times the weight
        // of every other rule.
        let physical = RuleScorer::new(
            RuleSet::Consensus,
            RuleWeights::default(),
            &PARNCUTT,
            Ruler::Physical,
        );
        // G with the second finger, A flat with the third: every edition of E flat major.
        let up = trigram(Hand::Right, (65, 1), (67, 2), (68, 3));
        assert_eq!(physical.impractical(&up), 0.0, "G to A flat, fingers 2 to 3");
        // And coming back down, where the same bound is the pair's upper one.
        let down = trigram(Hand::Right, (70, 4), (68, 3), (67, 2));
        assert_eq!(physical.impractical(&down), 0.0, "A flat to G, fingers 3 to 2");

        // A whole tone between the second finger and the fifth is 23.5 mm, which is
        // 1.71 average semitones against a bound of 2.
        let wide = trigram(Hand::Right, (60, 1), (62, 2), (64, 5));
        assert_eq!(physical.impractical(&wide), 0.0, "D to E, fingers 2 to 5");

        // What the rule is actually for still fires: two fingers that cannot reach.
        // The charge is on the step into the middle note, so the leap goes there.
        let far = trigram(Hand::Right, (60, 4), (84, 5), (86, 5));
        assert!(
            physical.impractical(&far) > 0.0,
            "a two-octave leap between the fourth and fifth fingers is impossible"
        );
    }

    #[test]
    fn the_position_rules_have_no_cliff_in_them() {
        // A cost function the search can reason about has to be continuous: two
        // fingerings a semitone apart in hand position must not be separated by more
        // than the rule's own slope. `balliauw_position_comfort` measured a too-narrow
        // interval against the far end of its window, so for the pair (2, 5) the charge
        // fell from seven to zero in one semitone — more than an impossible stretch
        // costs, and enough to drown every other term deciding the passage.
        let s = scorer(RuleSet::Balliauw);
        for (from, to) in [(2u8, 5u8), (1, 3), (3, 4), (2, 3)] {
            for rule in [Rule::BalliauwPositionComfort, Rule::PositionChangeSize] {
                let at = |apart: u8| {
                    let middle = if from == 1 { 2 } else { 1 };
                    let t = trigram(Hand::Right, (60, from), (61, middle), (60 + apart, to));
                    s.score_with(&t, &[rule]).total()
                };
                for apart in 0..14u8 {
                    let (here, next) = (at(apart), at(apart + 1));
                    assert!(
                        (next - here).abs() <= 2.5,
                        "{rule:?} jumps from {here} to {next} between {apart} and {}                          semitones for the pair ({from}, {to})",
                        apart + 1
                    );
                }
            }
        }
    }
}
