//! Renders the performance offscreen, frame by frame, and pipes it to ffmpeg with the
//! soundtrack.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

use crate::render::{Audio, Performance, Transport, VisualizerPlugin, LEAD_IN};

const TAIL_SECONDS: f64 = 2.0;

/// How long the "Made with OpenNote" card is on screen at full strength, from the first frame.
pub const INTRO_HOLD_SECONDS: f64 = 2.0;

/// How long it then takes to fade away.
const INTRO_FADE_SECONDS: f64 = 0.5;

/// The whole intro: the card, then its fade. The piece and its sound begin after this.
pub const INTRO_SECONDS: f64 = INTRO_HOLD_SECONDS + INTRO_FADE_SECONDS;

/// The colour the stage starts from, so the card fades into the piece without a flash.
const INTRO_BACKGROUND: [u8; 4] = [0x0D, 0x0E, 0x12, 0xFF];

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
    /// The ffmpeg video encoder: libx264 on the CPU, or a hardware one such as h264_nvenc.
    pub codec: String,
    /// An ffmpeg filter for the picture, applied as it is encoded: burnt-in subtitles, say.
    pub video_filter: Option<String>,
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
            codec: "libx264".into(),
            video_filter: None,
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
    /// The next frame to ask for.
    frame: u32,
    /// The next frame to hand to the encoder; frames read back early wait in `ready`.
    sent: u32,
    total: u32,
    warmup: u32,
    /// Screenshots asked for and not yet read back.
    in_flight: u32,
    ready: BTreeMap<u32, Vec<u8>>,
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
    let card = options.intro.then(|| IntroCard::new(options.width, options.height));
    let intro_frames = card.as_ref().map_or(0, |card| card.frame_count(options.fps));

    let mut encoder = start_ffmpeg(&ffmpeg, &options, &audio_path, intro_frames + total)?;
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

    if let Some(card) = &card {
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
    app.add_plugins(headless_plugins(&assets))
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

/// The export draws only into an offscreen image, so it needs no window, and is faster
/// without one: a window that never has focus is held to 60 updates a second, and each
/// screenshot took about three of them, so a 60 fps video was made at about 20 frames a
/// second. Without a window, the app updates as fast as the frames are drawn.
fn headless_plugins(assets: &Path) -> impl PluginGroup {
    let mut plugins = DefaultPlugins
        .build()
        .disable::<bevy::log::LogPlugin>()
        .disable::<bevy::winit::WinitPlugin>()
        .set(WindowPlugin {
            primary_window: None,
            exit_condition: bevy::window::ExitCondition::DontExit,
            ..default()
        })
        .add(bevy::app::ScheduleRunnerPlugin::run_loop(std::time::Duration::ZERO));
    if let Some(path) = assets.to_str() {
        plugins = plugins.set(AssetPlugin { file_path: path.to_string(), ..default() });
    }
    plugins
}

#[derive(Resource, Deref)]
struct ExportTarget(Handle<Image>);

const FRAMES_IN_FLIGHT: usize = 8;

/// How many frames are being drawn and read back from the GPU at once. With one, the GPU
/// and the CPU take turns; with several, the read-back of one frame overlaps the drawing
/// of the next.
const SCREENSHOTS_IN_FLIGHT: u32 = 8;

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
            sent: 0,
            total: pending.total,
            warmup: WARMUP_FRAMES,
            in_flight: 0,
            ready: BTreeMap::new(),
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
    if export.sent >= export.total {
        exit.write(AppExit::Success);
        return;
    }
    if export.in_flight >= SCREENSHOTS_IN_FLIGHT || export.frame >= export.total {
        return;
    }

    transport.position = export.moment(export.frame);
    transport.playing = false;
    export.in_flight += 1;

    let index = export.frame;
    export.frame += 1;
    let handle = export.target.clone();
    commands.spawn(Screenshot::image(handle)).observe(
        move |captured: On<ScreenshotCaptured>, mut export: ResMut<Export>| {
            let pixels = frame_bytes(&captured.image, export.options.width, export.options.height);
            if pixels.is_none() {
                error!("could not read frame {index} back");
                export.trouble.store(true, Ordering::Relaxed);
            }
            export.in_flight -= 1;
            export.ready.insert(index, pixels.unwrap_or_default());
            hand_over_ready_frames(&mut export);
        },
    );
}

/// Screenshots can come back out of order; the encoder takes them strictly in order.
fn hand_over_ready_frames(export: &mut Export) {
    while let Some(frame) = export.ready.remove(&export.sent) {
        if export.frames.send(frame).is_err() {
            error!("the encoder went away partway through the export");
            export.trouble.store(true, Ordering::Relaxed);
        }
        if export.sent.is_multiple_of(10) {
            println!("  frame {} of {}", export.sent, export.total);
        }
        export.sent += 1;
    }
}

