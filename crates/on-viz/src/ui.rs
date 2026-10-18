//! The menus, the transport bar and the shortcuts window, and the background tasks they
//! start.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use on_hand::{HandProfile, HandSize};

use crate::layout::Extent;
use crate::render::{Audio, Capture, Performance, Transport, ViewportInset};
use crate::session::{self, SessionSettings};

#[derive(Resource)]
pub struct Session {
    pub settings: SessionSettings,
    pub path: Option<PathBuf>,
    pub message: String,
    task: Option<Task>,
    help: bool,
}

impl Session {
    pub fn new(settings: SessionSettings, path: Option<PathBuf>) -> Self {
        Self { settings, path, message: String::new(), task: None, help: false }
    }

    fn busy(&self) -> bool {
        self.task.is_some()
    }
}

struct Task {
    what: &'static str,
    done: Arc<AtomicBool>,
    chosen: Arc<Mutex<Option<PathBuf>>>,
    outcome: Arc<Mutex<String>>,
}

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .add_plugins(FrameTimeDiagnosticsPlugin::default())
            .init_resource::<crate::updates::Updates>()
            .init_resource::<crate::shortcuts::Shortcuts>()
            .init_resource::<crate::welcome::Welcome>()
            .add_systems(Startup, crate::welcome::first_run)
            .add_systems(Startup, crate::updates::check_on_start)
            .add_systems(EguiPrimaryContextPass, draw_menus)
            .add_systems(Update, (finish_tasks, apply_reload).chain());
    }
}

#[derive(Resource, Default)]
pub struct Reload {
    pub path: Option<PathBuf>,
    pub wanted: bool,
    /// Only how the piece is laid out has changed: redraw it, without reading,
    /// fingering or rewinding it again.
    pub relayout: bool,
}

impl Reload {
    fn open(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.wanted = true;
    }

    fn resolve(&mut self) {
        self.wanted = true;
    }
}

