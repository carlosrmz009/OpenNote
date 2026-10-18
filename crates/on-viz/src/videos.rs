//! Making a video of the piece, in one dialog: choose what to film, get the helper program
//! that encodes it if it is missing, watch the progress, and open the result.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy_egui::egui;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Quality {
    Sharp,
    Quick,
}

impl Quality {
    fn label(self) -> &'static str {
        match self {
            Self::Sharp => "Sharp and smooth: 1080p, 60 frames a second",
            Self::Quick => "Quick and small: 720p, 30 frames a second",
        }
    }

    /// width, height, frames a second, and x264's quality number.
    fn numbers(self) -> (u32, u32, u32, u8) {
        match self {
            Self::Sharp => (1920, 1080, 60, 18),
            Self::Quick => (1280, 720, 30, 23),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Whole,
    Section,
}

/// What the person has chosen so far.
struct Setup {
    base: Vec<String>,
    input: PathBuf,
    title: String,
    section: Option<(f64, f64)>,
    part: Part,
    quality: Quality,
    intro: bool,
    speed: f64,
    output: PathBuf,
    picking: Option<Arc<Mutex<Option<Option<PathBuf>>>>>,
}

impl Setup {
    /// The `opennote render` command line for these choices.
    fn arguments(&self) -> Vec<String> {
        let (width, height, fps, crf) = self.quality.numbers();
        let mut arguments = self.base.clone();
        for (flag, value) in [
            ("--width", width.to_string()),
            ("--height", height.to_string()),
            ("--fps", fps.to_string()),
            ("--crf", crf.to_string()),
            ("--speed", format!("{:.3}", self.speed)),
        ] {
            arguments.extend([flag.to_string(), value]);
        }
        if !self.intro {
            arguments.push("--no-intro".into());
        }
        if let (Part::Section, Some((from, to))) = (self.part, self.section) {
            let to = if to.is_finite() { format!("{to:.3}") } else { String::new() };
            arguments.extend(["--from".to_string(), format!("{from:.3}")]);
            if !to.is_empty() {
                arguments.extend(["--to".to_string(), to]);
            }
        }
        arguments.push(self.input.display().to_string());
        arguments.extend(["-o".to_string(), self.output.display().to_string()]);
        arguments
    }
}

/// How far along the encoding is, and how it ended.
#[derive(Default)]
struct Run {
    done: u32,
    total: u32,
    result: Option<Result<(), String>>,
}

enum Phase {
    Closed,
    Setup(Setup),
    /// Looking for ffmpeg.
    Checking(Box<Setup>, Arc<Mutex<Option<String>>>),
    /// There is none; it can be downloaded.
    Offer(Box<Setup>),
    Downloading(Box<Setup>, Arc<Mutex<Option<Result<(), String>>>>),
    /// There is none, and it has to be installed by hand.
    ByHand,
    Making {
        run: Arc<Mutex<Run>>,
        cancel: Arc<AtomicBool>,
        output: PathBuf,
    },
    Done(PathBuf),
    Failed(String),
}

/// The video dialog.
#[derive(Resource)]
pub struct Videos(Mutex<Phase>);

impl Default for Videos {
    fn default() -> Self {
        Self(Mutex::new(Phase::Closed))
    }
}

impl Videos {
    /// Starts the dialog for the piece that is open. `base` is the `render` command line
    /// that makes the video look and sound as the viewer does.
    pub(crate) fn start(
        &self,
        base: Vec<String>,
        input: PathBuf,
        title: String,
        section: Option<(f64, f64)>,
        speed: f64,
    ) {
        let output = unused(&input.with_extension("mp4"));
        *self.0.lock().unwrap() = Phase::Setup(Setup {
            base,
            input,
            title,
            section,
            part: if section.is_some() { Part::Section } else { Part::Whole },
            quality: Quality::Sharp,
            intro: true,
            speed,
            output,
            picking: None,
        });
    }
}

/// A file name that is not taken: `name.mp4`, then `name (2).mp4`, and so on.
fn unused(wanted: &Path) -> PathBuf {
    if !wanted.exists() {
        return wanted.to_path_buf();
    }
    let stem = wanted.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
    let extension = wanted.extension().and_then(|s| s.to_str()).unwrap_or("mp4");
    (2..1000)
        .map(|n| wanted.with_file_name(format!("{stem} ({n}).{extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| wanted.to_path_buf())
}

fn own_program() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// The last few lines of what a program said, for telling someone what went wrong.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(6)..].join("\n")
}

/// Reads `  frame 120 of 3600`, which the renderer prints as it goes.
fn parse_progress(line: &str) -> Option<(u32, u32)> {
    let mut words = line.split_whitespace();
    (words.next()? == "frame").then_some(())?;
    let done = words.next()?.parse().ok()?;
    (words.next()? == "of").then_some(())?;
    Some((done, words.next()?.parse().ok()?))
}

fn check_for_ffmpeg() -> Arc<Mutex<Option<String>>> {
    let answer = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&answer);
    std::thread::spawn(move || {
        let said = own_program()
            .and_then(|program| Command::new(program).args(["ffmpeg", "check"]).output().ok())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_default();
        *slot.lock().unwrap() = Some(said);
    });
    answer
}

fn download_ffmpeg() -> Arc<Mutex<Option<Result<(), String>>>> {
    let answer = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&answer);
    std::thread::spawn(move || {
        let result =
            match own_program().map(|program| Command::new(program).args(["ffmpeg", "install"]).output()) {
                Some(Ok(output)) if output.status.success() => Ok(()),
                Some(Ok(output)) => Err(tail(&String::from_utf8_lossy(&output.stderr))),
                Some(Err(error)) => Err(error.to_string()),
                None => Err("could not work out where OpenNote is".to_string()),
            };
        *slot.lock().unwrap() = Some(result);
    });
    answer
}

/// Runs `opennote render` and keeps `run` up to date with how far it has got.
fn make_video(arguments: Vec<String>, run: Arc<Mutex<Run>>, cancel: Arc<AtomicBool>, output: PathBuf) {
    std::thread::spawn(move || {
        let outcome = (|| -> Result<(), String> {
            let program = own_program().ok_or("could not work out where OpenNote is")?;
            let mut child = Command::new(program)
                .args(&arguments)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| format!("could not start the video maker: {error}"))?;
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let progress = Arc::clone(&run);
            let reader = std::thread::spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(stdout?).lines().map_while(Result::ok) {
                    if let Some((done, total)) = parse_progress(&line) {
                        let mut run = progress.lock().unwrap();
                        run.done = done;
                        run.total = total;
                    }
                }
                Some(())
            });
            let errors = std::thread::spawn(move || {
                use std::io::Read;
                let mut text = String::new();
                if let Some(mut stderr) = stderr {
                    let _ = stderr.read_to_string(&mut text);
                }
                text
            });
            let status = loop {
                if cancel.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    let _ = errors.join();
                    return Err("cancelled".to_string());
                }
                match child.try_wait() {
                    Ok(Some(status)) => break status,
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
                    Err(error) => return Err(error.to_string()),
                }
            };
            let _ = reader.join();
            let text = errors.join().unwrap_or_default();
            if status.success() {
                Ok(())
            } else {
                Err(if text.trim().is_empty() {
                    format!("the video maker stopped ({status})")
                } else {
                    tail(&text)
                })
            }
        })();
        if outcome.is_err() {
            // A half-made file is no use to anyone.
            let _ = std::fs::remove_file(&output);
        }
        run.lock().unwrap().result = Some(outcome);
    });
}

