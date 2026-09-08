//! Writing a rendered performance out as a WAV file.
//!
//! The video export needs a soundtrack on disk to hand to ffmpeg. WAV is written
//! directly rather than through a crate because the format is a forty-four byte
//! header followed by the samples, and a dependency for that is not worth its weight.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use crate::sampled::SampledPiano;
use crate::synth::{Note, Synth};

/// Render a piece to a 16-bit stereo WAV file, and report how long it lasted.
///
/// Recording begins at `start`, which is normally negative: the view opens on an
/// empty keyboard for a moment before the first note arrives, and the soundtrack has
/// to carry that silence or the two will not line up. It carries on past the last
/// note for `tail_seconds`, so a piece does not end with its final chord cut off.
pub fn render_to_wav(
    path: &Path,
    notes: Vec<Note>,
    sample_rate: u32,
    start: f64,
    speed: f64,
    tail_seconds: f64,
    soundfont: Option<&Path>,
) -> Result<f64> {
    let last = notes.iter().fold(0.0f64, |a, n| a.max(n.end));
    let mut synth = Synth::new(notes, sample_rate as f32);
    if let Some(path) = soundfont {
        synth = synth.with_soundfont(SampledPiano::load(path, sample_rate)?);
    }
    synth.seek(start);
    synth.set_speed(speed);
    synth.set_playing(true);

    let seconds = (last - start + tail_seconds).max(0.0) / speed.max(1e-6);
    let total_frames = (seconds * f64::from(sample_rate)) as usize;
    let mut samples = Vec::with_capacity(total_frames * 2);
    let mut block = vec![0.0f32; 2 * 1_024];
    while samples.len() < total_frames * 2 {
        synth.render(&mut block);
        samples.extend_from_slice(&block);
    }
    samples.truncate(total_frames * 2);
    normalise(&mut samples);

    let mut file = std::io::BufWriter::new(
        std::fs::File::create(path)
            .with_context(|| format!("creating {}", path.display()))?,
    );
    write_header(&mut file, sample_rate, samples.len())?;
    for sample in &samples {
        let value = (sample.clamp(-1.0, 1.0) * 32_767.0) as i16;
        file.write_all(&value.to_le_bytes())?;
    }
    file.flush().context("writing the audio")?;
    Ok(total_frames as f64 / f64::from(sample_rate))
}

/// Headroom left below full scale, in decibels.
///
/// Not zero. A peak sitting exactly at full scale clips once the file is encoded to
/// AAC, because a lossy codec does not reproduce the waveform exactly and its
/// reconstruction overshoots.
const HEADROOM_DB: f32 = 1.0;

/// Bring the loudest moment of the piece up to just under full scale.
///
/// One gain over the whole file, worked out from its own peak — not compression, and
/// not per-block, so the dynamics are exactly as played and only the level changes. A
/// SoundFont is usually mastered conservatively and the modelled piano is quieter
/// still, which leaves an exported video ten to fifteen decibels below anything else
/// somebody will watch it next to.
fn normalise(samples: &mut [f32]) {
    let peak = samples.iter().fold(0.0f32, |loudest, s| loudest.max(s.abs()));
    // Silence, or already there: nothing worth scaling.
    if peak < 1e-4 {
        return;
    }
    let target = 10.0f32.powf(-HEADROOM_DB / 20.0);
    let gain = target / peak;
    for sample in samples {
        *sample *= gain;
    }
}

/// The RIFF header for 16-bit stereo PCM.
fn write_header(out: &mut impl Write, sample_rate: u32, samples: usize) -> Result<()> {
    let channels = 2u16;
    let bits = 16u16;
    let data_bytes = (samples * 2) as u32;
    let byte_rate = sample_rate * u32::from(channels) * u32::from(bits / 8);

    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data_bytes).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?; // size of this chunk
    out.write_all(&1u16.to_le_bytes())?; // 1 = uncompressed PCM
    out.write_all(&channels.to_le_bytes())?;
    out.write_all(&sample_rate.to_le_bytes())?;
    out.write_all(&byte_rate.to_le_bytes())?;
    out.write_all(&(channels * bits / 8).to_le_bytes())?; // bytes per frame
    out.write_all(&bits.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&data_bytes.to_le_bytes())?;
    Ok(())
}

/// Convenience for callers that only have the notes.
pub fn duration_of(notes: &[Note]) -> f64 {
    notes.iter().fold(0.0f64, |a, n| a.max(n.end))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exported file has to be loud enough to sit next to anything else.
    #[test]
    fn the_export_is_brought_up_to_just_under_full_scale() {
        let mut quiet: Vec<f32> = (0..2048)
            .map(|i| 0.02 * (i as f32 * 0.05).sin())
            .collect();
        normalise(&mut quiet);
        let peak = quiet.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        let target = 10.0f32.powf(-HEADROOM_DB / 20.0);
        assert!(
            (peak - target).abs() < 1e-3,
            "peaked at {peak}, wanted {target}"
        );
        assert!(peak < 1.0, "no headroom left for the encoder");
    }

    /// Silence stays silent rather than being amplified into noise.
    #[test]
    fn silence_is_left_alone() {
        let mut nothing = vec![0.0f32; 512];
        normalise(&mut nothing);
        assert!(nothing.iter().all(|s| *s == 0.0));
    }

    /// The gain is one number over the whole file, so the dynamics are untouched.
    #[test]
    fn normalising_does_not_change_the_shape_of_the_music() {
        let original: Vec<f32> = (0..512)
            .map(|i| 0.3 * (i as f32 * 0.1).sin() * (1.0 - i as f32 / 512.0))
            .collect();
        let mut scaled = original.clone();
        normalise(&mut scaled);
        // Every ratio to the original is the same gain.
        let gain = scaled[7] / original[7];
        for (before, after) in original.iter().zip(&scaled) {
            assert!(
                (after - before * gain).abs() < 1e-5,
                "the waveform changed shape"
            );
        }
    }

    #[test]
    fn a_written_file_has_a_header_and_the_right_amount_of_audio() {
        let dir = std::env::temp_dir().join("opennote-wav-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scale.wav");
        let notes = vec![
            Note { midi: 60, start: 0.0, end: 0.4, velocity: 90 },
            Note { midi: 64, start: 0.4, end: 0.8, velocity: 90 },
        ];
        let seconds = render_to_wav(&path, notes, 22_050, 0.0, 1.0, 0.5, None).unwrap();
        assert!((seconds - 1.3).abs() < 0.01, "wrote {seconds}s");

        let written = std::fs::read(&path).unwrap();
        assert_eq!(&written[..4], b"RIFF");
        assert_eq!(&written[8..12], b"WAVE");
        let frames = (written.len() - 44) / 4;
        assert_eq!(frames, (22_050.0 * 1.3) as usize);
        // Something has to have been played into it.
        assert!(written[44..].iter().any(|b| *b != 0));
        std::fs::remove_file(&path).ok();
    }
}