fn draw_menus(
    mut contexts: EguiContexts,
    mut session: ResMut<Session>,
    mut transport: ResMut<Transport>,
    mut inset: ResMut<ViewportInset>,
    mut reload: ResMut<Reload>,
    performance: Res<Performance>,
    audio: Res<Audio>,
    mut visuals: ResMut<crate::render::Visuals>,
    mut playing: ResMut<crate::render::Playing>,
    diagnostics: Res<DiagnosticsStore>,
    capture: Option<Res<Capture>>,
    updates: Res<crate::updates::Updates>,
    shortcuts: Res<crate::shortcuts::Shortcuts>,
    welcome: Res<crate::welcome::Welcome>,
    mut exit: MessageWriter<AppExit>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    let mut root = egui::Ui::new(
        ctx.clone(),
        "opennote-chrome".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()),
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
                    if ui.add_enabled(session.path.is_some(), egui::Button::new("Reload from disk")).clicked()
                    {
                        reload.resolve();
                        ui.close();
                    }
                    ui.separator();
                    if ui.add_enabled(session.path.is_some(), egui::Button::new("Export video…")).clicked()
                    {
                        export_video(&mut session);
                        ui.close();
                    }
                    if ui
                        .add_enabled(session.path.is_some(), egui::Button::new("Save fingered score…"))
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
                    ui.label("Hands played");
                    for (hands, label) in [
                        ([true, true], "Both"),
                        ([true, false], "Left only: you play the right"),
                        ([false, true], "Right only: you play the left"),
                    ] {
                        if ui.radio(playing.0 == hands, label).clicked() {
                            playing.set_if_neq(crate::render::Playing(hands));
                        }
                    }
                    ui.separator();
                    ui.label("Speed");
                    for (label, speed) in [("Half", 0.5), ("Three quarters", 0.75), ("Full", 1.0)] {
                        if ui
                            .radio((transport.speed - speed).abs() < 1e-6, format!("{label} ({speed:.2}×)"))
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
                                on_fingering::FingeringOptions::for_hand(HandProfile::from_size(size));
                            reload.resolve();
                        }
                    }
                    ui.separator();
                    ui.label("Fingered with every published rule set at once");
                });

                ui.menu_button("Sound", |ui| {
                    let sounds = on_audio::sampled::available(&session.settings.assets_root);
                    if sounds.is_empty() {
                        ui.label("No SoundFonts in assets/soundfont");
                    }
                    let chosen = session.settings.soundfont.clone().or_else(|| sounds.first().cloned());
                    for sound in sounds {
                        let picked = chosen.as_deref() == Some(sound.as_path());
                        if ui.radio(picked, soundfont_label(&sound)).clicked() && !picked {
                            session.settings.soundfont = Some(sound);
                            reload.resolve();
                        }
                    }
                });

                ui.menu_button("Expressiveness (BETA)", |ui| {
                    ui.label("Mood");
                    for mood in on_score::expression::Mood::ALL {
                        let picked = session.settings.mood.name == mood.name;
                        if ui.radio(picked, mood.name).clicked() && !picked {
                            session.settings.mood = mood;
                            reload.resolve();
                        }
                    }
                    ui.separator();
                    ui.label("How much");
                    for (amount, label) in EXPRESSION_STEPS {
                        let picked = (session.settings.expression - amount).abs() < 1e-3;
                        if ui.radio(picked, label).clicked() && !picked {
                            session.settings.expression = amount;
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
                        .add(egui::Slider::new(&mut lookahead, 1.0..=8.0).text("Seconds of notes ahead"))
                        .changed()
                    {
                        session.settings.lookahead = f64::from(lookahead);
                        reload.relayout = true;
                    }
                    ui.separator();
                    // Edited as a copy: taking the resource mutably every frame would mark it
                    // changed every frame, and the camera would be rebuilt every frame.
                    let mut wanted = *visuals;
                    ui.checkbox(&mut wanted.effects, "Sparks, flashes and glow");
                    ui.checkbox(&mut wanted.high_quality, "Shadows (slower)");
                    ui.checkbox(
                        &mut wanted.ambient_shading,
                        "Soft contact shading (can blur and shimmer in motion)",
                    );
                    visuals.set_if_neq(wanted);
                    ui.separator();
                    ui.label("Hand colours");
                    if hand_colour_pickers(ui, &mut session) {
                        reload.relayout = true;
                    }
                    for (colours, label) in [
                        (crate::render::HAND_COLOURS, "Blue and yellow"),
                        (crate::render::CLEAR_HAND_COLOURS, "Orange and sky blue, for colour blindness"),
                    ] {
                        if ui.radio(session.settings.note_colours == colours, label).clicked() {
                            session.settings.note_colours = colours;
                            reload.relayout = true;
                        }
                    }
                });

                ui.menu_button("Help", |ui| {
                    if ui.button("Welcome").clicked() {
                        welcome.reopen();
                        ui.close();
                    }
                    if ui.button("Check for updates…").clicked() {
                        updates.check(true);
                        ui.close();
                    }
                    if ui.button("Keyboard shortcuts    F1").clicked() {
                        session.help = !session.help;
                        ui.close();
                    }
                });

                ui.separator();
                let title = performance.timeline.title.clone().unwrap_or_else(|| "no score open".into());
                ui.label(egui::RichText::new(title).strong());
                let note = match &session.task {
                    Some(task) => format!("{}…", task.what),
                    None => session.message.clone(),
                };
                if !note.is_empty() {
                    ui.separator();
                    ui.label(note);
                }

                if capture.is_none() {
                    if let Some(fps) =
                        diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS).and_then(|fps| fps.smoothed())
                    {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(format!("{fps:.0} fps")).monospace().weak());
                        });
                    }
                }
            });
        });
    });
    inset.top = top.response.rect.height();

    let bottom = egui::Panel::bottom("transport").show(&mut root, |ui| {
        let duration = performance.timeline.duration.max(0.001);
        let sought = crate::seekbar::show(
            ui,
            transport.position.clamp(0.0, duration),
            duration,
            &performance.timeline.bars,
            &mut transport.section,
            0.0,
        );
        if let Some(target) = sought.seek {
            jump = Some(target);
        }
        ui.horizontal(|ui| {
            let label = if transport.playing { "⏸" } else { "▶" };
            if ui.button(label).clicked() {
                transport.playing = !transport.playing;
                playing_changed = true;
            }
            if ui.button("⏮").clicked() {
                jump = Some(crate::render::LEAD_IN);
            }
            ui.label(format!("{} / {}", clock(transport.position.max(0.0)), clock(duration)));

            ui.separator();
            let here = transport.position.max(0.0);
            if ui.button("A").on_hover_text("Practise from here  [").clicked() {
                transport.mark_start(here);
            }
            if ui.button("B").on_hover_text("Practise up to here  ]").clicked() {
                transport.mark_end(here);
            }
            match transport.section {
                Some((start, end)) => {
                    let end = if end.is_finite() { clock(end) } else { "end".to_string() };
                    ui.label(format!("{}–{end}", clock(start.max(0.0))));
                    if ui.button("Clear").on_hover_text("Play the whole piece  \\").clicked() {
                        transport.section = None;
                    }
                }
                None => {
                    ui.weak("whole piece");
                }
            }

            ui.separator();
            let mut speed = transport.speed;
            let dragged = ui
                .add(
                    egui::DragValue::new(&mut speed).range(0.1..=4.0).speed(0.01).max_decimals(2).suffix("×"),
                )
                .on_hover_text("Speed  ↑ ↓");
            if dragged.changed() {
                transport.speed = speed;
                speed_changed = true;
            }
        });
    });
    inset.bottom = bottom.response.rect.height();

    crate::welcome::show(&ctx, &welcome);
    if !welcome.open() {
        crate::updates::show(&ctx, &updates, &mut exit);
        crate::shortcuts::show(&ctx, &shortcuts, updates.busy());
    }

    if session.path.is_none() && !welcome.open() {
        start_screen(&ctx, &mut session);
    }

    if ctx.input(|i| i.key_pressed(egui::Key::F1) || i.key_pressed(egui::Key::Questionmark)) {
        session.help = !session.help;
    }
    if session.help {
        let mut open = true;
        egui::Window::new("Keyboard shortcuts")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(&ctx, |ui| {
                egui::Grid::new("shortcuts").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                    for (keys, what) in crate::SHORTCUTS {
                        ui.label(egui::RichText::new(*keys).monospace().strong());
                        ui.label(*what);
                        ui.end_row();
                    }
                });
            });
        session.help = open;
    }

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