/// Shows a file or folder in the system's file manager.
fn show_in_folder(path: &Path) {
    let _ = if cfg!(windows) {
        Command::new("explorer").arg(format!("/select,{}", path.display())).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg("-R").arg(path).spawn()
    } else {
        Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn()
    };
}

/// Opens a file with whatever the system plays it with.
fn open_file(path: &Path) {
    let _ = if cfg!(windows) {
        Command::new("cmd").args(["/C", "start", ""]).arg(path).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(path).spawn()
    } else {
        Command::new("xdg-open").arg(path).spawn()
    };
}

fn clock(seconds: f64) -> String {
    let whole = seconds.max(0.0) as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

fn primary(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).strong().size(16.0).color(egui::Color32::BLACK))
            .fill(egui::Color32::from_rgb(255, 205, 0))
            .min_size(egui::vec2(150.0, 38.0)),
    )
}

/// Draws whichever step the video dialog is on, and moves on when a step finishes.
pub(crate) fn show(ctx: &egui::Context, videos: &Videos) {
    let mut guard = videos.0.lock().unwrap();
    if matches!(*guard, Phase::Closed) {
        return;
    }
    let mut pressed: Option<Press> = None;
    crate::updates::window(ctx).show(ctx, |ui| {
        ui.set_width(430.0);
        match &mut *guard {
            Phase::Closed => {}
            Phase::Setup(setup) => {
                let chosen = setup.picking.as_ref().and_then(|slot| slot.lock().unwrap().take());
                if let Some(chosen) = chosen {
                    if let Some(path) = chosen {
                        setup.output = path;
                    }
                    setup.picking = None;
                }
                ui.label(egui::RichText::new("Make a video").size(24.0).strong());
                ui.label(egui::RichText::new(&setup.title).weak());
                ui.add_space(10.0);

                ui.label("What to film");
                ui.radio_value(&mut setup.part, Part::Whole, "The whole piece");
                let section_label = match setup.section {
                    Some((from, to)) => {
                        let to = if to.is_finite() { clock(to) } else { "the end".into() };
                        format!("Only my practice section ({} to {to})", clock(from.max(0.0)))
                    }
                    None => "Only my practice section (set A and B on the progress bar first)".to_string(),
                };
                ui.add_enabled_ui(setup.section.is_some(), |ui| {
                    ui.radio_value(&mut setup.part, Part::Section, section_label);
                });
                ui.add_space(8.0);

                ui.label("Quality");
                for quality in [Quality::Sharp, Quality::Quick] {
                    ui.radio_value(&mut setup.quality, quality, quality.label());
                }
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label("Speed");
                    ui.add(
                        egui::DragValue::new(&mut setup.speed)
                            .range(0.25..=2.0)
                            .speed(0.01)
                            .max_decimals(2)
                            .suffix("×"),
                    );
                    ui.weak("slower is good for a practice video");
                });
                ui.checkbox(&mut setup.intro, "Start with a \"Made with OpenNote\" card");
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label("Save as");
                    let name = setup.output.file_name().and_then(|n| n.to_str()).unwrap_or("video.mp4");
                    ui.label(egui::RichText::new(name).strong());
                    if setup.picking.is_none() && ui.button("Choose…").clicked() {
                        let slot = Arc::new(Mutex::new(None));
                        setup.picking = Some(Arc::clone(&slot));
                        let start = setup.output.clone();
                        std::thread::spawn(move || {
                            let mut dialog = rfd::FileDialog::new()
                                .set_title("Save the video")
                                .add_filter("MP4 video", &["mp4"]);
                            if let Some(folder) = start.parent() {
                                dialog = dialog.set_directory(folder);
                            }
                            if let Some(name) = start.file_name().and_then(|n| n.to_str()) {
                                dialog = dialog.set_file_name(name);
                            }
                            *slot.lock().unwrap() = Some(dialog.save_file());
                        });
                    }
                });
                if let Some(folder) = setup.output.parent() {
                    ui.label(egui::RichText::new(folder.display().to_string()).small().weak());
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Make video").clicked() {
                        pressed = Some(Press::Go);
                    }
                    if ui.button("Cancel").clicked() {
                        pressed = Some(Press::Cancel);
                    }
                });
            }
            Phase::Checking(_, answer) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Getting ready…");
                });
                let _ = answer;
            }
            Phase::Offer(_) => {
                ui.label(egui::RichText::new("One thing first").size(22.0).strong());
                ui.add_space(6.0);
                ui.label("Videos need a free helper called ffmpeg, and this computer does not have it yet.");
                ui.label(
                    "OpenNote can download it now (about 80 MB, once). It is kept in OpenNote's own folder.",
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Download it").clicked() {
                        pressed = Some(Press::Go);
                    }
                    if ui.button("Cancel").clicked() {
                        pressed = Some(Press::Cancel);
                    }
                });
            }
            Phase::Downloading(..) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Downloading the video helper…");
                });
            }
            Phase::ByHand => {
                ui.label(egui::RichText::new("One thing first").size(22.0).strong());
                ui.add_space(6.0);
                ui.label("Videos need a free helper called ffmpeg, and this computer does not have it yet.");
                if cfg!(target_os = "macos") {
                    ui.label("The easiest way is Homebrew: open Terminal and run  brew install ffmpeg");
                    ui.label("Then try again. (Homebrew is at brew.sh.)");
                } else {
                    ui.label("Install ffmpeg with your system's package manager, then try again.");
                }
                ui.add_space(12.0);
                if ui.button("Close").clicked() {
                    pressed = Some(Press::Cancel);
                }
            }
            Phase::Making { run, cancel, .. } => {
                let (done, total) = {
                    let run = run.lock().unwrap();
                    (run.done, run.total)
                };
                ui.label(egui::RichText::new("Making your video").size(22.0).strong());
                ui.add_space(8.0);
                let fraction = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                ui.add(
                    egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                        .desired_width(ui.available_width())
                        .show_percentage(),
                );
                ui.label(
                    egui::RichText::new(if total > 0 {
                        format!("Frame {done} of {total}")
                    } else {
                        "Starting…".into()
                    })
                    .weak(),
                );
                ui.add_space(10.0);
                if ui.button("Cancel").clicked() {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            Phase::Done(path) => {
                ui.label(egui::RichText::new("Your video is ready").size(24.0).strong());
                ui.label(egui::RichText::new(path.display().to_string()).small().weak());
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if primary(ui, "Play it").clicked() {
                        open_file(path);
                    }
                    if ui.button("Show in folder").clicked() {
                        show_in_folder(path);
                    }
                    if ui.button("Done").clicked() {
                        pressed = Some(Press::Cancel);
                    }
                });
            }
            Phase::Failed(message) => {
                ui.label(egui::RichText::new("The video could not be made").size(22.0).strong());
                ui.add_space(6.0);
                ui.label(egui::RichText::new(message.as_str()).weak());
                ui.add_space(12.0);
                if ui.button("Close").clicked() {
                    pressed = Some(Press::Cancel);
                }
            }
        }
    });

    advance(&mut guard, pressed);
}

