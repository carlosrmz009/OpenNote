//! Writing the performance out as a video file.
//!
//! The export is the live view, not a second renderer that has to be kept in
//! agreement with it. The same scene, the same systems and the same synthesiser are
//! used; the only difference is what drives them. Live, the sound card sets the pace
//! and the view draws whatever moment has been reached. Exporting, the frame number
//! sets the pace and every frame is drawn at exactly `n / fps` seconds, however long
//! it took to produce. So a slow machine gives the same file as a fast one.
//!
//! Frames leave the GPU through Bevy's screenshot path, aimed at an off-screen image
//! rather than at the window, which is what lets the video be 4K on a laptop with a
//! 1080p screen. They are handed to ffmpeg on its standard input, one at a time: a
//! frame is only asked for once the previous one has come back, so an export cannot
//! run away and fill memory with frames the encoder has not caught up with.
//!
//! ffmpeg is not bundled and is not downloaded. If it is not on the machine the
//! export says so and stops, rather than reaching out to the network on its own.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

use crate::render::{default_plugins, Audio, Performance, Transport, VisualizerPlugin, LEAD_IN};

/// How long to hold on the last chord before the video ends, in seconds.
const TAIL_SECONDS: f64 = 2.0;

/// Frames to let pass before the first one is recorded.
///
/// The hand models load asynchronously and the scene needs a moment to settle. This
/// costs a second or so of wall-clock time and saves an export that opens on a
/// keyboard with no hands over it.
const WARMUP_FRAMES: u32 = 240;

/// What to write, and how.
#[derive(Debug, Clone)]
pub struct VideoOptions {
    /// Where the video goes.
    pub path: PathBuf,
    /// Frame size in pixels.
    pub width: u32,
    pub height: u32,
    /// Frames per second.
    pub fps: u32,
    /// Playback rate, where 1.0 is the score's own tempo.
    pub speed: f64,
    /// The ffmpeg executable to use.
    pub ffmpeg: PathBuf,
    /// Quality, as x264's constant rate factor: lower is better and bigger.
    pub crf: u8,
    /// What colour each hand's notes are, left first.
    pub note_colours: [bevy::color::Color; 2],
}

impl Default for VideoOptions {
    fn default() -> Self {
        Self {
            path: PathBuf::from("opennote.mp4"),
            width: 1920,
            height: 1080,
            fps: 60,
            speed: 1.0,
            ffmpeg: PathBuf::from("ffmpeg"),
            crf: 18,
            note_colours: crate::render::NoteColours::default().0,
        }
    }
}

/// The state the export systems share.
#[derive(Resource)]
struct Export {
    options: VideoOptions,
    /// The off-screen image the camera draws into.
    target: Handle<Image>,
    /// Which frame is being made, counting from zero.
    frame: u32,
    /// How many there are altogether.
    total: u32,
    /// Frames still to elapse before recording starts.
    warmup: u32,
    /// True while a frame is on its way back from the GPU.
    in_flight: bool,
    /// Where finished frames go.
    frames: Sender<Vec<u8>>,
    /// Raised by the writer thread if ffmpeg stops taking frames.
    trouble: Arc<AtomicBool>,
    /// The first moment shown.
    start: f64,
}

impl Export {
    /// The moment frame `n` shows.
    fn moment(&self, frame: u32) -> f64 {
        self.start + f64::from(frame) / f64::from(self.options.fps) * self.options.speed
    }
}

