//! An AppImage is one file that registers nothing with the system. The first time OpenNote
//! runs from one, this offers to add it to the applications menu, and to the desktop, and
//! keeps the entry pointing at the file if it is moved.

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy_egui::egui;

const ENTRY: &str = "opennote.desktop";

/// Where the entries go, and what they start.
struct Targets {
    applications: PathBuf,
    icon: PathBuf,
    desktop: Option<PathBuf>,
    state: PathBuf,
    appimage: PathBuf,
    icon_source: PathBuf,
}

#[derive(Debug, PartialEq)]
enum State {
    Unasked,
    Declined,
    Installed { desktop: bool },
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_dir())
}

fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home_dir().map(|home| home.join(".local/share")))
}

/// The user's desktop folder, wherever their language and settings put it.
fn desktop_dir() -> Option<PathBuf> {
    let asked = std::process::Command::new("xdg-user-dir")
        .arg("DESKTOP")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()));
    asked
        .filter(|path| path.is_dir() && Some(path) != home_dir().as_ref())
        .or_else(|| home_dir().map(|home| home.join("Desktop")).filter(|path| path.is_dir()))
}

impl Targets {
    /// Only when running from an AppImage on a system that has an applications menu.
    fn detect() -> Option<Self> {
        let appimage = PathBuf::from(std::env::var_os("APPIMAGE")?);
        let appdir = PathBuf::from(std::env::var_os("APPDIR")?);
        let data = data_home()?;
        Some(Self {
            applications: data.join("applications"),
            icon: data.join("icons").join("opennote.png"),
            desktop: desktop_dir(),
            state: data.join("OpenNote").join("shortcuts"),
            appimage,
            icon_source: appdir.join("opennote.png"),
        })
    }

    /// The text of a `.desktop` entry that starts this AppImage.
    fn entry(&self) -> String {
        // The Exec line is parsed by the desktop: quote the path, and escape what it reads
        // as special inside quotes.
        let path: String = self
            .appimage
            .to_string_lossy()
            .chars()
            .flat_map(|c| match c {
                '"' | '`' | '$' | '\\' => vec!['\\', c],
                '%' => vec!['%', '%'],
                c => vec![c],
            })
            .collect();
        format!(
            "[Desktop Entry]\nType=Application\nName=OpenNote\n\
             Comment=See piano fingerings played by 3D hands\n\
             Exec=\"{path}\" %f\nIcon={}\nCategories=AudioVideo;Audio;Music;Education;\n\
             MimeType=application/vnd.recordare.musicxml+xml;application/vnd.recordare.musicxml;audio/midi;audio/x-midi;\n\
             Terminal=false\n",
            self.icon.display()
        )
    }

    fn state(&self) -> State {
        match std::fs::read_to_string(&self.state).as_deref().map(str::trim) {
            Ok("declined") => State::Declined,
            Ok("installed") => State::Installed { desktop: false },
            Ok("installed+desktop") => State::Installed { desktop: true },
            _ => State::Unasked,
        }
    }

    fn remember(&self, text: &str) {
        if let Some(folder) = self.state.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let _ = std::fs::write(&self.state, text);
    }

    fn decline(&self) {
        self.remember("declined");
    }

    /// Writes the menu entry, and the desktop one if asked.
    fn install(&self, with_desktop: bool) -> std::io::Result<()> {
        if let Some(folder) = self.icon.parent() {
            std::fs::create_dir_all(folder)?;
        }
        // The icon is copied out: the AppImage's own copy is only there while it runs.
        std::fs::copy(&self.icon_source, &self.icon)?;
        std::fs::create_dir_all(&self.applications)?;
        let entry = self.entry();
        write_entry(&self.applications.join(ENTRY), &entry)?;
        if with_desktop {
            if let Some(desktop) = &self.desktop {
                let file = desktop.join(ENTRY);
                write_entry(&file, &entry)?;
                // GNOME shows a desktop entry as a launcher only once it is marked trusted.
                let _ = std::process::Command::new("gio")
                    .args(["set", &file.to_string_lossy(), "metadata::trusted", "true"])
                    .output();
            }
        }
        let _ = std::process::Command::new("update-desktop-database").arg(&self.applications).output();
        self.remember(if with_desktop && self.desktop.is_some() { "installed+desktop" } else { "installed" });
        Ok(())
    }

    /// If the AppImage was moved since, point the entries at where it is now.
    fn refresh(&self, desktop: bool) {
        let entry = self.entry();
        let mut files = vec![self.applications.join(ENTRY)];
        if let (true, Some(folder)) = (desktop, &self.desktop) {
            files.push(folder.join(ENTRY));
        }
        for file in files {
            if file.exists() && std::fs::read_to_string(&file).ok().as_deref() != Some(entry.as_str()) {
                let _ = write_entry(&file, &entry);
            }
        }
        if !self.icon.exists() {
            let _ = std::fs::copy(&self.icon_source, &self.icon);
        }
    }
}

