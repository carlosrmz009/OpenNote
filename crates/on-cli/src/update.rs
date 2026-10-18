//! Looking for a newer OpenNote or trained model on GitHub, and installing it: what the
//! viewer's "Update!" button does.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, ensure, Context, Result};
use on_viz::updates::{Offer, Outcome, Updater};
use sha2::{Digest, Sha256};

use crate::models;

const REPO: &str = "carlosrmz009/OpenNote";

/// Checks the GitHub releases of this project.
pub struct GitHub;

impl Updater for GitHub {
    fn check(&self) -> Result<Option<Offer>, String> {
        look().map_err(|error| format!("{error:#}"))
    }
}

/// The model to finger with when none is asked for: one downloaded by an update, if it is
/// newer than the one built in, and otherwise the built-in one.
pub fn preferred_model() -> Result<Option<String>> {
    let built = crate::built_in()?;
    let built_name = env!("OPENNOTE_MODEL_NAME");
    Ok(match models::active() {
        Some((name, text)) if built.is_none() || version_of(&name) > version_of(built_name) => Some(text),
        _ => built,
    })
}

/// The name of the model this copy fingers with, if it has one.
fn current_model() -> Option<String> {
    let built = env!("OPENNOTE_MODEL_NAME");
    match models::active() {
        Some((name, _)) if built.is_empty() || version_of(&name) > version_of(built) => Some(name),
        _ if built.is_empty() => None,
        _ => Some(built.to_string()),
    }
}

/// The numbers in a version or model name, in order: `helios-beta1.10` is `[1, 10]`.
fn version_of(text: &str) -> Vec<u64> {
    let mut numbers = Vec::new();
    let mut digits = String::new();
    for c in text.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            digits.push(c);
        } else if !digits.is_empty() {
            numbers.push(digits.parse().unwrap_or(0));
            digits.clear();
        }
    }
    numbers
}

/// What comes before the first number: `helios-beta` for `helios-beta1.8`. Models of one
/// family can be compared; others are different models, not newer ones.
fn family_of(name: &str) -> &str {
    name.split(|c: char| c.is_ascii_digit()).next().unwrap_or(name)
}

fn is_newer(remote: &str, local: &str) -> bool {
    version_of(remote) > version_of(local)
}

struct AppUpdate {
    version: String,
    page: String,
    file: String,
    url: String,
    sums_url: String,
}

/// The name a release's installer for this system ends with.
fn installer_suffix() -> Option<&'static str> {
    if cfg!(windows) {
        Some("-windows-setup.exe")
    } else if cfg!(target_os = "macos") {
        Some(if cfg!(target_arch = "aarch64") { "-macos-arm64.dmg" } else { "-macos-intel.dmg" })
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        Some("-linux-x86_64.AppImage")
    } else {
        None
    }
}

fn newest_release(releases: &serde_json::Value, local: &str) -> Option<AppUpdate> {
    let suffix = installer_suffix()?;
    let mut best: Option<(Vec<u64>, &serde_json::Value)> = None;
    for release in releases.as_array()? {
        let tag = release["tag_name"].as_str()?;
        // The release holding the models is not a version of the program.
        if release["draft"] == true || release["prerelease"] == true || !tag.starts_with('v') {
            continue;
        }
        let number = version_of(tag);
        if !number.is_empty() && best.as_ref().is_none_or(|(highest, _)| number > *highest) {
            best = Some((number, release));
        }
    }
    let (_, release) = best?;
    let version = release["tag_name"].as_str()?.trim_start_matches('v').to_string();
    if !is_newer(&version, local) {
        return None;
    }
    let asset = |wanted: &dyn Fn(&str) -> bool| {
        release["assets"].as_array()?.iter().find_map(|asset| {
            let name = asset["name"].as_str()?;
            let url = asset["browser_download_url"].as_str()?;
            wanted(name).then(|| (name.to_string(), url.to_string()))
        })
    };
    // A release whose installers are still being built has nothing to offer yet.
    let (file, url) = asset(&|name| name.ends_with(suffix))?;
    let (_, sums_url) = asset(&|name| name == "SHA256SUMS.txt")?;
    Some(AppUpdate {
        version,
        page: release["html_url"]
            .as_str()
            .unwrap_or("https://github.com/carlosrmz009/OpenNote/releases")
            .into(),
        file,
        url,
        sums_url,
    })
}

