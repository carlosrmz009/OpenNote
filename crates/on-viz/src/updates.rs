//! Telling the pianist that a newer OpenNote or model is available when the viewer opens,
//! and installing it on one click. Where to look and how to install is the host program's
//! business: it registers an [`Updater`], and this module only asks and shows.

use std::sync::{Arc, Mutex, OnceLock};

use bevy::prelude::*;
use bevy_egui::egui;

/// What an accepted update did.
pub enum Outcome {
    /// The update is installed, or its installer is running, and OpenNote should close now.
    Quit,
    /// Nothing more to do but tell the user.
    Message(String),
}

/// An update that can be installed.
pub struct Offer {
    /// One line for each thing that would change, e.g. "OpenNote 0.2.0 (you have 0.1.0)".
    pub changes: Vec<String>,
    /// Downloads and installs everything in `changes`. Runs on its own thread.
    pub install: Box<dyn FnOnce() -> Result<Outcome, String> + Send>,
}

/// Looks for updates.
pub trait Updater: Send + Sync {
    /// Whether anything newer is available. Blocks while it asks, so it runs on its own
    /// thread. An error means it could not find out, which is not worth bothering anyone about.
    fn check(&self) -> Result<Option<Offer>, String>;
}

static UPDATER: OnceLock<Box<dyn Updater>> = OnceLock::new();

/// Lets the viewer check for updates through `updater`. Without one, it never does.
pub fn register(updater: impl Updater + 'static) {
    let _ = UPDATER.set(Box::new(updater));
}

enum Phase {
    Idle,
    Checking,
    Offered(Offer),
    Installing,
    Message(String),
    Quitting,
}

/// Where the update check and install have got to.
#[derive(Resource, Clone)]
pub struct Updates(Arc<Mutex<Phase>>);

impl Default for Updates {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Phase::Idle)))
    }
}

impl Updates {
    /// Asks in the background. With `announce`, also says so when there is nothing new,
    /// which is what someone who asked to check wants to hear.
    pub fn check(&self, announce: bool) {
        let Some(updater) = UPDATER.get() else {
            return;
        };
        {
            let mut phase = self.0.lock().unwrap();
            if !matches!(*phase, Phase::Idle | Phase::Message(_)) {
                return;
            }
            *phase = Phase::Checking;
        }
        let state = Arc::clone(&self.0);
        std::thread::spawn(move || {
            let found = updater.check();
            let mut phase = state.lock().unwrap();
            *phase = match found {
                Ok(Some(offer)) => Phase::Offered(offer),
                Ok(None) if announce => Phase::Message("OpenNote and its model are up to date.".into()),
                Err(error) if announce => Phase::Message(format!("Could not check for updates: {error}")),
                _ => Phase::Idle,
            };
        });
    }
}

impl Updates {
    /// Whether a popup about an update is on screen.
    pub(crate) fn busy(&self) -> bool {
        !matches!(*self.0.lock().unwrap(), Phase::Idle | Phase::Checking)
    }
}

/// Checks once as the viewer opens, unless told not to.
pub(crate) fn check_on_start(updates: Res<Updates>, capture: Option<Res<crate::render::Capture>>) {
    if capture.is_none() && std::env::var_os("OPENNOTE_NO_UPDATE_CHECK").is_none() {
        updates.check(false);
    }
}

/// The yellow button and the popup around it.
pub(crate) fn show(ctx: &egui::Context, updates: &Updates, exit: &mut MessageWriter<AppExit>) {
    let mut phase = updates.0.lock().unwrap();
    let mut next = None;
    match &*phase {
        Phase::Idle | Phase::Checking => {}
        Phase::Quitting => {
            exit.write(AppExit::Success);
        }
        Phase::Installing => {
            window(ctx).show(ctx, |ui| {
                ui.set_width(320.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Downloading and installing the update. OpenNote will restart by itself.");
                });
            });
        }
        Phase::Message(text) => {
            window(ctx).show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label(text.as_str());
                ui.add_space(8.0);
                if ui.button("OK").clicked() {
                    next = Some(Phase::Idle);
                }
            });
        }
        Phase::Offered(offer) => {
            let mut update = false;
            window(ctx).show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(egui::RichText::new("An update is available").size(20.0).strong());
                ui.add_space(6.0);
                for change in &offer.changes {
                    ui.label(format!("•  {change}"));
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let button = egui::Button::new(
                        egui::RichText::new("Update!").size(18.0).strong().color(egui::Color32::BLACK),
                    )
                    .fill(egui::Color32::from_rgb(255, 205, 0))
                    .min_size(egui::vec2(140.0, 38.0));
                    update = ui.add(button).clicked();
                    if ui.button("Not now").clicked() {
                        next = Some(Phase::Idle);
                    }
                });
            });
            if update {
                next = Some(Phase::Installing);
            }
        }
    }
    let Some(next) = next else {
        return;
    };
    let previous = std::mem::replace(&mut *phase, next);
    if let (Phase::Offered(offer), Phase::Installing) = (previous, &*phase) {
        let state = Arc::clone(&updates.0);
        std::thread::spawn(move || {
            let result = (offer.install)();
            *state.lock().unwrap() = match result {
                Ok(Outcome::Quit) => Phase::Quitting,
                Ok(Outcome::Message(text)) => Phase::Message(text),
                Err(error) => Phase::Message(format!("The update did not install: {error}")),
            };
        });
    }
}

pub(crate) fn window(ctx: &egui::Context) -> egui::Window<'static> {
    // Dims everything behind it, so the popup is the one thing to answer.
    let screen = ctx.viewport_rect();
    ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("update-dim"))).rect_filled(
        screen,
        0.0,
        egui::Color32::from_black_alpha(140),
    );
    egui::Window::new("Update")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
}
