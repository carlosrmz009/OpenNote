//! Getting from a file on disk to something on screen.
//!
//! Reading a score, assigning hands, solving the fingering and laying the keyboard
//! out is one piece of work, and three things want it: `opennote play`,
//! `opennote render`, and the Open command in the window's own menu. It lives here
//! so that opening a file from the menu gives exactly the same result as naming it on
//! the command line.

use std::path::{Path, PathBuf};

use std::sync::Arc;

use anyhow::{Context, Result};
use on_fingering::{FingeringOptions, NgramPrior};
use on_score::hands::HandAssignment;

use crate::layout::Extent;
use crate::{Layout, Performance, Timeline};

/// Everything about a performance that is not the score itself.
#[derive(Debug, Clone)]
pub struct SessionSettings {
    /// How the fingering is worked out: hand size, rule set, and the rest.
    pub fingering: FingeringOptions,
    /// Whether to show all eighty-eight keys or only the ones the piece uses.
    pub extent: Extent,
    /// How far ahead the falling notes reach, in seconds.
    pub lookahead: f64,
    /// Absolute path to the assets directory.
    pub assets_root: PathBuf,
    /// A recorded piano to play instead of the modelled one. `None` looks for any
    /// `.sf2` under `<assets>/soundfont`.
    pub soundfont: Option<PathBuf>,
    /// What colour each hand's notes are, left first.
    pub note_colours: [bevy::color::Color; 2],
    /// A trained statistical model, if one is in use.
    ///
    /// Behind an `Arc` because the settings are cloned every time the window opens a
    /// different score, and the model is the one part of them worth sharing rather
    /// than copying.
    pub prior: Option<Arc<NgramPrior>>,
}

impl SessionSettings {
    /// Sensible settings for a medium hand, a full keyboard, and `./assets`.
    pub fn new(assets_root: PathBuf) -> Self {
        Self {
            fingering: FingeringOptions::default(),
            extent: Extent::FullKeyboard,
            lookahead: 3.0,
            assets_root,
            soundfont: None,
            note_colours: crate::render::NoteColours::default().0,
            prior: None,
        }
    }
}

/// Read a score and work out everything needed to show it.
pub fn open(path: &Path, settings: &SessionSettings) -> Result<Performance> {
    let mut document =
        on_score::Document::open(path).with_context(|| format!("reading {}", path.display()))?;

    let assignment = HandAssignment {
        profile: settings.fingering.profile.clone(),
        ..Default::default()
    };
    on_score::assign_hands(document.score_mut(), &assignment);

    let score = document.score();
    let prior = settings.prior.as_deref().map(|p| p as &dyn on_fingering::FingeringPrior);
    let solution = on_fingering::finger_score_with_prior(score, &settings.fingering, prior);
    let mut timeline =
        Timeline::build_for(score, &solution.fingerings, &settings.fingering.profile);
    if timeline.title.is_none() {
        timeline.title = path
            .file_stem()
            .and_then(|name| name.to_str())
            .map(str::to_owned);
    }

    let mut layout = Layout::for_pitches(score.notes.iter().map(|n| n.midi), settings.extent);
    layout.lookahead = settings.lookahead.max(0.5);

    let assets = settings.assets_root.clone();
    let hands = hands_available(&assets);
    let soundfont = settings
        .soundfont
        .clone()
        .or_else(|| on_audio::sampled::find(&assets));
    Ok(Performance::new(
        timeline,
        layout,
        settings.fingering.profile.clone(),
        assets,
        hands,
        soundfont,
    ))
}

/// Whether both hand models are present under an assets directory.
pub fn hands_available(assets: &Path) -> bool {
    ["hand-left.glb", "hand-right.glb"]
        .iter()
        .all(|name| assets.join("hands").join(name).exists())
}

/// The assets directory, as an absolute path.
///
/// Absolute because Bevy resolves its asset root against the crate being run rather
/// than the working directory, so a relative path would break the moment the binary
/// is launched with `cargo run -p`.
///
/// Given nothing to go on, the working directory is tried first and then the places
/// the assets sit relative to the binary itself — beside it, and one or two levels up,
/// which is where `target/release/opennote` finds them. Without that, running the
/// binary from anywhere but the repository root fails on a missing hand model, which
/// is the first thing anyone does with it.
pub fn assets_root(candidate: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(named) = candidate {
        return std::fs::canonicalize(&named)
            .with_context(|| format!("looking for the assets directory at {}", named.display()));
    }

    let mut tried = vec![PathBuf::from("assets")];
    if let Ok(exe) = std::env::current_exe() {
        tried.extend(
            exe.ancestors()
                .skip(1)
                .take(4)
                .map(|directory| directory.join("assets")),
        );
    }
    for path in &tried {
        if let Ok(found) = std::fs::canonicalize(path) {
            if found.is_dir() {
                return Ok(found);
            }
        }
    }
    anyhow::bail!(
        "could not find an assets directory. Looked in:\n{}\n\
         Run from the repository root, or point at it with --assets.",
        tried
            .iter()
            .map(|p| format!("  {}", p.display()))
            .collect::<Vec<_>>()
            .join("\n")
    )
}
