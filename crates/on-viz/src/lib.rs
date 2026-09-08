//! Watching the fingering: a flat Synthesia keyboard with 3D hands over it.
//!
//! The visualizer is deliberately two different things in one view. The keyboard and
//! the falling notes are flat, because that is what makes a Synthesia display
//! readable — a note is a rectangle over the key it belongs to, and perspective only
//! gets in the way. The hands are the exception: those are real 3D, articulated, and
//! posed by the same biomechanical model that chose the fingering.
//!
//! Everything is seen through one orthographic camera looking straight down, and the
//! world is measured in millimetres. So the hands and the keys are to scale with each
//! other by construction rather than by tuning: a hand that spans an octave on a real
//! piano spans an octave here.
//!
//! * [`timeline`] — what is happening at time *t*: key states, hand postures
//! * [`layout`] — where things sit on screen, in millimetres and in pixels

pub mod layout;
#[cfg(feature = "render")]
pub mod export;
#[cfg(feature = "render")]
pub mod glove;
mod hands;
#[cfg(feature = "render")]
pub mod render;
pub mod rig;
// Building a performance needs the renderer's types, so this half of the crate is
// not available in the no-GPU build.
#[cfg(feature = "render")]
pub mod session;
#[cfg(feature = "render")]
pub mod skin;
pub mod timeline;
#[cfg(feature = "render")]
pub mod ui;

pub use layout::Layout;
#[cfg(feature = "render")]
pub use export::{export, VideoOptions};
#[cfg(feature = "render")]
pub use render::{capture, run, Audio, Capture, NoteColours, Performance, Transport, LEAD_IN};

/// The colour type a note is given, re-exported so callers need not depend on Bevy
/// only to name a colour.
pub use bevy::color::Color as NoteColour;
pub use rig::{BoneRole, RigBone};
#[cfg(feature = "render")]
pub use session::SessionSettings;
#[cfg(feature = "render")]
pub use ui::Session;
pub use timeline::{HandAnimator, KeyStates, Timeline, TimelineNote};