fn to_egui(colour: Color) -> egui::Color32 {
    let rgba = colour.to_srgba();
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgb(byte(rgba.red), byte(rgba.green), byte(rgba.blue))
}

fn from_egui(colour: egui::Color32) -> Color {
    Color::srgb_u8(colour.r(), colour.g(), colour.b())
}

/// One colour button for each hand's notes, left first. Returns whether either changed.
fn hand_colour_pickers(ui: &mut egui::Ui, session: &mut Session) -> bool {
    let mut changed = false;
    for (index, label) in ["Left hand notes", "Right hand notes"].into_iter().enumerate() {
        ui.horizontal(|ui| {
            let mut colour = to_egui(session.settings.note_colours[index]);
            if ui.color_edit_button_srgba(&mut colour).changed() {
                session.settings.note_colours[index] = from_egui(colour);
                changed = true;
            }
            ui.label(label);
        });
    }
    changed
}

/// The "Expressiveness (BETA)" choice. Returns whether it changed.
fn expressiveness_picker(ui: &mut egui::Ui, session: &mut Session) -> bool {
    let current = EXPRESSION_STEPS
        .iter()
        .find(|(amount, _)| (session.settings.expression - amount).abs() < 1e-3)
        .map_or("Custom", |(_, label)| *label);
    let mut changed = false;
    egui::ComboBox::from_label(egui::RichText::new("Expressiveness (BETA)"))
        .selected_text(current)
        .width(220.0)
        .show_ui(ui, |ui| {
            for (amount, label) in EXPRESSION_STEPS {
                let picked = (session.settings.expression - amount).abs() < 1e-3;
                if ui.selectable_label(picked, label).clicked() && !picked {
                    session.settings.expression = amount;
                    changed = true;
                }
            }
        });
    changed
}

