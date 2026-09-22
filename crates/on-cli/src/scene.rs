//! Command-line arguments that describe the scene, and how to build one.
//!
//! The work of turning a file into a performance lives in `on-viz`, so that the
//! window's own Open command produces exactly what naming a file here does. This is
//! only the argument parsing in front of it.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::Args;
use on_viz::layout::Extent;
use on_viz::session::{self, SessionSettings};
use on_viz::Performance;

use crate::ModelArgs;

/// How the keyboard and the falling-note lane are laid out.
#[derive(Debug, Args, Clone)]
pub struct SceneArgs {
    /// Crop the keyboard to the range the piece actually uses, instead of showing
    /// all eighty-eight keys.
    #[arg(long)]
    pub crop_keyboard: bool,

    /// How far ahead the falling notes reach, in seconds.
    #[arg(long, default_value_t = 3.0)]
    pub lookahead: f64,

    /// The assets directory, if it is not `./assets`. The hand models are expected
    /// at `<assets>/hands/hand-left.glb` and `hand-right.glb`.
    #[arg(long)]
    pub assets: Option<PathBuf>,

    /// A SoundFont (`.sf2`) to play, instead of the modelled piano. Any `.sf2` under
    /// `<assets>/soundfont` is used automatically; this overrides that.
    #[arg(long)]
    pub soundfont: Option<PathBuf>,

    /// What colour the left hand's notes are: a name, or `#rrggbb`.
    ///
    /// Names understood: red, orange, amber, yellow, green, teal, cyan, blue, indigo,
    /// violet, magenta, pink, white.
    #[arg(long, value_name = "COLOUR")]
    pub left_colour: Option<String>,

    /// What colour the right hand's notes are. Same forms as `--left-colour`.
    #[arg(long, value_name = "COLOUR")]
    pub right_colour: Option<String>,
}

/// The colours a note may be asked for by name.
///
/// Bright and saturated, because a note bar is a glowing thing over black and a muted
/// colour disappears into it. They are given in linear terms rather than as the sRGB a
/// paint program would show, since that is what the renderer works in.
const NAMED_COLOURS: [(&str, [f32; 3]); 13] = [
    ("red", [0.85, 0.012, 0.012]),
    ("orange", [0.95, 0.22, 0.01]),
    ("amber", [0.98, 0.50, 0.02]),
    ("yellow", [0.96, 0.83, 0.05]),
    ("green", [0.09, 0.86, 0.16]),
    ("teal", [0.03, 0.80, 0.55]),
    ("cyan", [0.045, 0.88, 1.0]),
    ("blue", [0.10, 0.35, 1.0]),
    ("indigo", [0.30, 0.16, 0.95]),
    ("violet", [0.55, 0.15, 0.95]),
    ("magenta", [0.92, 0.08, 0.85]),
    ("pink", [0.98, 0.32, 0.60]),
    ("white", [0.92, 0.92, 0.95]),
];

/// Read a colour from the command line, falling back to the hand's usual one.
fn parse_colour(asked: Option<&str>, hand: usize) -> Result<on_viz::NoteColour> {
    let Some(asked) = asked else {
        return Ok(on_viz::NoteColours::default().0[hand]);
    };
    let wanted = asked.trim().to_ascii_lowercase();
    if let Some((_, rgb)) = NAMED_COLOURS.iter().find(|(name, _)| *name == wanted) {
        return Ok(on_viz::NoteColour::srgb(rgb[0], rgb[1], rgb[2]));
    }
    let hex = wanted.strip_prefix('#').unwrap_or(&wanted);
    if hex.len() == 6 {
        if let Ok(packed) = u32::from_str_radix(hex, 16) {
            let channel = |shift: u32| ((packed >> shift) & 0xff) as f32 / 255.0;
            return Ok(on_viz::NoteColour::srgb(channel(16), channel(8), channel(0)));
        }
    }
    let names: Vec<&str> = NAMED_COLOURS.iter().map(|(name, _)| *name).collect();
    anyhow::bail!(
        "do not know the colour {asked:?}; give a six-digit `#rrggbb`, or one of: {}",
        names.join(", ")
    )
}

impl SceneArgs {
    /// Turn the command line into session settings.
    pub fn settings(&self, model: &ModelArgs) -> Result<SessionSettings> {
        let note_colours = [
            parse_colour(self.left_colour.as_deref(), 0)?,
            parse_colour(self.right_colour.as_deref(), 1)?,
        ];
        Ok(SessionSettings {
            fingering: model.options()?,
            extent: if self.crop_keyboard {
                Extent::MusicRange
            } else {
                Extent::FullKeyboard
            },
            lookahead: self.lookahead.max(0.5),
            assets_root: session::assets_root(self.assets.clone())?,
            soundfont: self.soundfont.clone(),
            note_colours,
            prior: model.prior()?,
        })
    }
}

/// Read a score, finger it, and build everything the visualizer needs.
pub fn build(input: &Path, settings: &SessionSettings) -> Result<Performance> {
    if !input.exists() {
        bail!("{} does not exist", input.display());
    }
    if !session::hands_available(&settings.assets_root) {
        eprintln!(
            "No rigged hand model found, so this will play without hands.\n\
             The hands are a premade, openly licensed model you supply — see\n\
             assets/hands/README.md for where to get one and how to convert it."
        );
    }
    let performance = session::open(input, settings)?;
    println!(
        "{} — {:.1}s, {} notes.",
        performance
            .timeline
            .title
            .clone()
            .unwrap_or_else(|| "untitled".into()),
        performance.timeline.duration,
        performance.timeline.notes.len()
    );
    Ok(performance)
}