fn write_entry(file: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(file, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

enum Phase {
    Idle,
    Offer { desktop: bool },
    Message(String),
}

/// The question, and what came of it.
#[derive(Resource)]
pub struct Shortcuts {
    phase: std::sync::Mutex<Phase>,
    targets: Option<Targets>,
}

impl Default for Shortcuts {
    fn default() -> Self {
        let targets = Targets::detect();
        let phase = match targets.as_ref().map(|targets| (targets.state(), targets)) {
            Some((State::Unasked, _)) => Phase::Offer { desktop: false },
            Some((State::Installed { desktop }, targets)) => {
                targets.refresh(desktop);
                Phase::Idle
            }
            _ => Phase::Idle,
        };
        Self { phase: std::sync::Mutex::new(phase), targets }
    }
}

/// Draws the question, unless the update popup has the screen.
pub(crate) fn show(ctx: &egui::Context, shortcuts: &Shortcuts, other_popup_open: bool) {
    let Some(targets) = &shortcuts.targets else {
        return;
    };
    let mut phase = shortcuts.phase.lock().unwrap();
    if other_popup_open || matches!(*phase, Phase::Idle) {
        return;
    }
    let mut next = None;
    match &mut *phase {
        Phase::Idle => {}
        Phase::Message(text) => {
            crate::updates::window(ctx).show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(text.as_str());
                ui.add_space(8.0);
                if ui.button("OK").clicked() {
                    next = Some(Phase::Idle);
                }
            });
        }
        Phase::Offer { desktop } => {
            crate::updates::window(ctx).show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(
                    egui::RichText::new("Add OpenNote to your applications?")
                        .font(egui::FontId::new(22.0, crate::theme::display())),
                );
                ui.add_space(6.0);
                ui.label("So you can open it from your applications menu instead of finding this file.");
                if targets.desktop.is_some() {
                    ui.add_space(6.0);
                    ui.checkbox(desktop, "Also put an icon on the desktop");
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new("Add").strong())
                                .min_size(egui::vec2(100.0, 32.0)),
                        )
                        .clicked()
                    {
                        next = Some(match targets.install(*desktop) {
                            Ok(()) => Phase::Message(
                                "Added. You can find OpenNote in your applications menu.".into(),
                            ),
                            Err(error) => Phase::Message(format!("Could not add it: {error}")),
                        });
                    }
                    if crate::theme::quiet(ui, "No thanks").clicked() {
                        targets.decline();
                        next = Some(Phase::Idle);
                    }
                });
            });
        }
    }
    if let Some(next) = next {
        *phase = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(folder: &Path) -> Targets {
        std::fs::write(folder.join("icon.png"), b"png").unwrap();
        Targets {
            applications: folder.join("applications"),
            icon: folder.join("icons/opennote.png"),
            desktop: Some(folder.join("Desktop")),
            state: folder.join("state/shortcuts"),
            appimage: folder.join("My Apps/OpenNote-1.0.0.AppImage"),
            icon_source: folder.join("icon.png"),
        }
    }

    #[test]
    fn the_entry_quotes_a_path_with_spaces() {
        let folder = tempfile::tempdir().unwrap();
        let entry = targets(folder.path()).entry();
        let exec = entry.lines().find(|line| line.starts_with("Exec=")).unwrap();
        assert!(exec.starts_with("Exec=\"") && exec.ends_with("AppImage\" %f"), "{exec}");
        assert!(exec.contains("My Apps"));
    }

    #[test]
    fn installing_writes_the_menu_entry_and_the_desktop_one_only_when_asked() {
        let folder = tempfile::tempdir().unwrap();
        let targets = targets(folder.path());
        std::fs::create_dir_all(folder.path().join("Desktop")).unwrap();
        assert_eq!(targets.state(), State::Unasked);

        targets.install(false).unwrap();
        assert!(folder.path().join("applications/opennote.desktop").exists());
        assert!(!folder.path().join("Desktop/opennote.desktop").exists());
        assert!(targets.icon.exists());
        assert_eq!(targets.state(), State::Installed { desktop: false });

        targets.install(true).unwrap();
        assert!(folder.path().join("Desktop/opennote.desktop").exists());
        assert_eq!(targets.state(), State::Installed { desktop: true });
    }

    #[test]
    fn saying_no_is_remembered() {
        let folder = tempfile::tempdir().unwrap();
        let targets = targets(folder.path());
        targets.decline();
        assert_eq!(targets.state(), State::Declined);
    }

    #[test]
    fn a_moved_appimage_is_followed() {
        let folder = tempfile::tempdir().unwrap();
        let mut targets = targets(folder.path());
        targets.install(false).unwrap();
        targets.appimage = folder.path().join("elsewhere/OpenNote.AppImage");
        targets.refresh(false);
        let text = std::fs::read_to_string(folder.path().join("applications/opennote.desktop")).unwrap();
        assert!(text.contains("elsewhere/OpenNote.AppImage"));
    }
}