/// Render a performance to a video file.
pub fn export(performance: Performance, options: VideoOptions) -> Result<()> {
    let ffmpeg = check_ffmpeg(&options.ffmpeg)?;

    // The soundtrack is written first, in full, so ffmpeg can mux it as it encodes
    // rather than the whole thing having to be remade afterwards.
    let audio_path = options.path.with_extension("opennote.wav");
    let notes = performance.timeline.audio_notes();
    let last = performance.timeline.duration;
    on_audio::render_to_wav(
        &audio_path,
        notes,
        48_000,
        LEAD_IN,
        options.speed,
        TAIL_SECONDS,
        performance.soundfont.as_deref(),
    )
    .context("rendering the soundtrack")?;

    let seconds = (last - LEAD_IN + TAIL_SECONDS) / options.speed.max(1e-6);
    let total = (seconds * f64::from(options.fps)).ceil() as u32;

    let mut encoder = start_ffmpeg(&ffmpeg, &options, &audio_path)?;
    let stdin = encoder
        .stdin
        .take()
        .context("ffmpeg did not give us anywhere to write frames")?;

    // Frames go to a thread of their own. Writing them from the observer would block
    // the render thread every time the encoder fell behind.
    let (frames, incoming) = mpsc::channel::<Vec<u8>>();
    let trouble = Arc::new(AtomicBool::new(false));
    let report = Arc::clone(&trouble);
    let writer = std::thread::Builder::new()
        .name("opennote-encode".into())
        .spawn(move || {
            let mut stdin = std::io::BufWriter::with_capacity(1 << 20, stdin);
            for frame in incoming {
                if let Err(error) = stdin.write_all(&frame) {
                    tracing::error!("ffmpeg stopped taking frames: {error}");
                    report.store(true, Ordering::Relaxed);
                    return;
                }
            }
            if let Err(error) = stdin.flush() {
                tracing::error!("could not finish writing to ffmpeg: {error}");
                report.store(true, Ordering::Relaxed);
            }
        })
        .context("starting the encoder thread")?;

    println!(
        "Rendering {} frames at {}x{}, {} fps.",
        total, options.width, options.height, options.fps
    );

    let assets = performance.assets_root.clone();
    let width = options.width;
    let height = options.height;
    let mut app = App::new();
    app.add_plugins(default_plugins("OpenNote — exporting", &assets))
        .insert_resource(crate::render::NoteColours(options.note_colours))
        .insert_resource(performance)
        // No sound card during an export. The soundtrack has already been written,
        // and opening a device would only fight the frame clock for control of the
        // transport.
        .init_resource::<Audio>()
        .add_plugins(VisualizerPlugin)
        // Chained, and after the scene is up: each of these needs the one before it
        // to have finished and had its commands applied.
        .add_systems(
            PostStartup,
            (
                move |mut commands: Commands, mut images: ResMut<Assets<Image>>| {
                    let target = images.add(offscreen_target(width, height));
                    commands.insert_resource(ExportTarget(target));
                },
                aim_camera_at_target,
                build_export_resource,
            )
                .chain(),
        );

    // The resource is finished off once the image handle exists, which is why the
    // target is created in a startup system rather than here.
    app.world_mut().insert_resource(PendingExport {
        options: options.clone(),
        total,
        frames,
        trouble,
        start: LEAD_IN,
    });
    app.add_systems(First, drive_export);
    app.run();

    // Dropping the app drops the sender, which ends the writer thread, which closes
    // ffmpeg's input and makes it finish the file.
    drop(app);
    let _ = writer.join();
    let status = encoder.wait().context("waiting for ffmpeg")?;
    let _ = std::fs::remove_file(&audio_path);
    if !status.success() {
        bail!("ffmpeg failed: {status}");
    }
    println!("Wrote {}", options.path.display());
    Ok(())
}

/// The image the camera draws into, before the resource proper exists.
#[derive(Resource, Deref)]
struct ExportTarget(Handle<Image>);

/// The parts of [`Export`] that are known before the app starts.
#[derive(Resource)]
struct PendingExport {
    options: VideoOptions,
    total: u32,
    frames: Sender<Vec<u8>>,
    trouble: Arc<AtomicBool>,
    start: f64,
}

/// Put the two halves together, once the render target exists.
fn build_export_resource(mut commands: Commands) {
    commands.queue(|world: &mut World| {
        let Some(target) = world.remove_resource::<ExportTarget>() else {
            return;
        };
        let Some(pending) = world.remove_resource::<PendingExport>() else {
            return;
        };
        world.insert_resource(Export {
            options: pending.options,
            target: target.0,
            frame: 0,
            total: pending.total,
            warmup: WARMUP_FRAMES,
            in_flight: false,
            frames: pending.frames,
            trouble: pending.trouble,
            start: pending.start,
        });
    });
}

