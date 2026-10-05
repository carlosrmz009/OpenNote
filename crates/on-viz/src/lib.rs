pub mod layout;
#[cfg(feature = "render")]
pub mod export;
#[cfg(feature = "render")]
pub mod glove;
#[cfg(feature = "render")]
mod hands;
#[cfg(feature = "render")]
pub mod render;
pub mod rig;
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

#[cfg(feature = "render")]
pub use bevy::color::Color as NoteColour;
pub use rig::{BoneRole, RigBone};
#[cfg(feature = "render")]
pub use session::SessionSettings;
#[cfg(feature = "render")]
pub use ui::Session;
pub use timeline::{HandAnimator, KeyStates, Timeline, TimelineNote};
