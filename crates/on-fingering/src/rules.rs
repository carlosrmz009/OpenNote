use on_hand::keyboard::{is_black, is_white};
use on_hand::{Finger, Hand};
use serde::{Deserialize, Serialize};

use crate::ruler::Ruler;
use crate::spans::SpanTable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub midi: u8,
    pub finger: Finger,
}

impl Placement {
    pub fn new(midi: u8, finger: Finger) -> Self {
        Self { midi, finger }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trigram {
    pub hand: Hand,
    pub prev: Option<Placement>,
    pub current: Placement,
    pub next: Option<Placement>,
}

impl Trigram {
    fn hand_only_travelled(&self) -> bool {
        matches!(
            (self.prev, self.next),
            (Some(prev), Some(next))
                if prev.finger == self.current.finger && self.current.finger == next.finger
        )
    }

    pub fn new(hand: Hand, prev: Placement, current: Placement, next: Placement) -> Self {
        Self { hand, prev: Some(prev), current, next: Some(next) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rule {
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
    RepeatedFinger,
}

impl Rule {
    pub const ALL: [Rule; 21] = [
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
        Rule::RepeatedFinger,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

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
            Rule::RepeatedFinger => "rep",
        }
    }

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
            Rule::FourOnBlack => "finger 3 is on a black key beside finger 4 on a white one",
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
            Rule::RepeatedFinger => "one finger has to play two different notes in a row",
        }
    }
}

pub const RULE_COUNT: usize = Rule::ALL.len();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RuleSet {
    Parncutt,
    Jacobs,
    Balliauw,
    Badgerow,
    #[default]
    Consensus,
}

const SAME_FINGER_STEP: f32 = 3.0;

pub const UNIVERSAL_RULES: [Rule; 1] = [Rule::RepeatedFinger];

const HARD_CONSTRAINTS: [Rule; 2] = [Rule::Impractical, Rule::RepeatedFinger];

pub const PUBLISHED: [RuleSet; 4] =
    [RuleSet::Parncutt, RuleSet::Jacobs, RuleSet::Balliauw, RuleSet::Badgerow];

impl RuleSet {
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
                RepeatedFinger,
            ],
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
                RepeatedFinger,
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
                RepeatedFinger,
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
                RepeatedFinger,
            ],
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
                RepeatedFinger,
            ],
        }
    }

    pub fn endorsements(rule: Rule) -> usize {
        PUBLISHED
            .iter()
            .filter(|set| set.rules().contains(&rule))
            .count()
    }

    fn weak_is_only_four(self) -> bool {
        matches!(self, RuleSet::Jacobs)
    }

    fn bad_level_change_cost(self) -> f32 {
        match self {
            RuleSet::Balliauw => 2.0,
            RuleSet::Consensus => 2.75,
            _ => 3.0,
        }
    }

    fn large_span_penalty(self) -> f32 {
        match self {
            RuleSet::Jacobs => 1.0,
            RuleSet::Consensus => 1.75,
            _ => 2.0,
        }
    }

    fn small_span_penalty(self) -> f32 {
        match self {
            RuleSet::Balliauw => 1.0,
            RuleSet::Consensus => 1.75,
            _ => 2.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RuleWeights(pub [f32; RULE_COUNT]);

impl Default for RuleWeights {
    fn default() -> Self {
        let mut w = [1.0; RULE_COUNT];
        w[Rule::Impractical.index()] = 10.0;
        w[Rule::RepeatedFinger.index()] = 10.0;
        RuleWeights(w)
    }
}

impl RuleWeights {
    pub fn by_endorsement(self) -> Self {
        let mut weights = self;
        for rule in Rule::ALL {
            if HARD_CONSTRAINTS.contains(&rule) {
                continue;
            }
            let share = RuleSet::endorsements(rule) as f32 / PUBLISHED.len() as f32;
            let share = if share == 0.0 { 1.0 } else { share };
            weights.0[rule.index()] *= share;
        }
        weights
    }
}

impl RuleWeights {
    pub fn get(&self, rule: Rule) -> f32 {
        self.0[rule.index()]
    }

    pub fn set(&mut self, rule: Rule, weight: f32) {
        self.0[rule.index()] = weight;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuleCosts {
    pub per_rule: [f32; RULE_COUNT],
}

impl Default for RuleCosts {
    fn default() -> Self {
        Self { per_rule: [0.0; RULE_COUNT] }
    }
}

impl RuleCosts {
    pub fn total(&self) -> f32 {
        self.per_rule.iter().sum()
    }

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

const SPAN_RULES: &[Rule] = &[
    Rule::Stretch,
    Rule::SmallSpan,
    Rule::LargeSpan,
    Rule::Impractical,
];

#[derive(Debug, Clone)]
pub struct RuleScorer {
    set: RuleSet,
    weights: RuleWeights,
    spans: &'static SpanTable,
    ruler: Ruler,
    reach: f32,
}

impl RuleScorer {
    pub fn new(set: RuleSet, weights: RuleWeights, spans: &'static SpanTable, ruler: Ruler) -> Self {
        let reach = spans.0[Finger::Thumb.index()][Finger::Little.index()].max_prac as f32;
        let weights = match set {
            RuleSet::Consensus => weights.by_endorsement(),
            _ => weights,
        };
        Self { set, weights, spans, ruler, reach }
    }

    fn is_leap(&self, t: &Trigram) -> bool {
        match t.prev {
            Some(prev) => self.distance(prev.midi, t.current.midi).abs() > self.reach,
            None => false,
        }
    }

    pub fn weights(&self) -> &RuleWeights {
        &self.weights
    }

    pub fn set_weights(&mut self, weights: RuleWeights) {
        self.weights = weights;
    }

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

    pub fn total(&self, t: &Trigram) -> f32 {
        self.score(t).total()
    }

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

    fn distance(&self, from: u8, to: u8) -> f32 {
        self.ruler.distance(from, to)
    }

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
            Rule::RepeatedFinger => self.repeated_finger(t),
        }
    }

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
        let (low, high) = if pitches_ascend { (a, b) } else { (b, a) };
        Some(self.spans.get(t.hand, low, high).max_rel as f32)
    }

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

    fn three_to_four(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        if prev.finger == Finger::Middle && t.current.finger == Finger::Ring {
            1.0
        } else {
            0.0
        }
    }

    fn four_on_black(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        let awkward = |a: Placement, b: Placement| {
            a.finger == Finger::Middle
                && is_black(a.midi)
                && b.finger == Finger::Ring
                && is_white(b.midi)
        };
        if awkward(prev, t.current) || awkward(t.current, prev) {
            1.0
        } else {
            0.0
        }
    }

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

    fn thumb_passing(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        if prev.finger == t.current.finger {
            return 0.0;
        }
        let (m1, m2) = (prev.midi, t.current.midi);
        let same_level = is_black(m1) == is_black(m2);
        let bad = self.set.bad_level_change_cost();

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

    fn balliauw_position_change(&self, t: &Trigram) -> f32 {
        let (Some(prev), Some(next)) = (t.prev, t.next) else {
            return 0.0;
        };
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

    fn balliauw_position_comfort(&self, t: &Trigram) -> f32 {
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

    fn repeated_finger(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        if prev.finger == t.current.finger && prev.midi != t.current.midi {
            SAME_FINGER_STEP
        } else {
            0.0
        }
    }

    fn impractical(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        if prev.finger == t.current.finger {
            return 0.0;
        }
        let span = self.spans.get(t.hand, prev.finger, t.current.finger);
        let (lo, hi) = (span.min_prac as f32, span.max_prac as f32);

        let far = self.distance(prev.midi, t.current.midi);
        let near = Ruler::Chromatic.distance(prev.midi, t.current.midi);
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

    fn non_repeating_finger(&self, t: &Trigram) -> f32 {
        let Some(prev) = t.prev else { return 0.0 };
        if prev.midi == t.current.midi && prev.finger != t.current.finger {
            1.0
        } else {
            0.0
        }
    }

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
        let t = trigram(Hand::Right, (65, 2), (66, 1), (67, 2));
        assert_eq!(s.thumb_on_black(&t), 5.0);

        let t = trigram(Hand::Right, (61, 2), (66, 1), (68, 2));
        assert_eq!(s.thumb_on_black(&t), 1.0);

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
        let t = Trigram {
            hand: Hand::Right,
            prev: Some(Placement::new(64, Finger::Middle)),
            current: Placement::new(65, Finger::Thumb),
            next: None,
        };
        assert_eq!(s.thumb_passing(&t), 1.0);

        let t = Trigram { hand: Hand::Left, ..t };
        assert_eq!(s.thumb_passing(&t), 0.0);

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
        assert!(fired.windows(2).all(|w| w[0].1 >= w[1].1));
    }

    #[test]
    fn a_hand_that_only_travels_has_not_changed_position() {
        let costs = scorer(RuleSet::Parncutt).score(&trigram(Hand::Right, (72, 5), (74, 5), (76, 5)));
        assert_eq!(costs.per_rule[Rule::PositionChangeCount.index()], 0.0);
        assert_eq!(costs.per_rule[Rule::PositionChangeSize.index()], 0.0);
    }

    #[test]
    fn a_thumb_that_comes_back_has_changed_position() {
        let costs = scorer(RuleSet::Parncutt).score(&trigram(Hand::Right, (65, 1), (64, 2), (62, 1)));
        assert!(costs.per_rule[Rule::PositionChangeCount.index()] > 0.0);
    }

    #[test]
    fn three_different_fingers_still_report_a_position_change() {
        let costs = scorer(RuleSet::Parncutt).score(&trigram(Hand::Right, (72, 5), (74, 4), (88, 1)));
        assert!(costs.per_rule[Rule::PositionChangeCount.index()] > 0.0);
        assert!(costs.per_rule[Rule::PositionChangeSize.index()] > 0.0);
    }

    #[test]
    fn an_ordinary_semitone_between_adjacent_fingers_is_not_impossible() {
        let physical = RuleScorer::new(
            RuleSet::Consensus,
            RuleWeights::default(),
            &PARNCUTT,
            Ruler::Physical,
        );
        let up = trigram(Hand::Right, (65, 1), (67, 2), (68, 3));
        assert_eq!(physical.impractical(&up), 0.0, "G to A flat, fingers 2 to 3");
        let down = trigram(Hand::Right, (70, 4), (68, 3), (67, 2));
        assert_eq!(physical.impractical(&down), 0.0, "A flat to G, fingers 3 to 2");

        let wide = trigram(Hand::Right, (60, 1), (62, 2), (64, 5));
        assert_eq!(physical.impractical(&wide), 0.0, "D to E, fingers 2 to 5");

        let far = trigram(Hand::Right, (60, 4), (84, 5), (86, 5));
        assert!(
            physical.impractical(&far) > 0.0,
            "a two-octave leap between the fourth and fifth fingers is impossible"
        );
    }

    #[test]
    fn the_position_rules_have_no_cliff_in_them() {
        for (rule, set) in [
            (Rule::BalliauwPositionComfort, RuleSet::Balliauw),
            (Rule::PositionChangeSize, RuleSet::Parncutt),
        ] {
            let s = scorer(set);
            for (from, to) in [(2u8, 5u8), (1, 3), (3, 4), (2, 3)] {
                let at = |apart: u8| {
                    let middle = if from == 1 { 2 } else { 1 };
                    let t = trigram(Hand::Right, (60, from), (61, middle), (60 + apart, to));
                    s.score_with(&t, &[rule]).total()
                };
                let samples: Vec<f32> = (0..15u8).map(at).collect();
                assert!(
                    samples.iter().any(|v| *v != 0.0),
                    "{rule:?} never fires under {set:?} for the pair ({from}, {to}), so                      sweeping it proves nothing"
                );
                for (apart, pair) in samples.windows(2).enumerate() {
                    assert!(
                        (pair[1] - pair[0]).abs() <= 2.5,
                        "{rule:?} jumps from {} to {} between {apart} and {} semitones                          for the pair ({from}, {to})",
                        pair[0],
                        pair[1],
                        apart + 1
                    );
                }
            }
        }
    }

    #[test]
    fn every_set_carries_the_rules_no_author_had_to_write() {
        for set in PUBLISHED.iter().chain(std::iter::once(&RuleSet::Consensus)) {
            for rule in UNIVERSAL_RULES {
                assert!(
                    set.rules().contains(&rule),
                    "{set:?} is missing {rule:?}, which is not optional"
                );
            }
        }
    }

    #[test]
    fn a_finger_cannot_play_two_different_notes_in_a_row_under_any_set() {
        for set in [
            RuleSet::Parncutt,
            RuleSet::Jacobs,
            RuleSet::Balliauw,
            RuleSet::Badgerow,
            RuleSet::Consensus,
        ] {
            let s = scorer(set);
            let stepped = trigram(Hand::Right, (60, 1), (62, 1), (64, 2));
            assert!(
                s.score_with(&stepped, &[Rule::RepeatedFinger]).total() > 0.0,
                "{set:?} lets one finger take two different notes in a row"
            );
            let repeated = trigram(Hand::Right, (60, 1), (60, 1), (62, 2));
            assert_eq!(
                s.score_with(&repeated, &[Rule::RepeatedFinger]).total(),
                0.0,
                "{set:?} charges a repeated pitch, which is what a finger is for"
            );
        }
    }

    #[test]
    fn four_on_black_charges_the_awkward_arrangement_not_the_ordinary_one() {
        let s = scorer(RuleSet::Parncutt);

        assert_eq!(s.four_on_black(&trigram(Hand::Right, (58, 3), (59, 4), (60, 1))), 1.0);
        assert_eq!(s.four_on_black(&trigram(Hand::Right, (59, 4), (58, 3), (60, 1))), 1.0);

        assert_eq!(
            s.four_on_black(&trigram(Hand::Right, (57, 3), (58, 4), (60, 1))),
            0.0,
            "the taught flat-key arrangement must not be charged"
        );
        assert_eq!(s.four_on_black(&trigram(Hand::Right, (60, 1), (62, 2), (64, 3))), 0.0);
        assert_eq!(s.four_on_black(&trigram(Hand::Right, (58, 3), (61, 4), (60, 1))), 0.0);
    }
}
