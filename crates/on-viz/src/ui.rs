//! The menu bar, the transport bar, and the things they do.
//!
//! Everything here is a thin layer over work that already exists: opening a score is
//! [`crate::session::open`], the same call the command line makes; changing the hand
//! size re-solves with [`on_fingering`]; exporting runs the same `opennote render`
//! that can be typed by hand. So a piece looks the same whether it was opened from a
//! menu or named on the command line, and nothing the interface can do is unavailable
//! without it.
//!
//! The one thing done at arm's length is the video export. It needs a Bevy app of its
//! own and there is already one running, so the menu launches the program again as a
//! child process and watches it. That also means an export cannot take the window
//! down with it, and the piece can go on playing while it runs.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use on_hand::{HandProfile, HandSize};

use crate::layout::Extent;
use crate::render::{Audio, Performance, Transport, ViewportInset};
use crate::session::{self, SessionSettings};

/// What the window knows about the piece it is showing.
#[derive(Resource)]
pub struct Session {
    /// How the fingering and the layout are worked out.
    pub settings: SessionSettings,
    /// The file the current performance came from, if it came from one.
    pub path: Option<PathBuf>,
    /// A line of feedback for the user: what was just opened, or what went wrong.
    pub message: String,
    /// A file dialog or an export running on another thread.
    task: Option<Task>,
}

impl Session {
    /// Start a session with the given settings and, optionally, an open file.
    pub fn new(settings: SessionSettings, path: Option<PathBuf>) -> Self {
        Self {
            settings,
            path,
            message: String::new(),
            task: None,
        }
    }

    /// Whether something is running that should stop the menus being used again.
    fn busy(&self) -> bool {
        self.task.is_some()
    }
}

/// Something running off the main thread, with somewhere to put its answer.
struct Task {
    /// What it is, for the status line.
    what: &'static str,
    /// Set when the work is over.
    done: Arc<AtomicBool>,
    /// A file to open, once one has been chosen.
    chosen: Arc<Mutex<Option<PathBuf>>>,
    /// What to say when it finishes.
    outcome: Arc<Mutex<String>>,
}

/// The menu bar and the transport bar.
pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .add_systems(EguiPrimaryContextPass, draw_menus)
            .add_systems(Update, (finish_tasks, apply_reload).chain());
    }
}

/// Set when something has changed that means the scene has to be built again.
#[derive(Resource, Default)]
pub struct Reload {
    /// A different file to show. `None` means re-solve the one already open.
    pub path: Option<PathBuf>,
    /// Whether anything is wanted at all.
    pub wanted: bool,
}

impl Reload {
    /// Ask for a different file.
    fn open(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.wanted = true;
    }

    /// Ask for the current file to be worked out again, with new settings.
    fn resolve(&mut self) {
        self.wanted = true;
    }
}

