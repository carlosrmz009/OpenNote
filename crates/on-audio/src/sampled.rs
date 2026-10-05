use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

const CHANNEL: i32 = 0;

pub struct SampledPiano {
    synthesizer: Synthesizer,
    left: Vec<f32>,
    right: Vec<f32>,
    name: String,
}

impl SampledPiano {
    pub fn load(path: &Path, sample_rate: u32) -> Result<Self> {
        let mut file = std::fs::File::open(path)
            .with_context(|| format!("opening the soundfont at {}", path.display()))?;
        let mut reader = std::io::BufReader::new(&mut file);
        let soundfont = SoundFont::new(&mut reader)
            .map_err(|error| anyhow::anyhow!("{error}"))
            .with_context(|| format!("{} is not a SoundFont", path.display()))?;

        let name = soundfont
            .get_presets()
            .first()
            .map(|preset| preset.get_name().to_string())
            .unwrap_or_else(|| "unnamed".into());

        let mut settings = SynthesizerSettings::new(sample_rate as i32);
        settings.maximum_polyphony = 64;
        settings.enable_reverb_and_chorus = false;

        let synthesizer = Synthesizer::new(&Arc::new(soundfont), &settings)
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("starting the soundfont player")?;

        Ok(Self {
            synthesizer,
            left: Vec::new(),
            right: Vec::new(),
            name,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn strike(&mut self, midi: u8, velocity: u8) {
        self.synthesizer
            .note_on(CHANNEL, i32::from(midi), i32::from(velocity));
    }

    pub fn release(&mut self, midi: u8) {
        self.synthesizer.note_off(CHANNEL, i32::from(midi));
    }

    pub fn silence(&mut self) {
        self.synthesizer.note_off_all(true);
    }

    pub fn render(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        self.left.clear();
        self.left.resize(frames, 0.0);
        self.right.clear();
        self.right.resize(frames, 0.0);
        self.synthesizer.render(&mut self.left, &mut self.right);
        for (frame, out) in out.chunks_exact_mut(2).enumerate() {
            out[0] = self.left[frame];
            out[1] = self.right[frame];
        }
    }
}

pub fn find(assets: &Path) -> Option<std::path::PathBuf> {
    available(assets).into_iter().next()
}

pub fn available(assets: &Path) -> Vec<std::path::PathBuf> {
    let directory = assets.join("soundfont");
    let mut found: Vec<std::path::PathBuf> = match std::fs::read_dir(directory) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("sf2"))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    found.sort();
    found
}
