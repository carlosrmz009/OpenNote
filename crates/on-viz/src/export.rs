//! Renders the performance offscreen, frame by frame, and pipes it to ffmpeg with the
//! soundtrack.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

use crate::render::{default_plugins, Audio, Performance, Transport, VisualizerPlugin, LEAD_IN};

const TAIL_SECONDS: f64 = 2.0;

/// How long the "Made with OpenNote" card is shown, fades included.
pub const INTRO_SECONDS: f64 = 2.6;

/// The card's fade in and fade out.
const INTRO_FADE_SECONDS: f64 = 0.55;

/// The colour the stage starts from, so the card fades into the piece without a flash.
const INTRO_BACKGROUND: [u8; 3] = [0x0D, 0x0E, 0x12];

const INTRO_LOCKUP: &[u8] = include_bytes!("../../../assets/branding/made-with-opennote.png");

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
    /// Starts with a "Made with OpenNote" card.
    pub intro: bool,
    /// Only the part of the piece from here, in seconds. The whole piece when empty.
    pub from: Option<f64>,
    /// Only the part of the piece up to here, in seconds.
    pub to: Option<f64>,
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
            intro: true,
            from: None,
            to: None,
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
    frames: SyncSender<Vec<u8>>,
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
    let (from, to) = video_range(&options, last)?;
    on_audio::render_to_wav(
        &audio_path,
        notes,
        48_000,
        from,
        options.speed,
        TAIL_SECONDS,
        performance.soundfont.as_deref(),
    )
    .context("rendering the soundtrack")?;
    let _soundtrack = Scratch(audio_path.clone());

    let seconds = (to - from) / options.speed.max(1e-6);
    let total = (seconds * f64::from(options.fps)).ceil() as u32;

    let mut encoder = start_ffmpeg(&ffmpeg, &options, &audio_path)?;
    let stdin = encoder.stdin.take().context("ffmpeg did not give us anywhere to write frames")?;

    // Bounded, so that rendering waits for the encoder rather than holding every frame
    // it has not yet taken in memory.
    let (frames, incoming) = mpsc::sync_channel::<Vec<u8>>(FRAMES_IN_FLIGHT);
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

    if options.intro {
        let card = IntroCard::new(options.width, options.height);
        for frame in card.frames(options.fps) {
            if frames.send(frame).is_err() {
                bail!("the encoder stopped before the video began");
            }
        }
    }
    println!("Rendering {} frames at {}x{}, {} fps.", total, options.width, options.height, options.fps);

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
        start: from,
    });
    app.add_systems(First, drive_export);
    app.run();

    drop(app);
    let _ = writer.join();
    let status = encoder.wait().context("waiting for ffmpeg")?;
    if !status.success() {
        bail!("ffmpeg failed: {status}");
    }
    println!("Wrote {}", options.path.display());
    Ok(())
}

#[derive(Resource, Deref)]
struct ExportTarget(Handle<Image>);

const FRAMES_IN_FLIGHT: usize = 8;

/// A temporary file, removed however the export ends.
struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[derive(Resource)]
struct PendingExport {
    options: VideoOptions,
    total: u32,
    frames: SyncSender<Vec<u8>>,
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
        bevy::render::render_resource::Extent3d { width, height, depth_or_array_layers: 1 },
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
            if index % 10 == 0 {
                println!("  frame {index} of {total}");
            }
        },
    );
}

fn check_ffmpeg(path: &Path) -> Result<PathBuf> {
    let found = Command::new(path).arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status();
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
        .args(intro_delay_filter(options))
        .arg("-shortest")
        .arg(&options.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null());
    command.spawn().context("starting ffmpeg")
}

/// The part of the piece to film, in seconds: all of it unless asked for less.
fn video_range(options: &VideoOptions, last: f64) -> Result<(f64, f64)> {
    let from = options.from.unwrap_or(LEAD_IN).max(LEAD_IN);
    let to = options.to.unwrap_or(last + TAIL_SECONDS);
    if to <= from + 0.05 {
        bail!("the video would be empty: it would run from {from:.2}s to {to:.2}s");
    }
    Ok((from, to))
}

/// Holds the soundtrack back until the intro card is over.
fn intro_delay_filter(options: &VideoOptions) -> Vec<String> {
    if !options.intro {
        return Vec::new();
    }
    let milliseconds = (INTRO_SECONDS * 1000.0).round() as u64;
    vec!["-af".into(), format!("adelay={milliseconds}:all=1")]
}

/// The "Made with OpenNote" card, drawn for one frame size.
struct IntroCard {
    width: u32,
    height: u32,
    /// The picture, scaled, with its alpha.
    lockup: image::RgbaImage,
    left: u32,
    top: u32,
}