/// What the person pressed.
enum Press {
    /// The main button of the step: go on.
    Go,
    /// Cancel, close or done.
    Cancel,
}

/// Moves to the next step: from what was pressed, or from a background job that finished.
fn advance(phase: &mut Phase, pressed: Option<Press>) {
    match pressed {
        Some(Press::Cancel) => {
            *phase = Phase::Closed;
            return;
        }
        Some(Press::Go) => {
            let current = std::mem::replace(phase, Phase::Closed);
            *phase = match current {
                Phase::Setup(setup) => Phase::Checking(Box::new(setup), check_for_ffmpeg()),
                Phase::Offer(setup) => Phase::Downloading(setup, download_ffmpeg()),
                other => other,
            };
            return;
        }
        None => {}
    }
    let current = std::mem::replace(phase, Phase::Closed);
    *phase = match current {
        Phase::Checking(setup, answer) => {
            let said = answer.lock().unwrap().clone();
            match said.as_deref() {
                None => Phase::Checking(setup, answer),
                Some(said) if said.starts_with("found") => begin(*setup),
                Some("can-download") => Phase::Offer(setup),
                Some(_) => Phase::ByHand,
            }
        }
        Phase::Downloading(setup, answer) => {
            let finished = answer.lock().unwrap().take();
            match finished {
                None => Phase::Downloading(setup, answer),
                Some(Ok(())) => begin(*setup),
                Some(Err(message)) => Phase::Failed(format!("Could not download ffmpeg.\n{message}")),
            }
        }
        Phase::Making { run, cancel, output } => {
            let result = run.lock().unwrap().result.clone();
            match result {
                None => Phase::Making { run, cancel, output },
                Some(Ok(())) => Phase::Done(output),
                Some(Err(message)) if message == "cancelled" => Phase::Closed,
                Some(Err(message)) => Phase::Failed(message),
            }
        }
        other => other,
    };
}

