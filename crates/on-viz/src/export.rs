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

const TAIL_SECONDS: f64 = 2.0;

const WARMUP_FRAMES: u32 = 240;

#[derive(Debug, Clone)]
pub struct VideoOptions {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub speed: f64,
    pub ffmpeg: PathBuf,
    pub crf: u8,
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

#[derive(Resource)]
struct Export {
    options: VideoOptions,
    target: Handle<Image>,
    frame: u32,
    total: u32,
    warmup: u32,
    in_flight: bool,
    frames: Sender<Vec<u8>>,
    trouble: Arc<AtomicBool>,
    start: f64,
}

impl Export {
    fn moment(&self, frame: u32) -> f64 {
        self.start + f64::from(frame) / f64::from(self.options.fps) * self.options.speed
    }
}

pub fn export(performance: Performance, options: VideoOptions) -> Result<()> {
    let ffmpeg = check_ffmpeg(&options.ffmpeg)?;

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
        .init_resource::<Audio>()
        .add_plugins(VisualizerPlugin)
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

    app.world_mut().insert_resource(PendingExport {
        options: options.clone(),
        total,
        frames,
        trouble,
        start: LEAD_IN,
    });
    app.add_systems(First, drive_export);
    app.run();

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

#[derive(Resource, Deref)]
struct ExportTarget(Handle<Image>);

#[derive(Resource)]
struct PendingExport {
    options: VideoOptions,
    total: u32,
    frames: Sender<Vec<u8>>,
    trouble: Arc<AtomicBool>,
    start: f64,
}

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

fn aim_camera_at_target(
    mut commands: Commands,
    target: Res<ExportTarget>,
    cameras: Query<(Entity, &mut bevy::camera::RenderTarget), With<Camera>>,
) {
    for (entity, mut render_target) in cameras {
        *render_target = bevy::camera::RenderTarget::Image((*target).clone().into());
        commands.entity(entity).insert(bevy::ui::IsDefaultUiCamera);
    }
}

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

fn start_ffmpeg(ffmpeg: &Path, options: &VideoOptions, audio: &Path) -> Result<Child> {
    let mut command = Command::new(ffmpeg);
    command
        .arg("-y")
        .args(["-loglevel", "error"])
        .args(["-f", "rawvideo", "-pixel_format", "rgb24"])
        .args(["-video_size", &format!("{}x{}", options.width, options.height)])
        .args(["-framerate", &options.fps.to_string()])
        .args(["-i", "-"])
        .arg("-i")
        .arg(audio)
        .args(["-c:v", "libx264", "-preset", "medium", "-pix_fmt", "yuv420p"])
        .args(["-crf", &options.crf.to_string()])
        .args(["-c:a", "aac", "-b:a", "192k"])
        .arg("-shortest")
        .arg(&options.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null());
    command.spawn().context("starting ffmpeg")
}