/// Draw the interface.
fn draw_menus(
    mut contexts: EguiContexts,
    mut session: ResMut<Session>,
    mut transport: ResMut<Transport>,
    mut inset: ResMut<ViewportInset>,
    mut reload: ResMut<Reload>,
    performance: Res<Performance>,
    audio: Res<Audio>,
    mut exit: MessageWriter<AppExit>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    // The scene is drawn by the camera underneath, so the panels are laid out in a
    // background layer over the whole viewport rather than in a window of their own.
    let mut root = egui::Ui::new(
        ctx.clone(),
        "opennote-chrome".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );

    let busy = session.busy();
    let mut jump: Option<f64> = None;
    let mut playing_changed = false;
    let mut speed_changed = false;

    let top = egui::Panel::top("menu").show(&mut root, |ui| {
        ui.add_enabled_ui(!busy, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open score…").clicked() {
                        choose_score(&mut session);
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            session.path.is_some(),
                            egui::Button::new("Reload from disk"),
                        )
                        .clicked()
                    {
                        reload.resolve();
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(session.path.is_some(), egui::Button::new("Export video…"))
                        .clicked()
                    {
                        export_video(&mut session);
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            session.path.is_some(),
                            egui::Button::new("Save fingered score…"),
                        )
                        .clicked()
                    {
                        annotate_score(&mut session);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        exit.write(AppExit::Success);
                    }
                });

                ui.menu_button("Playback", |ui| {
                    let label = if transport.playing { "Pause" } else { "Play" };
                    if ui.button(label).clicked() {
                        transport.playing = !transport.playing;
                        playing_changed = true;
                        ui.close();
                    }
                    if ui.button("Back to the start").clicked() {
                        jump = Some(crate::render::LEAD_IN);
                        ui.close();
                    }
                    ui.checkbox(&mut transport.looping, "Loop");
                    ui.separator();
                    ui.label("Speed");
                    for (label, speed) in
                        [("Half", 0.5), ("Three quarters", 0.75), ("Full", 1.0)]
                    {
                        if ui
                            .radio(
                                (transport.speed - speed).abs() < 1e-6,
                                format!("{label} ({speed:.2}×)"),
                            )
                            .clicked()
                        {
                            transport.speed = speed;
                            speed_changed = true;
                        }
                    }
                });

                ui.menu_button("Fingering", |ui| {
                    ui.label("Hand size");
                    let current = session.settings.fingering.profile.size();
                    for size in [
                        HandSize::ExtraSmall,
                        HandSize::Small,
                        HandSize::Medium,
                        HandSize::Large,
                        HandSize::ExtraLarge,
                    ] {
                        if ui.radio(current == size, hand_size_label(size)).clicked() {
                            session.settings.fingering =
                                on_fingering::FingeringOptions::for_hand(HandProfile::from_size(
                                    size,
                                ));
                            reload.resolve();
                        }
                    }
                    ui.separator();
                    // No rule-set picker. Choosing between Parncutt, Jacobs, Balliauw
                    // and Badgerow is not a decision anyone opening a score should have
                    // to make, and it was never really a choice between four opinions
                    // about the answer — they are four variations on one model that
                    // differ about which rules matter. All four are used, every time.
                    ui.label("Fingered with every published rule set at once");
                });

                ui.menu_button("Sound", |ui| {
                    let sounds = on_audio::sampled::available(&session.settings.assets_root);
                    if sounds.is_empty() {
                        ui.label("No SoundFonts in assets/soundfont");
                    }
                    // Whichever would be picked if nothing were chosen, so the menu
                    // shows the truth rather than an empty selection on first open.
                    let chosen = session.settings.soundfont.clone().or_else(|| sounds.first().cloned());
                    for sound in sounds {
                        let picked = chosen.as_deref() == Some(sound.as_path());
                        if ui.radio(picked, soundfont_label(&sound)).clicked() && !picked {
                            session.settings.soundfont = Some(sound);
                            reload.resolve();
                        }
                    }
                });

                ui.menu_button("View", |ui| {
                    let full = session.settings.extent == Extent::FullKeyboard;
                    if ui.radio(full, "All eighty-eight keys").clicked() && !full {
                        session.settings.extent = Extent::FullKeyboard;
                        reload.resolve();
                    }
                    if ui.radio(!full, "Only the keys this piece uses").clicked() && full {
                        session.settings.extent = Extent::MusicRange;
                        reload.resolve();
                    }
                    ui.separator();
                    let mut lookahead = session.settings.lookahead as f32;
                    if ui
                        .add(
                            egui::Slider::new(&mut lookahead, 1.0..=8.0)
                                .text("Seconds of notes ahead"),
                        )
                        .changed()
                    {
                        session.settings.lookahead = f64::from(lookahead);
                        reload.resolve();
                    }
                });

                ui.separator();
                let title = performance
                    .timeline
                    .title
                    .clone()
                    .unwrap_or_else(|| "no score open".into());
                ui.label(egui::RichText::new(title).strong());
                let note = match &session.task {
                    Some(task) => format!("{}…", task.what),
                    None => session.message.clone(),
                };
                if !note.is_empty() {
                    ui.separator();
                    ui.label(note);
                }
            });
        });
    });
    inset.top = top.response.rect.height();

    // A seek bar along the bottom. Dragging it is how most people expect to move
    // through a piece, and it is the one control the keyboard shortcuts cannot offer.
    let bottom = egui::Panel::bottom("transport").show(&mut root, |ui| {
        ui.horizontal(|ui| {
            let label = if transport.playing { "⏸" } else { "▶" };
            if ui.button(label).clicked() {
                transport.playing = !transport.playing;
                playing_changed = true;
            }
            if ui.button("⏮").clicked() {
                jump = Some(crate::render::LEAD_IN);
            }

            let duration = performance.timeline.duration.max(0.001);
            let mut position = transport.position.clamp(0.0, duration);
            let slider = ui.add_sized(
                [ui.available_width() - 120.0, 20.0],
                egui::Slider::new(&mut position, 0.0..=duration).show_value(false),
            );
            if slider.changed() {
                jump = Some(position);
            }
            ui.label(format!(
                "{} / {}",
                clock(transport.position.max(0.0)),
                clock(duration)
            ));
        });
    });
    inset.bottom = bottom.response.rect.height();

    if let Some(target) = jump {
        transport.seek_to(target);
        audio.seek(target);
    }
    if playing_changed {
        audio.set_playing(transport.playing);
    }
    if speed_changed {
        audio.set_speed(transport.speed);
    }
    Ok(())
}