fn begin(setup: Setup) -> Phase {
    let run = Arc::new(Mutex::new(Run::default()));
    let cancel = Arc::new(AtomicBool::new(false));
    make_video(setup.arguments(), Arc::clone(&run), Arc::clone(&cancel), setup.output.clone());
    Phase::Making { run, cancel, output: setup.output }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Setup {
        Setup {
            base: vec!["render".into(), "--rules".into(), "consensus".into()],
            input: PathBuf::from("/music/fur-elise.musicxml"),
            title: "Fur Elise".into(),
            section: Some((12.0, 30.0)),
            part: Part::Whole,
            quality: Quality::Sharp,
            intro: true,
            speed: 1.0,
            output: PathBuf::from("/music/fur-elise.mp4"),
            picking: None,
        }
    }

    #[test]
    fn the_command_line_carries_every_choice() {
        let mut setup = setup();
        let sharp = setup.arguments().join(" ");
        assert!(sharp.contains("--width 1920 --height 1080 --fps 60"), "{sharp}");
        assert!(!sharp.contains("--no-intro") && !sharp.contains("--from"));
        assert!(sharp.ends_with("/music/fur-elise.musicxml -o /music/fur-elise.mp4"));

        setup.quality = Quality::Quick;
        setup.intro = false;
        setup.part = Part::Section;
        setup.speed = 0.5;
        let quick = setup.arguments().join(" ");
        assert!(quick.contains("--width 1280 --height 720 --fps 30"), "{quick}");
        assert!(quick.contains("--no-intro") && quick.contains("--speed 0.500"));
        assert!(quick.contains("--from 12.000 --to 30.000"), "{quick}");
    }

    #[test]
    fn a_section_with_no_end_runs_to_the_end_of_the_piece() {
        let mut setup = setup();
        setup.part = Part::Section;
        setup.section = Some((5.0, f64::INFINITY));
        let arguments = setup.arguments().join(" ");
        assert!(arguments.contains("--from 5.000") && !arguments.contains("--to"));
    }

    #[test]
    fn progress_lines_are_read_and_other_lines_are_not() {
        assert_eq!(parse_progress("  frame 120 of 3600"), Some((120, 3600)));
        assert_eq!(parse_progress("Rendering 95 frames at 480x270, 10 fps."), None);
        assert_eq!(parse_progress("Wrote /tmp/x.mp4"), None);
    }

    #[test]
    fn a_taken_file_name_is_not_overwritten() {
        let folder = tempfile::tempdir().unwrap();
        let wanted = folder.path().join("piece.mp4");
        assert_eq!(unused(&wanted), wanted);
        std::fs::write(&wanted, b"x").unwrap();
        assert_eq!(unused(&wanted), folder.path().join("piece (2).mp4"));
        std::fs::write(folder.path().join("piece (2).mp4"), b"x").unwrap();
        assert_eq!(unused(&wanted), folder.path().join("piece (3).mp4"));
    }

    #[test]
    fn only_the_last_lines_of_an_error_are_shown() {
        let text = (1..=20).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
        let shown = tail(&text);
        assert!(shown.starts_with("line 15") && shown.ends_with("line 20"));
    }
}
