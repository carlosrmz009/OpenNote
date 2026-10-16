use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::sampled::SampledPiano;
use crate::synth::{Note, Synth};

enum Command {
    Play(bool),
    Seek(f64),
    Speed(f64),
    Load(Vec<Note>),
    Instrument(u64, Option<SampledPiano>),
}

struct Shared {
    position: AtomicU64,
    silent: AtomicBool,
    instrument: AtomicU64,
}

pub struct Player {
    commands: Sender<Command>,
    shared: Arc<Shared>,
    sample_rate: u32,
}

impl Player {
    pub fn new(notes: Vec<Note>, soundfont: Option<PathBuf>) -> Result<Self> {
        let (commands, orders) = mpsc::channel();
        let shared = Arc::new(Shared {
            position: AtomicU64::new(0.0f64.to_bits()),
            silent: AtomicBool::new(false),
            instrument: AtomicU64::new(0),
        });

        let (ready, opened) = mpsc::channel();
        let thread_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("opennote-audio".into())
            .spawn(move || audio_thread(notes, soundfont, orders, thread_shared, ready))
            .context("starting the audio thread")?;

        let sample_rate = opened
            .recv()
            .context("the audio thread stopped before it opened a device")?
            .map_err(|error| anyhow!("{error}"))?;

        Ok(Self {
            commands,
            shared,
            sample_rate,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn position(&self) -> f64 {
        f64::from_bits(self.shared.position.load(Ordering::Relaxed))
    }

    pub fn silent(&self) -> bool {
        self.shared.silent.load(Ordering::Relaxed)
    }

    pub fn set_playing(&self, playing: bool) {
        let _ = self.commands.send(Command::Play(playing));
    }

    pub fn seek(&self, seconds: f64) {
        let _ = self.commands.send(Command::Seek(seconds));
    }

    pub fn set_speed(&self, speed: f64) {
        let _ = self.commands.send(Command::Speed(speed));
    }

    pub fn load(&self, notes: Vec<Note>) {
        let _ = self.commands.send(Command::Load(notes));
    }

    /// Reading a SoundFont takes far longer than one audio buffer, so it is read on a
    /// thread of its own and handed over when ready. If another is asked for meanwhile,
    /// whichever was asked for last is the one that plays.
    pub fn set_instrument(&self, soundfont: Option<PathBuf>) {
        let generation = self.shared.instrument.fetch_add(1, Ordering::Relaxed) + 1;
        let commands = self.commands.clone();
        let sample_rate = self.sample_rate;
        let loading = std::thread::Builder::new().name("opennote-soundfont".into()).spawn(move || {
            let piano = open_soundfont(soundfont.as_deref(), sample_rate);
            let _ = commands.send(Command::Instrument(generation, piano));
        });
        if let Err(error) = loading {
            tracing::warn!("could not start reading the SoundFont: {error}");
        }
    }
}

fn open_soundfont(path: Option<&std::path::Path>, sample_rate: u32) -> Option<SampledPiano> {
    let path = path?;
    match SampledPiano::load(path, sample_rate) {
        Ok(piano) => {
            tracing::info!("playing {}", piano.name());
            Some(piano)
        }
        Err(error) => {
            tracing::warn!("{error:#}; using the modelled piano instead");
            None
        }
    }
}

fn audio_thread(
    notes: Vec<Note>,
    soundfont: Option<PathBuf>,
    orders: Receiver<Command>,
    shared: Arc<Shared>,
    ready: Sender<Result<u32, String>>,
) {
    let stream = match open_stream(notes, soundfont, orders, shared) {
        Ok((stream, rate)) => {
            let _ = ready.send(Ok(rate));
            stream
        }
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };
    if let Err(error) = stream.play() {
        tracing::error!("could not start the audio stream: {error}");
        return;
    }
    loop {
        std::thread::park();
    }
}

fn open_stream(
    notes: Vec<Note>,
    soundfont: Option<PathBuf>,
    orders: Receiver<Command>,
    shared: Arc<Shared>,
) -> Result<(cpal::Stream, u32)> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow!("no audio output device"))?;
    let config = device
        .default_output_config()
        .context("asking the audio device what it supports")?;
    let sample_rate = config.sample_rate();
    let channels = config.channels() as usize;
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();

    let mut synth = Synth::new(notes, sample_rate as f32);
    if let Some(piano) = open_soundfont(soundfont.as_deref(), sample_rate) {
        synth = synth.with_soundfont(piano);
    }
    let mut stereo: Vec<f32> = Vec::new();
    let instrument = Arc::clone(&shared);

    let mut fill = move |frames: usize, mut write: Box<dyn FnMut(usize, usize, f32) + '_>| {
        while let Ok(order) = orders.try_recv() {
            match order {
                Command::Play(playing) => synth.set_playing(playing),
                Command::Seek(seconds) => synth.seek(seconds),
                Command::Speed(speed) => synth.set_speed(speed),
                Command::Load(notes) => {
                    synth.load(notes);
                }
                Command::Instrument(generation, piano) => {
                    if generation == instrument.instrument.load(Ordering::Relaxed) {
                        synth.set_soundfont(piano);
                    }
                }
            }
        }
        stereo.clear();
        stereo.resize(frames * 2, 0.0);
        synth.render(&mut stereo);
        shared
            .position
            .store(synth.position().to_bits(), Ordering::Relaxed);
        shared.silent.store(synth.silent(), Ordering::Relaxed);

        for frame in 0..frames {
            for channel in 0..channels {
                let sample = if channels == 1 {
                    (stereo[frame * 2] + stereo[frame * 2 + 1]) * 0.5
                } else {
                    stereo[frame * 2 + channel % 2]
                };
                write(frame, channel, sample);
            }
        }
    };

    let on_error = |error| tracing::error!("audio stream error: {error}");
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _| {
                let frames = data.len() / channels;
                fill(
                    frames,
                    Box::new(|frame, channel, sample| data[frame * channels + channel] = sample),
                );
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            &config,
            move |data: &mut [i16], _| {
                let frames = data.len() / channels;
                fill(
                    frames,
                    Box::new(|frame, channel, sample| {
                        data[frame * channels + channel] = to_i16(sample)
                    }),
                );
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_output_stream(
            &config,
            move |data: &mut [u16], _| {
                let frames = data.len() / channels;
                fill(
                    frames,
                    Box::new(|frame, channel, sample| {
                        data[frame * channels + channel] = to_u16(sample)
                    }),
                );
            },
            on_error,
            None,
        ),
        other => return Err(anyhow!("the audio device wants {other:?} samples, which this does not write")),
    }
    .context("opening the audio output stream")?;

    Ok((stream, sample_rate))
}

fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * 32_767.0) as i16
}

fn to_u16(sample: f32) -> u16 {
    (i32::from(to_i16(sample)) + 32_768) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sample_past_full_scale_is_clipped_rather_than_wrapped() {
        assert_eq!(to_u16(0.0), 32_768);
        assert_eq!(to_u16(1.0), 65_535);
        assert_eq!(to_u16(1.5), 65_535);
        assert_eq!(to_u16(-1.5), 1);
        assert_eq!(to_i16(1.5), 32_767);
        assert_eq!(to_i16(-1.5), -32_767);
    }
}