/// The first thing seen when no piece is open: choose one, and the few settings that
/// matter to someone who just wants to watch a piece.
fn start_screen(ctx: &egui::Context, session: &mut Session) {
    egui::Window::new("OpenNote")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, -40.0])
        .show(ctx, |ui| {
            ui.set_width(320.0);
            ui.vertical_centered(|ui| {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("OpenNote").size(28.0).strong());
                ui.label("Pick a piece to see it played, with the fingering shown on the hands.");
                ui.add_space(14.0);
                let pick = egui::Button::new(egui::RichText::new("Select piece…").size(20.0))
                    .min_size(egui::vec2(280.0, 54.0));
                if ui.add_enabled(!session.busy(), pick).clicked() {
                    choose_score(session);
                }
                ui.add_space(4.0);
                ui.weak("MusicXML (.musicxml, .mxl, .xml) or MIDI (.mid)");
                ui.add_space(14.0);
            });
            ui.separator();
            ui.add_space(6.0);
            hand_colour_pickers(ui, session);
            ui.add_space(6.0);
            expressiveness_picker(ui, session);
            ui.add_space(6.0);
            if !session.message.is_empty() {
                ui.separator();
                ui.colored_label(ui.visuals().warn_fg_color, session.message.clone());
            }
        });
}

const EXPRESSION_STEPS: [(f32, &str); 5] = [
    (0.0, "None: exactly as written"),
    (0.5, "A little"),
    (1.0, "Natural"),
    (1.5, "A lot"),
    (2.0, "As much as possible"),
];

/// A time to a tenth of a second, for the tooltip on the progress bar.
pub(crate) fn clock_precise(seconds: f64) -> String {
    let tenths = (seconds.max(0.0) * 10.0).round() as u64;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

fn clock(seconds: f64) -> String {
    let whole = seconds.max(0.0) as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

fn hand_size_label(size: HandSize) -> &'static str {
    match size {
        HandSize::ExtraSmall => "Extra small",
        HandSize::Small => "Small",
        HandSize::Medium => "Medium",
        HandSize::Large => "Large",
        HandSize::ExtraLarge => "Extra large",
    }
}

fn soundfont_label(path: &std::path::Path) -> String {
    path.file_stem().and_then(|name| name.to_str()).unwrap_or("SoundFont").to_owned()
}

fn choose_score(session: &mut Session) {
    let chosen: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let done = Arc::new(AtomicBool::new(false));
    let outcome = Arc::new(Mutex::new(String::new()));

    let start = session.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()));
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

    session.task = Some(Task { what: "opening", done, chosen, outcome });
}

fn export_video(session: &mut Session) {
    let Some(input) = session.path.clone() else {
        return;
    };
    let done = Arc::new(AtomicBool::new(false));
    let outcome = Arc::new(Mutex::new(String::new()));
    let (finished, report) = (Arc::clone(&done), Arc::clone(&outcome));
    let suggestion = input.with_extension("mp4");
    let arguments = command_line("render", &session.settings);

    std::thread::spawn(move || {
        let chosen = rfd::FileDialog::new()
            .set_title("Export video")
            .set_file_name(suggestion.file_name().and_then(|n| n.to_str()).unwrap_or("opennote.mp4"))
            .add_filter("MP4 video", &["mp4"])
            .save_file();
        if let Some(output) = chosen {
            *report.lock().unwrap() =
                run_self(&arguments, &input, &output, &format!("exported {}", output.display()));
        }
        finished.store(true, Ordering::Release);
    });

    session.task = Some(Task { what: "exporting", done, chosen: Arc::new(Mutex::new(None)), outcome });
}

fn annotate_score(session: &mut Session) {
    let Some(input) = session.path.clone() else {
        return;
    };
    let done = Arc::new(AtomicBool::new(false));
    let outcome = Arc::new(Mutex::new(String::new()));
    let (finished, report) = (Arc::clone(&done), Arc::clone(&outcome));
    let suggestion = input.with_extension("fingered.musicxml");
    let arguments = command_line("annotate", &session.settings);

    std::thread::spawn(move || {
        let chosen = rfd::FileDialog::new()
            .set_title("Save the fingered score")
            .set_file_name(suggestion.file_name().and_then(|n| n.to_str()).unwrap_or("fingered.musicxml"))
            .add_filter("MusicXML", &["musicxml", "mxl"])
            .save_file();
        if let Some(output) = chosen {
            *report.lock().unwrap() =
                run_self(&arguments, &input, &output, &format!("saved {}", output.display()));
        }
        finished.store(true, Ordering::Release);
    });

    session.task = Some(Task { what: "saving", done, chosen: Arc::new(Mutex::new(None)), outcome });
}