fn newest_model() -> Result<Option<String>> {
    let Some(current) = current_model() else {
        return Ok(None);
    };
    let family = family_of(&current).to_string();
    let newest = models::published_names()?
        .into_iter()
        .filter(|name| family_of(name) == family && is_newer(name, &current))
        .max_by_key(|name| version_of(name));
    Ok(newest)
}

fn look() -> Result<Option<Offer>> {
    let local = env!("CARGO_PKG_VERSION");
    let body = models::curl(&[&format!("https://api.github.com/repos/{REPO}/releases?per_page=30")])?;
    let releases: serde_json::Value =
        serde_json::from_slice(&body).context("reading the list of releases")?;
    let app = newest_release(&releases, local);
    // A build that cannot open published models has no use for a newer one.
    let model = if models::can_open_published() { newest_model().ok().flatten() } else { None };
    if app.is_none() && model.is_none() {
        return Ok(None);
    }

    let mut changes = Vec::new();
    if let Some(app) = &app {
        changes.push(format!("OpenNote {} (you have {local})", app.version));
    }
    if let (Some(model), Some(current)) = (&model, current_model()) {
        changes.push(format!("The model {model} (you have {current})"));
    }
    Ok(Some(Offer {
        changes,
        install: Box::new(move || install(app, model).map_err(|error| format!("{error:#}"))),
    }))
}

fn install(app: Option<AppUpdate>, model: Option<String>) -> Result<Outcome> {
    if let Some(name) = &model {
        models::fetch(name)?;
        models::make_active(name)?;
    }
    match app {
        Some(app) => install_app(&app),
        None => {
            relaunch()?;
            Ok(Outcome::Quit)
        }
    }
}

fn install_app(app: &AppUpdate) -> Result<Outcome> {
    let folder = std::env::temp_dir().join("opennote-update");
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder)?;
    let file = folder.join(&app.file);

    models::curl_for(1800, &["-o", &file.to_string_lossy(), &app.url]).context("downloading the update")?;
    let sums = models::curl(&[&app.sums_url]).context("downloading its checksums")?;
    verify(&file, &String::from_utf8_lossy(&sums), &app.file)?;

    if cfg!(windows) {
        // The installer closes this copy, replaces it and starts the new one.
        Command::new(&file)
            .args(["/SILENT", "/NORESTART", "/SUPPRESSMSGBOXES"])
            .spawn()
            .context("starting the installer")?;
        return Ok(Outcome::Quit);
    }
    let replaced =
        if cfg!(target_os = "macos") { replace_app_bundle(&file, &folder) } else { replace_appimage(&file) };
    match replaced {
        Ok(()) => {
            relaunch()?;
            Ok(Outcome::Quit)
        }
        Err(error) => {
            // Not an installed copy (a build from source, say), or one this user cannot write
            // to: leave the download where they can fetch it themselves.
            open_in_browser(&app.page);
            Ok(Outcome::Message(format!(
                "OpenNote could not replace itself here ({error:#}), so the download page has been opened."
            )))
        }
    }
}

fn verify(file: &Path, sums: &str, name: &str) -> Result<()> {
    let wanted = sums
        .lines()
        .filter_map(|line| line.split_once(char::is_whitespace))
        .find(|(_, listed)| listed.trim().trim_start_matches('*') == name)
        .map(|(hash, _)| hash.to_ascii_lowercase())
        .with_context(|| format!("no checksum is published for {name}"))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(file)?, &mut hasher)?;
    let found: String = hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
    ensure!(
        found == wanted,
        "the download is damaged (its checksum does not match), so it was not installed"
    );
    Ok(())
}

