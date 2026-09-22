//! The sound of the piece.
//!
//! OpenNote ships no sound font and no samples. It synthesises the piano from a
//! description of what a struck string does — see [`voice`] for the model and the
//! sources behind it — which means the audio works the moment the program is built,
//! on any machine, with nothing to download and nothing to license.
//!
//! There is one synthesiser, used two ways. [`Player`] runs it on the sound card for
//! the live window and is the clock the view follows; [`render_to_wav`] runs the same
//! object at whatever rate is asked for and writes the result to disk for the video
//! export. What is exported is therefore exactly what was heard.

pub mod sampled;
pub mod synth;
pub mod voice;
pub mod wav;

#[cfg(feature = "playback")]
pub mod output;

pub use sampled::SampledPiano;
pub use synth::{Note, Synth};
pub use wav::render_to_wav;

#[cfg(feature = "playback")]
pub use output::Player;