/// The frame's pixels as ffmpeg's `rgba`. The target is 8-bit RGBA with no padding between
/// rows, so its bytes go over as they are, which keeps a per-pixel conversion off the thread
/// that draws.
fn frame_bytes(image: &Image, width: u32, height: u32) -> Option<Vec<u8>> {
    let expected = width as usize * height as usize * 4;
    image.data.as_ref().filter(|data| data.len() == expected).cloned()
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

fn start_ffmpeg(ffmpeg: &Path, options: &VideoOptions, audio: &Path, frames: u32) -> Result<Child> {
    let mut command = Command::new(ffmpeg);
    command
        .arg("-y")
        .args(["-loglevel", "error"])
        .args(["-f", "rawvideo", "-pixel_format", "rgba"])
        .args(["-video_size", &format!("{}x{}", options.width, options.height)])
        .args(["-framerate", &options.fps.to_string()])
        .args(["-i", "-"])
        .arg("-i")
        .arg(audio)
        .args(video_encoder_args(options))
        .args(["-c:a", "aac", "-b:a", "192k"])
        .args(intro_delay_filter(options))
        // The exact length rather than -shortest, which makes ffmpeg hold seconds of raw
        // frames in its queue while it waits to see which stream ends first: about 2 GB of
        // memory at 1080p.
        .args(["-t", &video_length(frames, options.fps)])
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

/// The encoder, its quality, and the picture filter, as ffmpeg arguments.
///
/// x264 and x265 keep their medium preset and take --crf as it is. NVIDIA's encoders take it as
/// their constant quality level, and take the RGBA frames as they are, converting them to 4:2:0
/// on the GPU, unless a filter needs them converted first. Any other encoder gets the quality
/// and the pixel format only, since presets differ from one encoder to the next.
fn video_encoder_args(options: &VideoOptions) -> Vec<String> {
    let codec = options.codec.as_str();
    let crf = options.crf.to_string();
    let nvenc = codec.ends_with("_nvenc");
    let mut args: Vec<String> = vec!["-c:v".into(), codec.into()];
    if !nvenc || options.video_filter.is_some() {
        args.extend(["-pix_fmt".into(), "yuv420p".into()]);
    }
    if nvenc {
        args.extend(
            ["-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", &crf, "-b:v", "0"].map(String::from),
        );
    } else if matches!(codec, "libx264" | "libx265") {
        args.extend(["-preset", "medium", "-crf", &crf].map(String::from));
    } else {
        args.extend(["-crf".into(), crf]);
    }
    if let Some(filter) = &options.video_filter {
        args.extend(["-vf".into(), filter.clone()]);
    }
    args
}

/// How long `frames` frames last, as ffmpeg's `-t` takes it.
fn video_length(frames: u32, fps: u32) -> String {
    format!("{:.6}", f64::from(frames) / f64::from(fps))
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
        let mut frame = Vec::with_capacity(self.width as usize * self.height as usize * 4);
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
            let at = (py as usize * self.width as usize + px as usize) * 4;
            for channel in 0..3 {
                let behind = f64::from(frame[at + channel]);
                frame[at + channel] = (behind + (f64::from(pixel[channel]) - behind) * alpha).round() as u8;
            }
        }
        frame
    }

    /// How many frames the intro takes.
    fn frame_count(&self, fps: u32) -> u32 {
        (INTRO_SECONDS * f64::from(fps)).round() as u32
    }

    /// Every frame of the intro: the card at full strength, then fading out.
    fn frames(&self, fps: u32) -> impl Iterator<Item = Vec<u8>> + '_ {
        (0..self.frame_count(fps)).map(move |index| self.frame(fade(f64::from(index) / f64::from(fps))))
    }
}