/// Replaces the AppImage that is running with the new one, keeping its name.
fn replace_appimage(new: &Path) -> Result<()> {
    let Some(target) = std::env::var_os("APPIMAGE").map(PathBuf::from) else {
        bail!("this is not the AppImage");
    };
    let staged = PathBuf::from(format!("{}.update", target.display()));
    std::fs::copy(new, &staged).with_context(|| format!("writing next to {}", target.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&staged, &target).with_context(|| format!("replacing {}", target.display()))
}

/// Copies OpenNote.app out of the disk image over the one that is running.
fn replace_app_bundle(dmg: &Path, folder: &Path) -> Result<()> {
    let exe = std::env::current_exe()?;
    let Some(bundle) = exe.ancestors().find(|path| path.extension().is_some_and(|ext| ext == "app")) else {
        bail!("this is not the installed app");
    };
    let mount = folder.join("mount");
    std::fs::create_dir_all(&mount)?;
    run(
        "hdiutil",
        &[
            "attach",
            "-nobrowse",
            "-readonly",
            "-mountpoint",
            &mount.to_string_lossy(),
            &dmg.to_string_lossy(),
        ],
    )?;
    let copied = (|| {
        let staged = bundle.with_extension("app.update");
        let _ = std::fs::remove_dir_all(&staged);
        run("ditto", &[&mount.join("OpenNote.app").to_string_lossy(), &staged.to_string_lossy()])?;
        std::fs::remove_dir_all(bundle).context("removing the old version")?;
        std::fs::rename(&staged, bundle).context("putting the new version in place")
    })();
    let _ = run("hdiutil", &["detach", "-quiet", &mount.to_string_lossy()]);
    copied
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program).args(args).output().with_context(|| format!("running {program}"))?;
    ensure!(output.status.success(), "{program}: {}", String::from_utf8_lossy(&output.stderr).trim());
    Ok(())
}

/// Starts OpenNote again, as it is installed now.
fn relaunch() -> Result<()> {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        Command::new(appimage).spawn().context("starting OpenNote again")?;
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    if cfg!(target_os = "macos") {
        if let Some(bundle) = exe.ancestors().find(|path| path.extension().is_some_and(|ext| ext == "app")) {
            return run("open", &["-n", &bundle.to_string_lossy()]);
        }
    }
    Command::new(exe).spawn().context("starting OpenNote again")?;
    Ok(())
}

fn open_in_browser(url: &str) {
    let _ = if cfg!(windows) {
        Command::new("cmd").args(["/C", "start", "", url]).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert!(is_newer("0.10.0", "0.9.3"));
        assert!(is_newer("v1.0.0".trim_start_matches('v'), "0.9.9"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(is_newer("helios-beta1.10", "helios-beta1.8"));
    }

    #[test]
    fn only_models_of_one_family_are_compared() {
        assert_eq!(family_of("helios-beta1.8"), "helios-beta");
        assert_ne!(family_of("apollo2.0"), family_of("helios-beta1.8"));
    }

    fn releases() -> serde_json::Value {
        let suffix = installer_suffix().unwrap_or("-linux-x86_64.AppImage");
        serde_json::json!([
            {"tag_name": "models", "draft": false, "prerelease": false, "assets": []},
            {"tag_name": "v0.3.0", "draft": false, "prerelease": false, "html_url": "page", "assets": [
                {"name": format!("OpenNote-0.3.0{suffix}"), "browser_download_url": "https://x/installer"},
                {"name": "SHA256SUMS.txt", "browser_download_url": "https://x/sums"}]},
            {"tag_name": "v0.2.0", "draft": false, "prerelease": false, "assets": []},
        ])
    }

    #[test]
    fn the_newest_version_tag_is_offered_and_the_models_release_is_not() {
        if installer_suffix().is_none() {
            return;
        }
        let update = newest_release(&releases(), "0.1.0").expect("an update");
        assert_eq!(update.version, "0.3.0");
        assert_eq!(update.url, "https://x/installer");
        assert!(newest_release(&releases(), "0.3.0").is_none(), "already up to date");
    }

    #[test]
    fn a_release_without_its_installer_yet_is_not_offered() {
        let unfinished =
            serde_json::json!([{"tag_name": "v9.0.0", "draft": false, "prerelease": false, "assets": []}]);
        assert!(newest_release(&unfinished, "0.1.0").is_none());
    }

    #[test]
    fn a_download_is_checked_against_its_published_checksum() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("a.bin");
        std::fs::write(&file, b"hello").unwrap();
        // sha256 of "hello"
        let good = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify(&file, &format!("{good}  a.bin\n"), "a.bin").is_ok());
        assert!(verify(&file, &format!("{}  a.bin\n", "0".repeat(64)), "a.bin").is_err());
        assert!(verify(&file, &format!("{good}  other.bin\n"), "a.bin").is_err(), "no checksum for it");
    }
}
