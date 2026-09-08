//! `opennote render` — write the performance out as a video file.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use crate::scene::{self, SceneArgs};
use crate::ModelArgs;

/// Render a score to an MP4 with sound.
#[derive(Debug, Args)]
pub struct RenderArgs {
    /// The score to read.
    pub input: PathBuf,

    /// Where to write the video.
    #[arg(long, short, default_value = "opennote.mp4")]
    pub out: PathBuf,

    /// Frame width in pixels.
    #[arg(long, default_value_t = 1920)]
    pub width: u32,

    /// Frame height in pixels.
    #[arg(long, default_value_t = 1080)]
    pub height: u32,

    /// Frames per second.
    #[arg(long, default_value_t = 60)]
    pub fps: u32,

    /// Playback rate, where 1.0 is the score's own tempo. Useful for a slow practice
    /// video of a fast piece.
    #[arg(long, default_value_t = 1.0)]
    pub speed: f64,

    /// Quality, as x264's constant rate factor: 0 is lossless, 18 is very good, 28 is
    /// small. Lower means a bigger file.
    #[arg(long, default_value_t = 18)]
    pub crf: u8,

    /// The ffmpeg executable, if it is not on PATH.
    #[arg(long, default_value = "ffmpeg")]
    pub ffmpeg: PathBuf,

    #[command(flatten)]
    pub scene: SceneArgs,

    #[command(flatten)]
    pub model: ModelArgs,
}

/// Run the command.
pub fn run(args: RenderArgs) -> Result<()> {
    let settings = args.scene.settings(&args.model)?;
    let performance = scene::build(&args.input, &settings)?;
    on_viz::export(
        performance,
        on_viz::VideoOptions {
            path: args.out,
            width: args.width,
            height: args.height,
            fps: args.fps,
            speed: args.speed.max(0.05),
            ffmpeg: args.ffmpeg,
            crf: args.crf,
            note_colours: settings.note_colours,
        },
    )
}