impl IntroCard {
    fn new(width: u32, height: u32) -> Self {
        let picture = image::load_from_memory(INTRO_LOCKUP).expect("the intro picture is a PNG").to_rgba8();
        // As big as is comfortable: up to 55% of the height and 70% of the width.
        let scale = (f64::from(height) * 0.55 / f64::from(picture.height()))
            .min(f64::from(width) * 0.70 / f64::from(picture.width()));
        let (w, h) = (
            ((f64::from(picture.width()) * scale).round() as u32).max(1),
            ((f64::from(picture.height()) * scale).round() as u32).max(1),
        );
        let lockup = image::imageops::resize(&picture, w, h, image::imageops::FilterType::Lanczos3);
        Self { width, height, left: (width - w.min(width)) / 2, top: (height - h.min(height)) / 2, lockup }
    }

    /// The card with the picture `visible` strong, from 0 (only the background) to 1.
    fn frame(&self, visible: f64) -> Vec<u8> {
        let mut frame = Vec::with_capacity(self.width as usize * self.height as usize * 3);
        for _ in 0..(self.width as usize * self.height as usize) {
            frame.extend_from_slice(&INTRO_BACKGROUND);
        }
        let visible = visible.clamp(0.0, 1.0);
        for (x, y, pixel) in self.lockup.enumerate_pixels() {
            let alpha = f64::from(pixel[3]) / 255.0 * visible;
            if alpha <= 0.0 {
                continue;
            }
            let (px, py) = (self.left + x, self.top + y);
            if px >= self.width || py >= self.height {
                continue;
            }
            let at = (py as usize * self.width as usize + px as usize) * 3;
            for channel in 0..3 {
                let behind = f64::from(frame[at + channel]);
                frame[at + channel] = (behind + (f64::from(pixel[channel]) - behind) * alpha).round() as u8;
            }
        }
        frame
    }

    /// Every frame of the intro: the card fading in, held, and fading out.
    fn frames(&self, fps: u32) -> impl Iterator<Item = Vec<u8>> + '_ {
        let count = (INTRO_SECONDS * f64::from(fps)).round() as u32;
        (0..count).map(move |index| self.frame(fade(f64::from(index) / f64::from(fps))))
    }
}

/// How visible the card is `seconds` into the intro: smooth in, steady, smooth out.
fn fade(seconds: f64) -> f64 {
    let smooth = |t: f64| {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    smooth(seconds / INTRO_FADE_SECONDS).min(smooth((INTRO_SECONDS - seconds) / INTRO_FADE_SECONDS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_fades_in_holds_and_fades_out() {
        assert!(fade(0.0) < 0.01);
        assert!((fade(INTRO_SECONDS / 2.0) - 1.0).abs() < 1e-9);
        assert!(fade(INTRO_SECONDS - 0.01) < 0.05);
        assert!(fade(0.2) > fade(0.1), "rising");
    }

    #[test]
    fn a_card_frame_is_the_right_size_and_shows_the_picture_only_when_visible() {
        let card = IntroCard::new(320, 180);
        let hidden = card.frame(0.0);
        let shown = card.frame(1.0);
        assert_eq!(hidden.len(), 320 * 180 * 3);
        assert_eq!(shown.len(), hidden.len());
        assert!(hidden.chunks(3).all(|pixel| pixel == INTRO_BACKGROUND), "nothing but the background");
        assert!(shown.chunks(3).any(|pixel| pixel != INTRO_BACKGROUND), "the picture appears");
    }

    #[test]
    fn the_intro_has_a_frame_for_every_tick_of_its_length() {
        let card = IntroCard::new(64, 36);
        assert_eq!(card.frames(10).count(), (INTRO_SECONDS * 10.0).round() as usize);
    }

    #[test]
    fn a_section_runs_between_its_ends_and_nothing_is_not_a_video() {
        let mut options = VideoOptions::default();
        assert_eq!(video_range(&options, 10.0).unwrap(), (LEAD_IN, 10.0 + TAIL_SECONDS));
        options.from = Some(3.0);
        options.to = Some(7.0);
        assert_eq!(video_range(&options, 10.0).unwrap(), (3.0, 7.0));
        options.to = Some(3.0);
        assert!(video_range(&options, 10.0).is_err());
    }

    #[test]
    fn the_soundtrack_waits_for_the_intro_only_when_there_is_one() {
        let mut options = VideoOptions::default();
        assert_eq!(intro_delay_filter(&options), vec!["-af".to_string(), "adelay=2600:all=1".to_string()]);
        options.intro = false;
        assert!(intro_delay_filter(&options).is_empty());
    }
}
