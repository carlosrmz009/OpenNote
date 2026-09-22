//! Getting the synthesiser out of the speakers.
//!
//! The audio device runs on its own clock, which is not the clock the window draws
//! on. Rather than have the two guess at each other, the synthesiser is the one that
//! counts: it advances by exactly one sample period per sample it produces and
//! publishes where it has got to, and the view reads that and draws the frame that
//! belongs to the sound coming out at that moment. Notes therefore land on the hit
//! line when they are heard rather than a frame or two either side of it, and they
//! stay that way over a long piece — which they would not if the view counted
//! wall-clock time and the audio counted samples.
//!
//! The device is driven from a thread of its own. A `cpal` stream is not `Send` on
//! every platform, so it cannot be held in a resource that moves between threads;
//! instead the thread owns the stream for as long as the program runs, and the rest
//! of the program talks to it through a channel and an atomic.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::sampled::SampledPiano;
use crate::synth::{Note, Synth};

/// Something for the audio thread to do.
enum Command {
    Play(bool),
    Seek(f64),
    Speed(f64),
    /// A different piece, on the same instrument.
    Load(Vec<Note>),
    /// A different instrument, for the same piece. `None` is the modelled piano.
    Instrument(Option<PathBuf>),
}

/// What the audio thread publishes back.
struct Shared {
    /// The playhead, as the bits of an `f64`.
    position: AtomicU64,
    /// Whether the piece has finished and stopped ringing.
    silent: AtomicBool,
}

/// A piano playing out of the speakers.
pub struct Player {
    commands: Sender<Command>,
    shared: Arc<Shared>,
    sample_rate: u32,
}

impl Player {
    /// Open the default output device and start a silent, paused piano on it.
    ///
    /// `soundfont` is the recorded instrument to play, if there is one. Without it the
    /// modelled piano plays, which needs nothing downloaded.
    pub fn new(notes: Vec<Note>, soundfont: Option<PathBuf>) -> Result<Self> {
        let (commands, orders) = mpsc::channel();
        let shared = Arc::new(Shared {
            position: AtomicU64::new(0.0f64.to_bits()),
            silent: AtomicBool::new(false),
        });

        // The thread reports whether it managed to open a device, so that a missing
        // or busy sound card is an error here rather than silence later.
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

    /// The rate the device is running at.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Where the sound coming out of the speakers has got to, in seconds.
    pub fn position(&self) -> f64 {
        f64::from_bits(self.shared.position.load(Ordering::Relaxed))
    }

    /// Whether the piece has finished and everything has stopped ringing.
    pub fn silent(&self) -> bool {
        self.shared.silent.load(Ordering::Relaxed)
    }

    /// Start or stop the playhead.
    pub fn set_playing(&self, playing: bool) {
        let _ = self.commands.send(Command::Play(playing));
    }

    /// Jump the playhead.
    pub fn seek(&self, seconds: f64) {
        let _ = self.commands.send(Command::Seek(seconds));
    }

    /// Play faster or slower.
    pub fn set_speed(&self, speed: f64) {
        let _ = self.commands.send(Command::Speed(speed));
    }

    /// Put a different piece on the same instrument.
    ///
    /// The device stays open. Closing it and opening another would give an audible
    /// gap, and on some machines a device that has just been released is not
    /// immediately available again.
    pub fn load(&self, notes: Vec<Note>) {
        let _ = self.commands.send(Command::Load(notes));
    }

    /// Play on a different recorded instrument, or on the modelled one.
    ///
    /// The file is opened on the audio thread, which can take a moment for a large
    /// SoundFont — the good ones run to a gigabyte. The sound carries on meanwhile, on
    /// whatever it was playing on before.
    pub fn set_instrument(&self, soundfont: Option<PathBuf>) {
        let _ = self.commands.send(Command::Instrument(soundfont));
    }
}

/// Open a recorded instrument, or report why not and carry on without one.
///
/// A SoundFont that will not load is worth saying so about but not worth refusing to
/// play over: the modelled piano is right there.
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

/// Own the device for the life of the program.
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
    // The stream stops the moment it is dropped, so this thread holds it and sleeps.
    loop {
        std::thread::park();
    }
}

/// Build the output stream around a synthesiser.
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
    // Scratch space for the stereo mix, before it is spread over however many
    // channels the device actually has.
    let mut stereo: Vec<f32> = Vec::new();

    let mut fill = move |frames: usize, mut write: Box<dyn FnMut(usize, usize, f32) + '_>| {
        while let Ok(order) = orders.try_recv() {
            match order {
                Command::Play(playing) => synth.set_playing(playing),
                Command::Seek(seconds) => synth.seek(seconds),
                Command::Speed(speed) => synth.set_speed(speed),
                Command::Load(notes) => {
                    // In place, so the recorded instrument this synth was opened with
                    // survives. Replacing the synth wholesale dropped it, and the piece
                    // came back on the modelled piano.
                    synth.load(notes);
                }
                Command::Instrument(path) => {
                    synth.set_soundfont(open_soundfont(path.as_deref(), sample_rate));
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
                // Mono devices get the two sides summed; anything wider than stereo
                // gets the pair repeated, which is right for the common case of a
                // surround device with nothing plugged into the back.
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
                        data[frame * channels + channel] = (sample * 32_767.0) as i16
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
                        data[frame * channels + channel] =
                            ((sample * 32_767.0) as i32 + 32_768) as u16
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
