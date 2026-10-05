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
