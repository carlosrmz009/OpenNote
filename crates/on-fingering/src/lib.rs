//! Choosing which finger plays which note.
//!
//! The engine combines four things that each know something different:
//!
//! 1. [`rules`] — the published ergonomic rule sets, which encode decades of
//!    observation about what pianists find awkward, in semitones and key colours
//! 2. [`ruler`] — real keyboard geometry, so those rules see millimetres rather
//!    than semitone counts
//! 3. `biomech` — the hand itself, from [`on_hand`]: what a posture costs to hold,
//!    and how much work it is to get from one posture to the next in the time the
//!    music allows
//! 4. a statistical prior trained on expert fingerings, which supplies the part
//!    that no rule captures — what pianists actually do
//!
//! Nothing here is a heuristic search. The cost decomposes over windows of three
//! consecutive notes, so the best fingering for a passage is found exactly, by
//! dynamic programming.

pub mod biomech;
pub mod patterns;
pub mod prior;
pub mod ruler;
pub mod rules;
pub mod scales;
pub mod solver;
pub mod spans;

pub use biomech::{BiomechModel, BiomechWeights, Grip};
pub use prior::NgramPrior;
pub use ruler::Ruler;
pub use rules::{Placement, Rule, RuleCosts, RuleScorer, RuleSet, RuleWeights, Trigram};
pub use solver::{
    finger_score, finger_score_consensus, finger_score_with_prior, Agreement, CostBreakdown,
    FingeringOptions, FingeringPrior,
    NoteExplanation, Solution,
};
pub use spans::{Span, SpanModel, SpanTable};
