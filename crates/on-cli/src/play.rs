//! `opennote play` — watch the fingering being played.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use crate::scene::{self, SceneArgs};
use crate::ModelArgs;

/// Play a score with falling notes and 3D hands.
#[derive(Debug, Args)]
pub struct PlayArgs {
    /// The score to read.
    pub input: PathBuf,

    /// Instead of playing, save a single frame to this image and exit.
    #[arg(long)]
    pub screenshot: Option<PathBuf>,

    /// The moment to capture, in seconds. Only used with `--screenshot`.
    #[arg(long, default_value_t = 1.0)]
    pub at: f64,

    /// Frames to let pass before capturing, so assets have loaded and the scene has
    /// settled.
    #[arg(long, default_value_t = 200)]
    pub warmup: u32,

    #[command(flatten)]
    pub scene: SceneArgs,

    #[command(flatten)]
    pub model: ModelArgs,
}

/// Run the command.
pub fn run(args: PlayArgs) -> Result<()> {
    let settings = args.scene.settings(&args.model)?;
    let performance = scene::build(&args.input, &settings)?;
    match args.screenshot {
        Some(path) => on_viz::capture(
            performance,
            on_viz::Capture {
                at: args.at,
                path,
                warmup: args.warmup,
            },
            settings,
            Some(args.input),
        ),
        None => {
            println!("space: play/pause   arrows: seek and speed   L: loop");
            on_viz::run(performance, settings, Some(args.input))
        }
    }
}