/// An image that can be drawn into and read back.
fn offscreen_target(width: u32, height: u32) -> Image {
    use bevy::image::Image;
    use bevy::render::render_resource::{TextureDimension, TextureFormat, TextureUsages};

    let mut image = Image::new_fill(
        bevy::render::render_resource::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage = TextureUsages::COPY_SRC
        | TextureUsages::COPY_DST
        | TextureUsages::TEXTURE_BINDING
        | TextureUsages::RENDER_ATTACHMENT;
    image
}

/// Point the camera at the off-screen image instead of at the window.
///
/// This is what decouples the size of the video from the size of the screen: the
/// window can be a postage stamp, or behind another window, and the frames are still
/// produced at whatever resolution was asked for.
fn aim_camera_at_target(
    mut commands: Commands,
    target: Res<ExportTarget>,
    cameras: Query<(Entity, &mut bevy::camera::RenderTarget), With<Camera>>,
) {
    for (entity, mut render_target) in cameras {
        *render_target = bevy::camera::RenderTarget::Image((*target).clone().into());
        // The interface picks its camera by looking for one that draws to a window,
        // and this one no longer does. Without saying so explicitly the finger
        // numbers would be laid out and then drawn nowhere, and the export would come
        // back looking right until you noticed every note was unlabelled.
        commands.entity(entity).insert(bevy::ui::IsDefaultUiCamera);
    }
}

/// Advance one frame per screenshot, and stop when the piece is done.
fn drive_export(
    mut commands: Commands,
    export: Option<ResMut<Export>>,
    mut transport: ResMut<Transport>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut export) = export else {
        return;
    };
    if export.trouble.load(Ordering::Relaxed) {
        exit.write(AppExit::error());
        return;
    }
    if export.warmup > 0 {
        export.warmup -= 1;
        // Hold on the first moment while the models load, so the scene that is
        // finally recorded is the one the piece starts on.
        transport.position = export.start;
        transport.playing = false;
        return;
    }
    if export.in_flight {
        return;
    }
    if export.frame >= export.total {
        exit.write(AppExit::Success);
        return;
    }

    // The frame clock, not the wall clock: this is the whole point of the export.
    transport.position = export.moment(export.frame);
    transport.playing = false;
    export.in_flight = true;

    let frames = export.frames.clone();
    let index = export.frame;
    let total = export.total;
    let handle = export.target.clone();
    commands.spawn(Screenshot::image(handle)).observe(
        move |captured: On<ScreenshotCaptured>, mut export: ResMut<Export>| {
            match captured.image.clone().try_into_dynamic() {
                Ok(image) => {
                    if frames.send(image.to_rgb8().into_raw()).is_err() {
                        error!("the encoder went away partway through the export");
                    }
                }
                Err(error) => error!("could not read frame {index} back: {error}"),
            }
            export.frame += 1;
            export.in_flight = false;
            if index % 60 == 0 {
                println!("  frame {index} of {total}");
            }
        },
    );
}

/// Make sure there is an ffmpeg to talk to, and say what to do if there is not.
fn check_ffmpeg(path: &Path) -> Result<PathBuf> {
    let found = Command::new(path)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match found {
        Ok(status) if status.success() => Ok(path.to_path_buf()),
        _ => bail!(
            "could not run ffmpeg at {}.\n\
             The video export hands its frames to ffmpeg, which is not bundled with \
             this program and is not downloaded by it.\n\
             Install it — `winget install ffmpeg` on Windows, `brew install ffmpeg` on \
             macOS, or your package manager on Linux — or point at a copy you already \
             have with --ffmpeg <path>.",
            path.display()
        ),
    }
}

/// Start the encoder, reading raw frames from its standard input.
fn start_ffmpeg(ffmpeg: &Path, options: &VideoOptions, audio: &Path) -> Result<Child> {
    let mut command = Command::new(ffmpeg);
    command
        .arg("-y")
        .args(["-loglevel", "error"])
        // The video comes in raw, so ffmpeg has to be told what it is looking at.
        .args(["-f", "rawvideo", "-pixel_format", "rgb24"])
        .args(["-video_size", &format!("{}x{}", options.width, options.height)])
        .args(["-framerate", &options.fps.to_string()])
        .args(["-i", "-"])
        .arg("-i")
        .arg(audio)
        // yuv420p rather than anything better, because it is what every player and
        // every website can actually decode.
        .args(["-c:v", "libx264", "-preset", "medium", "-pix_fmt", "yuv420p"])
        .args(["-crf", &options.crf.to_string()])
        .args(["-c:a", "aac", "-b:a", "192k"])
        .arg("-shortest")
        .arg(&options.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null());
    command.spawn().context("starting ffmpeg")
}
