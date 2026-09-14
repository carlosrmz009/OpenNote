//! Teaching the fingering model from examples, on the command line.
//!
//! Everything here exists to answer one question: does adding this data make the
//! fingering better? That is why the pieces are the shape they are — a corpus you can
//! add to a file at a time, a model that trains in a second and can say what it
//! learned, and an evaluation that reports before and after so a change that made
//! things worse is obvious rather than silent.
//!
//! * [`corpus`] — reading expert fingerings in, from PIG files or marked-up MusicXML
//! * [`video`] — reading them out of overhead video of somebody playing
//! * [`train`] — fitting the model, and the weights that blend it with the rules
//! * [`eval`] — how well a fingering agrees with the people who wrote one

pub mod corpus;
pub mod eval;
pub mod train;
pub mod video;

pub use corpus::{Corpus, Piece};
pub use eval::{evaluate, recombined, MatchRates};
pub use train::{split, train, Split, TrainReport};
pub use video::{Homography, Watched};
