//! Playing a real piano, from a SoundFont somebody recorded.
//!
//! [`crate::voice`] models a struck string, which works everywhere and needs nothing
//! downloaded. What it cannot do is sound like a *particular* instrument: a real grand
//! has a soundboard, sympathetic strings, key noise, una corda, and a spectrum that no
//! handful of partials reproduces. For that you need somebody's recording of one.
//!
//! So a SoundFont, if there is one, wins. The reading and voicing of it is
//! `rustysynth`'s job — an SF2 player is thousands of lines of envelopes, loop points,
//! filters and modulators, and there is a good one already.
//!
//! Nothing is bundled and nothing is downloaded. See `assets/soundfont/README.md` for
//! where to get one and what the licences ask of you.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

/// The MIDI channel everything is played on. One instrument, one channel.
const CHANNEL: i32 = 0;

/// A sampled piano, loaded from a SoundFont file.
pub struct SampledPiano {
    synthesizer: Synthesizer,
    /// Scratch buffers, because SoundFont players render the two sides separately and
    /// everything else here is interleaved.
    left: Vec<f32>,
    right: Vec<f32>,
    /// What the file calls itself, for the line the program prints on startup.
    name: String,
}

impl SampledPiano {
    /// Load a SoundFont.
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
        // A pianist has ten fingers and a sustain pedal; the default ceiling is for
        // whole orchestras and costs more than it is worth here.
        settings.maximum_polyphony = 64;
        // The reverb is the SoundFont author's, not the room's, and a falling-note
        // display wants the note to stop when the key comes up.
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

    /// What the soundfont calls its first preset.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Strike a key.
    pub fn strike(&mut self, midi: u8, velocity: u8) {
        self.synthesizer
            .note_on(CHANNEL, i32::from(midi), i32::from(velocity));
    }

    /// Let a key up.
    pub fn release(&mut self, midi: u8) {
        self.synthesizer.note_off(CHANNEL, i32::from(midi));
    }

    /// Stop everything at once, for a seek.
    pub fn silence(&mut self) {
        self.synthesizer.note_off_all(true);
    }

    /// Render a block of interleaved stereo into `out`, which must be an even length.
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

/// Look for a soundfont to use, if the caller did not name one.
///
/// Any `.sf2` under `<assets>/soundfont`. Alphabetical, so which one is picked is
/// predictable when there are several.
pub fn find(assets: &Path) -> Option<std::path::PathBuf> {
    available(assets).into_iter().next()
}

/// Every SoundFont sitting under `<assets>/soundfont`, in a stable order.
///
/// Whatever is dropped in there is offered; nothing is named in the code. A SoundFont
/// is a large file somebody has chosen to download, and having to edit the program to
/// use one would be a strange arrangement.
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