/// Minutes and seconds.
fn clock(seconds: f64) -> String {
    let whole = seconds.max(0.0) as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// The name a hand size goes by in the interface.
fn hand_size_label(size: HandSize) -> &'static str {
    match size {
        HandSize::ExtraSmall => "Extra small",
        HandSize::Small => "Small",
        HandSize::Medium => "Medium",
        HandSize::Large => "Large",
        HandSize::ExtraLarge => "Extra large",
    }
}

/// What a SoundFont is called in the menu: its file name, without the extension.
fn soundfont_label(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("SoundFont")
        .to_owned()
}

/// Ask for a score to open.
///
/// The dialog runs on a thread of its own. A native file dialog blocks until the user
/// answers it, and blocking the main thread would freeze the piece mid-bar.
fn choose_score(session: &mut Session) {
    let chosen: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let done = Arc::new(AtomicBool::new(false));
    let outcome = Arc::new(Mutex::new(String::new()));

    let start = session
        .path
        .as_ref()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let (result, finished) = (Arc::clone(&chosen), Arc::clone(&done));
    std::thread::spawn(move || {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open a score")
            .add_filter("Scores", &on_score::Document::EXTENSIONS)
            .add_filter("All files", &["*"]);
        if let Some(directory) = start {
            dialog = dialog.set_directory(directory);
        }
        if let Some(path) = dialog.pick_file() {
            *result.lock().unwrap() = Some(path);
        }
        finished.store(true, Ordering::Release);
    });

    session.task = Some(Task {
        what: "opening",
        done,
        chosen,
        outcome,
    });
}

/// Ask where to put a video, then render it in another process.
fn export_video(session: &mut Session) {
    let Some(input) = session.path.clone() else {
        return;
    };
    let done = Arc::new(AtomicBool::new(false));
    let outcome = Arc::new(Mutex::new(String::new()));
    let (finished, report) = (Arc::clone(&done), Arc::clone(&outcome));
    let suggestion = input.with_extension("mp4");
    let extent = session.settings.extent;

    std::thread::spawn(move || {
        let chosen = rfd::FileDialog::new()
            .set_title("Export video")
            .set_file_name(
                suggestion
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("opennote.mp4"),
            )
            .add_filter("MP4 video", &["mp4"])
            .save_file();
        if let Some(output) = chosen {
            *report.lock().unwrap() = run_self(
                &["render"],
                &input,
                &output,
                extent,
                &format!("exported {}", output.display()),
            );
        }
        finished.store(true, Ordering::Release);
    });

    session.task = Some(Task {
        what: "exporting",
        done,
        chosen: Arc::new(Mutex::new(None)),
        outcome,
    });
}