/// How visible the card is `seconds` into the intro: fully, for the hold, then smoothly less.
fn fade(seconds: f64) -> f64 {
    let t = ((seconds - INTRO_HOLD_SECONDS) / INTRO_FADE_SECONDS).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_is_there_from_the_first_frame_for_two_seconds_and_then_fades() {
        assert_eq!(fade(0.0), 1.0, "no fade in: it appears with the video");
        assert_eq!(fade(INTRO_HOLD_SECONDS - 0.01), 1.0, "solid for the whole two seconds");
        assert!(fade(INTRO_HOLD_SECONDS + INTRO_FADE_SECONDS / 2.0) < 0.6, "fading");
        assert!(fade(INTRO_HOLD_SECONDS + 0.1) > fade(INTRO_HOLD_SECONDS + 0.3), "ever fainter");
        assert!(fade(INTRO_SECONDS) < 0.001, "gone when the piece starts");
    }

    #[test]
    fn a_card_frame_is_the_right_size_and_shows_the_picture_only_when_visible() {
        let card = IntroCard::new(320, 180);
        let hidden = card.frame(0.0);
        let shown = card.frame(1.0);
        assert_eq!(hidden.len(), 320 * 180 * 4, "rgba, like the frames read back from the GPU");
        assert_eq!(shown.len(), hidden.len());
        assert!(hidden.chunks(4).all(|pixel| pixel == INTRO_BACKGROUND), "nothing but the background");
        assert!(shown.chunks(4).any(|pixel| pixel != INTRO_BACKGROUND), "the picture appears");
        assert!(shown.chunks(4).all(|pixel| pixel[3] == 0xFF), "the card itself is opaque");
    }

    #[test]
    fn the_intro_has_a_frame_for_every_tick_of_its_length() {
        let card = IntroCard::new(64, 36);
        assert_eq!(card.frames(10).count(), (INTRO_SECONDS * 10.0).round() as usize);
        assert_eq!(card.frame_count(10) as usize, card.frames(10).count());
    }

    #[test]
    fn x264_is_still_the_encoder_unless_another_is_asked_for() {
        let options = VideoOptions::default();
        assert_eq!(
            video_encoder_args(&options),
            ["-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "medium", "-crf", "18"]
        );
    }

    #[test]
    fn nvenc_takes_the_frames_as_they_are_unless_a_filter_needs_them_converted() {
        let mut options = VideoOptions { codec: "h264_nvenc".into(), crf: 23, ..VideoOptions::default() };
        let args = video_encoder_args(&options);
        assert!(!args.contains(&"-pix_fmt".to_string()), "the GPU converts RGBA itself");
        assert!(args.windows(2).any(|pair| pair == ["-cq", "23"]), "--crf is the constant quality");
        assert!(!args.contains(&"-crf".to_string()));
        options.video_filter = Some("subtitles=lines.ass".into());
        let args = video_encoder_args(&options);
        assert!(args.windows(2).any(|pair| pair == ["-pix_fmt", "yuv420p"]));
        assert!(args.ends_with(&["-vf".to_string(), "subtitles=lines.ass".to_string()]));
    }

    #[test]
    fn another_encoder_gets_no_preset_it_might_not_have() {
        let options = VideoOptions { codec: "h264_qsv".into(), ..VideoOptions::default() };
        assert_eq!(video_encoder_args(&options), ["-c:v", "h264_qsv", "-pix_fmt", "yuv420p", "-crf", "18"]);
    }

    #[test]
    fn the_video_is_cut_to_the_frames_it_was_given() {
        assert_eq!(video_length(150 + 3600, 60), "62.500000");
        assert_eq!(video_length(1, 30), "0.033333");
    }

    #[test]
    fn a_read_back_frame_is_taken_only_at_the_size_of_the_video() {
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
        let image = Image::new_fill(
            Extent3d { width: 4, height: 2, depth_or_array_layers: 1 },
            TextureDimension::D2,
            &[1, 2, 3, 255],
            TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::default(),
        );
        assert_eq!(frame_bytes(&image, 4, 2).map(|bytes| bytes.len()), Some(4 * 2 * 4));
        assert_eq!(frame_bytes(&image, 4, 2).unwrap()[..4], [1, 2, 3, 255]);
        assert!(frame_bytes(&image, 8, 2).is_none(), "a frame of the wrong size is not passed on");
    }

    #[test]
    fn frames_read_back_out_of_order_reach_the_encoder_in_order() {
        let (frames, incoming) = mpsc::sync_channel::<Vec<u8>>(16);
        let mut export = Export {
            options: VideoOptions::default(),
            target: Handle::default(),
            frame: 4,
            sent: 0,
            total: 4,
            warmup: 0,
            in_flight: 0,
            ready: BTreeMap::new(),
            frames,
            trouble: Arc::new(AtomicBool::new(false)),
            start: 0.0,
        };
        for index in [2u8, 0, 3, 1] {
            export.ready.insert(u32::from(index), vec![index]);
            hand_over_ready_frames(&mut export);
        }
        drop(export);
        let order: Vec<u8> = incoming.iter().map(|frame| frame[0]).collect();
        assert_eq!(order, vec![0, 1, 2, 3]);
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
        assert_eq!(intro_delay_filter(&options), vec!["-af".to_string(), "adelay=2500:all=1".to_string()]);
        options.intro = false;
        assert!(intro_delay_filter(&options).is_empty());
    }
}
