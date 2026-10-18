//! `opennote play`: the interactive viewer, or a single screenshot of it.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use crate::scene::{self, SceneArgs};
use crate::ModelArgs;

/// Play a score with falling notes and 3D hands.
#[derive(Debug, Args)]
pub struct PlayArgs {
    /// The score to read. Without one, the viewer opens and asks for it.
    pub input: Option<PathBuf>,

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

/// Opens the viewer with every option at its default, on `score` if there is one. This is
/// what starting `opennote` with no command does, as a double-click does.
pub fn launch(score: Option<PathBuf>) -> Result<()> {
    use clap::FromArgMatches;
    let matches = PlayArgs::augment_args(clap::Command::new("play")).get_matches_from(["play"]);
    let mut args = PlayArgs::from_arg_matches(&matches)?;
    args.input = score;
    run(args)
}

pub fn run(args: PlayArgs) -> Result<()> {
    on_viz::updates::register(crate::update::GitHub);
    let settings = args.scene.settings(&args.model)?;
    let Some(input) = args.input else {
        let performance = on_viz::session::empty(&settings);
        return match args.screenshot {
            Some(path) => on_viz::capture(
                performance,
                on_viz::Capture { at: args.at, path, warmup: args.warmup },
                settings,
                None,
            ),
            None => on_viz::run(performance, settings, None),
        };
    };
    let performance = scene::build(&input, &settings)?;
    match args.screenshot {
        Some(path) => on_viz::capture(
            performance,
            on_viz::Capture { at: args.at, path, warmup: args.warmup },
            settings,
            Some(input),
        ),
        None => {
            println!("space: play/pause   arrows: seek and speed   L: loop   F1: all the shortcuts");
            on_viz::run(performance, settings, Some(input))
        }
    }
}