/// Ask where to put the fingered score, then write it in another process.
fn annotate_score(session: &mut Session) {
    let Some(input) = session.path.clone() else {
        return;
    };
    let done = Arc::new(AtomicBool::new(false));
    let outcome = Arc::new(Mutex::new(String::new()));
    let (finished, report) = (Arc::clone(&done), Arc::clone(&outcome));
    let suggestion = input.with_extension("fingered.musicxml");
    let extent = session.settings.extent;

    std::thread::spawn(move || {
        let chosen = rfd::FileDialog::new()
            .set_title("Save the fingered score")
            .set_file_name(
                suggestion
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("fingered.musicxml"),
            )
            .add_filter("MusicXML", &["musicxml", "mxl"])
            .save_file();
        if let Some(output) = chosen {
            *report.lock().unwrap() = run_self(
                &["annotate"],
                &input,
                &output,
                extent,
                &format!("saved {}", output.display()),
            );
        }
        finished.store(true, Ordering::Release);
    });

    session.task = Some(Task {
        what: "saving",
        done,
        chosen: Arc::new(Mutex::new(None)),
        outcome,
    });
}

/// Run this same program again with a different subcommand, and report how it went.
fn run_self(
    subcommand: &[&str],
    input: &std::path::Path,
    output: &std::path::Path,
    extent: Extent,
    success: &str,
) -> String {
    let Ok(program) = std::env::current_exe() else {
        return "could not work out where this program lives".into();
    };
    let mut command = std::process::Command::new(program);
    command.args(subcommand).arg(input).arg("--out").arg(output);
    if extent == Extent::MusicRange {
        command.arg("--crop-keyboard");
    }
    match command.status() {
        Ok(status) if status.success() => success.to_string(),
        Ok(status) => format!("that did not work ({status})"),
        Err(error) => format!("could not start: {error}"),
    }
}

/// Collect the answer from whatever was running off the main thread.
fn finish_tasks(mut session: ResMut<Session>, mut reload: ResMut<Reload>) {
    let Some(task) = &session.task else {
        return;
    };
    if !task.done.load(Ordering::Acquire) {
        return;
    }
    let chosen = task.chosen.lock().unwrap().clone();
    let outcome = task.outcome.lock().unwrap().clone();
    session.task = None;

    if let Some(path) = chosen {
        reload.open(path);
    } else if !outcome.is_empty() {
        session.message = outcome;
    }
}

/// Build the scene again, because the score or the settings changed.
fn apply_reload(world: &mut World) {
    if !world.resource::<Reload>().wanted {
        return;
    }
    let (path, settings) = {
        let reload = world.resource::<Reload>();
        let session = world.resource::<Session>();
        (
            reload.path.clone().or_else(|| session.path.clone()),
            session.settings.clone(),
        )
    };
    world.resource_mut::<Reload>().wanted = false;
    world.resource_mut::<Reload>().path = None;

    let Some(path) = path else {
        return;
    };
    let performance = match session::open(&path, &settings) {
        Ok(performance) => performance,
        Err(error) => {
            let mut session = world.resource_mut::<Session>();
            session.message = format!("could not open {}: {error:#}", path.display());
            return;
        }
    };

    {
        let mut session = world.resource_mut::<Session>();
        session.message = String::new();
        session.path = Some(path);
    }

    // The audio is told about the new piece before the scene is, so that the clock
    // the view follows is already counting the right notes when the first frame of
    // the new scene is drawn.
    let notes = performance.timeline.audio_notes();
    {
        let audio = world.resource::<Audio>();
        audio.load(notes);
        // Only when it has actually changed. Opening a SoundFont means reading a file
        // that can run to a gigabyte, and every other setting in the menus reloads the
        // piece too.
        let playing = world.resource::<Performance>().soundfont.clone();
        if playing != performance.soundfont {
            audio.set_instrument(performance.soundfont.clone());
        }
    }
    world.insert_resource(performance);
    crate::render::rebuild_scene(world);

    let mut transport = world.resource_mut::<Transport>();
    let start = crate::render::LEAD_IN;
    transport.seek_to(start);
    let playing = transport.playing;
    let speed = transport.speed;
    let audio = world.resource::<Audio>();
    audio.seek(start);
    audio.set_speed(speed);
    audio.set_playing(playing);
}
