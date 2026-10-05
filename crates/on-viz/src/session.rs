use std::path::{Path, PathBuf};

use std::sync::Arc;

use anyhow::{Context, Result};
use on_fingering::{FingeringOptions, NgramPrior};
use on_score::hands::HandAssignment;

use crate::layout::Extent;
use crate::{Layout, Performance, Timeline};

#[derive(Debug, Clone)]
pub struct SessionSettings {
    pub fingering: FingeringOptions,
    pub extent: Extent,
    pub lookahead: f64,
    pub assets_root: PathBuf,
    pub soundfont: Option<PathBuf>,
    pub note_colours: [bevy::color::Color; 2],
    pub prior: Option<Arc<NgramPrior>>,
    pub mood: on_score::expression::Mood,
    pub expression: f32,
}

impl SessionSettings {
    pub fn new(assets_root: PathBuf) -> Self {
        Self {
            fingering: FingeringOptions::default(),
            extent: Extent::FullKeyboard,
            lookahead: 3.0,
            assets_root,
            soundfont: None,
            note_colours: crate::render::NoteColours::default().0,
            prior: None,
            mood: on_score::expression::Mood::default(),
            expression: 1.0,
        }
    }
}

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
    let performed = on_score::expression::perform(score, &settings.mood, settings.expression);
    let mut timeline =
        Timeline::build_for(&performed, &solution.fingerings, &settings.fingering.profile);
    let scale = |of: f32| 1.0 + (of - 1.0) * settings.expression.min(2.0);
    timeline.style = crate::timeline::MotionStyle {
        lift: scale(settings.mood.lift),
        weight: scale(settings.mood.weight),
        phrase_ends: on_score::expression::phrases(&performed)
            .iter()
            .map(|(_, end)| performed.tempo.seconds_at(*end))
            .collect(),
    };
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

pub fn hands_available(assets: &Path) -> bool {
    ["hand-left.glb", "hand-right.glb"]
        .iter()
        .all(|name| assets.join("hands").join(name).exists())
}

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
