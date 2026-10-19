//! The first time OpenNote opens: a short welcome with glover, and how to use it. Shown once,
//! remembered in a small file, and brought back from Help > Welcome.

use std::path::PathBuf;
use std::sync::Mutex;

use bevy::prelude::*;
use bevy_egui::egui;

/// Where OpenNote keeps what it remembers between runs.
pub(crate) fn data_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
    };
    base.map(|base| base.join("OpenNote"))
}

fn marker() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("welcomed"))
}

const PAGES: usize = 2;

struct State {
    open: bool,
    page: usize,
}

#[derive(Resource)]
pub struct Welcome(Mutex<State>);

impl Default for Welcome {
    fn default() -> Self {
        Self(Mutex::new(State { open: false, page: 0 }))
    }
}

impl Welcome {
    /// Whether the welcome is on screen, which holds back the other popups.
    pub(crate) fn open(&self) -> bool {
        self.0.lock().unwrap().open
    }

    /// Shows it again, from the start.
    pub(crate) fn reopen(&self) {
        let mut state = self.0.lock().unwrap();
        state.open = true;
        state.page = 0;
    }
}

/// Opens the welcome on the first run, unless this is a screenshot or video.
pub(crate) fn first_run(welcome: Res<Welcome>, capture: Option<Res<crate::render::Capture>>) {
    let forced = std::env::var_os("OPENNOTE_WELCOME").is_some();
    if capture.is_some() && !forced {
        return;
    }
    let seen = marker().is_some_and(|marker| marker.exists());
    if forced || !seen {
        welcome.reopen();
        // OPENNOTE_WELCOME=2 starts on the second page, to look at it.
        if let Some(page) = std::env::var("OPENNOTE_WELCOME").ok().and_then(|page| page.parse::<usize>().ok())
        {
            welcome.0.lock().unwrap().page = page.saturating_sub(1).min(PAGES - 1);
        }
    }
}

fn finish(state: &mut State) {
    state.open = false;
    if let Some(marker) = marker() {
        if let Some(folder) = marker.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let _ = std::fs::write(marker, "yes");
    }
}

pub(crate) fn show(ctx: &egui::Context, welcome: &Welcome) {
    let mut state = welcome.0.lock().unwrap();
    if !state.open {
        return;
    }
    let mut finished = false;
    crate::updates::window(ctx).show(ctx, |ui| {
        ui.set_width(400.0);
        match state.page {
            0 => {
                ui.vertical_centered(|ui| {
                    ui.add_space(4.0);
                    crate::theme::show_glover(ui, 190.0);
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Welcome!").font(egui::FontId::new(40.0, crate::theme::display())));
                    ui.add_space(6.0);
                    ui.label("OpenNote works out which finger plays each note,\nand shows it on two 3D hands you can watch play.");
                    ui.add_space(14.0);
                });
            }
            _ => {
                ui.label(egui::RichText::new("How it works").font(egui::FontId::new(26.0, crate::theme::display())));
                ui.add_space(8.0);
                for (number, text) in [
                    ("1", "Press Select piece… and choose a score (MusicXML or MIDI)."),
                    ("2", "Watch the hands. The numbers are the fingers: 1 is the thumb, 5 the little finger."),
                    ("3", "Click the bar along the bottom to jump anywhere. Set A and B to practise one passage."),
                    ("4", "Leave one hand to yourself from Playback > Hands played. Press F1 for every shortcut."),
                ] {
                    ui.horizontal_top(|ui| {
                        ui.label(egui::RichText::new(number).strong().size(16.0));
                        ui.add(egui::Label::new(text).wrap());
                    });
                    ui.add_space(4.0);
                }
                ui.add_space(8.0);
            }
        }

        ui.horizontal(|ui| {
            for page in 0..PAGES {
                let colour = if page == state.page { ui.visuals().text_color() } else { ui.visuals().weak_text_color() };
                let (area, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                ui.painter().circle_filled(area.center(), 4.0, colour);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if state.page + 1 < PAGES {
                    if crate::theme::primary(ui, "Next", egui::vec2(120.0, 40.0)).clicked() {
                        state.page += 1;
                    }
                    if crate::theme::quiet(ui, "Skip").clicked() {
                        finished = true;
                    }
                } else {
                    if crate::theme::primary(ui, "Let's go!", egui::vec2(120.0, 40.0)).clicked() {
                        finished = true;
                    }
                    if crate::theme::quiet(ui, "Back").clicked() {
                        state.page -= 1;
                    }
                }
            });
        });
    });
    if finished {
        finish(&mut state);
    }
}
