//! The first time OpenNote opens: a short welcome with glover, and how to use it. Shown once,
//! remembered in a small file, and brought back from Help > Welcome.

use std::path::PathBuf;
use std::sync::Mutex;

use bevy::prelude::*;
use bevy_egui::egui;

const GLOVER: &[u8] = include_bytes!("../../../assets/branding/glover.png");

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
    glover: Option<egui::TextureHandle>,
}

#[derive(Resource)]
pub struct Welcome(Mutex<State>);

impl Default for Welcome {
    fn default() -> Self {
        Self(Mutex::new(State { open: false, page: 0, glover: None }))
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

fn glover(ctx: &egui::Context, state: &mut State) -> Option<egui::TextureHandle> {
    if state.glover.is_none() {
        let image = image::load_from_memory(GLOVER).ok()?.to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let colour = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        state.glover = Some(ctx.load_texture("glover", colour, egui::TextureOptions::LINEAR));
    }
    state.glover.clone()
}

pub(crate) fn show(ctx: &egui::Context, welcome: &Welcome) {
    let mut state = welcome.0.lock().unwrap();
    if !state.open {
        return;
    }
    let picture = glover(ctx, &mut state);
    let mut finished = false;
    crate::updates::window(ctx).show(ctx, |ui| {
        ui.set_width(400.0);
        match state.page {
            0 => {
                ui.vertical_centered(|ui| {
                    ui.add_space(4.0);
                    if let Some(picture) = &picture {
                        let width = 190.0;
                        let size = picture.size_vec2();
                        ui.image((picture.id(), egui::vec2(width, width * size.y / size.x)));
                    }
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Welcome!").size(36.0).strong());
                    ui.add_space(6.0);
                    ui.label("OpenNote works out which finger plays each note,");
                    ui.label("and shows it on two 3D hands you can watch play.");
                    ui.add_space(14.0);
                });
            }
            _ => {
                ui.label(egui::RichText::new("How it works").size(24.0).strong());
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
                    let next = egui::Button::new(
                        egui::RichText::new("Next").size(16.0).strong().color(egui::Color32::BLACK),
                    )
                    .fill(egui::Color32::from_rgb(255, 205, 0))
                    .min_size(egui::vec2(110.0, 34.0));
                    if ui.add(next).clicked() {
                        state.page += 1;
                    }
                    if ui.button("Skip").clicked() {
                        finished = true;
                    }
                } else {
                    let go = egui::Button::new(
                        egui::RichText::new("Let's go!").size(16.0).strong().color(egui::Color32::BLACK),
                    )
                    .fill(egui::Color32::from_rgb(255, 205, 0))
                    .min_size(egui::vec2(110.0, 34.0));
                    if ui.add(go).clicked() {
                        finished = true;
                    }
                    if ui.button("Back").clicked() {
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

#[cfg(test)]
mod tests {
    #[test]
    fn glover_is_a_real_picture() {
        let image = image::load_from_memory(super::GLOVER).expect("glover.png decodes");
        assert!(image.width() > 100 && image.height() > 100);
    }
}