/// The `opennote` command line that does what the viewer is showing: the same hand and
/// rules for both, and for a video the same look and sound as well.
fn command_line(subcommand: &str, settings: &SessionSettings) -> Vec<String> {
    let mut arguments = vec![subcommand.to_string()];
    if subcommand == "annotate" {
        // The save dialog has already asked before replacing a file.
        arguments.push("--force".into());
    }
    let fingering = &settings.fingering;
    let mut flag = |name: &str, value: String| arguments.extend([format!("--{name}"), value]);
    flag("hand-length", format!("{}", fingering.profile.hand_length));
    flag("rules", format!("{:?}", fingering.rule_set).to_lowercase());
    if subcommand == "render" {
        flag("lookahead", format!("{}", settings.lookahead));
        flag("mood", settings.mood.name.to_string());
        flag("expression", format!("{}", settings.expression));
        let hex = |colour: Color| {
            let [r, g, b, _] = colour.to_srgba().to_u8_array();
            format!("#{r:02x}{g:02x}{b:02x}")
        };
        flag("left-colour", hex(settings.note_colours[0]));
        flag("right-colour", hex(settings.note_colours[1]));
        if let Some(soundfont) = &settings.soundfont {
            flag("soundfont", soundfont.display().to_string());
        }
    }
    if fingering.ruler == on_fingering::Ruler::Chromatic {
        arguments.push("--chromatic".into());
    }
    if subcommand == "render" && settings.extent == Extent::MusicRange {
        arguments.push("--crop-keyboard".into());
    }
    arguments
}

fn run_self(
    arguments: &[String],
    input: &std::path::Path,
    output: &std::path::Path,
    success: &str,
) -> String {
    let Ok(program) = std::env::current_exe() else {
        return "could not work out where this program lives".into();
    };
    let mut command = std::process::Command::new(program);
    command.args(arguments).arg(input).arg("-o").arg(output);
    match command.status() {
        Ok(status) if status.success() => success.to_string(),
        Ok(status) => format!("that did not work ({status})"),
        Err(error) => format!("could not start: {error}"),
    }
}

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

fn apply_reload(world: &mut World) {
    if !world.resource::<Reload>().wanted {
        if std::mem::take(&mut world.resource_mut::<Reload>().relayout) {
            let lookahead = world.resource::<Session>().settings.lookahead;
            world.resource_mut::<Performance>().layout.lookahead = lookahead.max(0.5);
            let colours = crate::render::NoteColours(world.resource::<Session>().settings.note_colours);
            world.resource_mut::<crate::render::NoteColours>().set_if_neq(colours);
            crate::render::rebuild_scene(world);
        }
        return;
    }
    world.resource_mut::<Reload>().relayout = false;
    let (path, settings) = {
        let reload = world.resource::<Reload>();
        let session = world.resource::<Session>();
        (reload.path.clone().or_else(|| session.path.clone()), session.settings.clone())
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

    let notes = performance.timeline.audio_notes_for(world.resource::<crate::render::Playing>().0);
    {
        let audio = world.resource::<Audio>();
        audio.load(notes);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_the_score_asks_only_for_what_annotate_understands() {
        let mut settings = SessionSettings::new(PathBuf::from("assets"));
        settings.extent = Extent::MusicRange;
        settings.soundfont = Some(PathBuf::from("piano.sf2"));
        let annotate = command_line("annotate", &settings);
        assert_eq!(annotate[0], "annotate");
        for render_only in ["--crop-keyboard", "--soundfont", "--mood", "--lookahead", "--left-colour"] {
            assert!(!annotate.iter().any(|a| a == render_only), "{render_only} in {annotate:?}");
        }
        assert!(annotate.windows(2).any(|w| w[0] == "--rules" && w[1] == "consensus"), "{annotate:?}");

        let render = command_line("render", &settings);
        for wanted in ["--crop-keyboard", "--soundfont", "--hand-length", "--left-colour"] {
            assert!(render.iter().any(|a| a == wanted), "{wanted} missing from {render:?}");
        }
    }
}
