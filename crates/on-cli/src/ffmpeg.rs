//! ffmpeg, which turns the rendered frames into a video: finding a copy that is already
//! there, and downloading one into OpenNote's own folder when there is none.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, ensure, Context, Result};
use clap::{Args, Subcommand};
use sha2::{Digest, Sha256};

use crate::models;

/// Where the downloaded copy lives.
pub fn dir() -> PathBuf {
    models::dir().parent().map_or_else(|| PathBuf::from("ffmpeg"), |data| data.join("ffmpeg"))
}

fn downloaded() -> PathBuf {
    dir().join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" })
}

fn works(path: &Path) -> bool {
    Command::new(path)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Every place worth looking, in order. A program started by clicking an icon does not get
/// the shell's PATH, so the usual places are tried by name.
fn candidates(asked: &Path) -> Vec<PathBuf> {
    let mut found = vec![asked.to_path_buf()];
    if asked == Path::new("ffmpeg") {
        found.push(downloaded());
        for place in
            ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg", "/opt/local/bin/ffmpeg", "/usr/bin/ffmpeg"]
        {
            found.push(PathBuf::from(place));
        }
    }
    found
}

/// A copy of ffmpeg that runs: the one named, or one OpenNote can find.
pub fn find(asked: &Path) -> Option<PathBuf> {
    candidates(asked).into_iter().find(|path| works(path))
}

/// What to tell someone who has no ffmpeg.
pub fn missing_help() -> String {
    let how = if cfg!(target_os = "macos") {
        "Install it with Homebrew (`brew install ffmpeg`), or download it from https://ffmpeg.org/download.html, \
         then run this again."
    } else {
        "OpenNote can download it for you: run `opennote ffmpeg install`, or press the download button \
         when the viewer asks."
    };
    format!("Making a video needs a free helper called ffmpeg, and there is none on this computer.\n{how}")
}

/// Whether OpenNote can fetch ffmpeg itself on this system.
pub fn can_install() -> bool {
    download_target().is_some()
}

/// The release file for this system, from the BtbN builds, which are the ones ffmpeg's own
/// download page points to for Windows and Linux.
fn download_target() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("ffmpeg-master-latest-win64-gpl.zip")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("ffmpeg-master-latest-linux64-gpl.tar.xz")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("ffmpeg-master-latest-linuxarm64-gpl.tar.xz")
    } else {
        None
    }
}

const RELEASE: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest";

/// Downloads ffmpeg, checks it against the checksum its publisher lists, and unpacks it.
pub fn install() -> Result<PathBuf> {
    let Some(file) = download_target() else {
        bail!("{}", missing_help());
    };
    let work = std::env::temp_dir().join("opennote-ffmpeg");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let archive = work.join(file);

    models::curl_for(1800, &["-o", &archive.to_string_lossy(), &format!("{RELEASE}/{file}")])
        .context("downloading ffmpeg")?;
    let sums =
        models::curl(&[&format!("{RELEASE}/checksums.sha256")]).context("downloading its checksums")?;
    verify(&archive, &String::from_utf8_lossy(&sums), file)?;

    let unpacked = work.join("unpacked");
    std::fs::create_dir_all(&unpacked)?;
    // `tar` unpacks both kinds: Windows 10 and later ship one that reads zip files.
    let status = Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(&unpacked)
        .status()
        .context("running tar to unpack ffmpeg")?;
    ensure!(status.success(), "could not unpack the ffmpeg download");

    let name = downloaded().file_name().map(|name| name.to_owned()).context("no file name")?;
    let program = find_in(&unpacked, &name).context("the download did not contain ffmpeg")?;
    std::fs::create_dir_all(dir())?;
    let target = downloaded();
    std::fs::copy(&program, &target).context("putting ffmpeg in place")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
    }
    let _ = std::fs::remove_dir_all(&work);
    ensure!(works(&target), "ffmpeg was installed but does not run on this computer");
    Ok(target)
}

fn find_in(folder: &Path, name: &std::ffi::OsStr) -> Option<PathBuf> {
    for entry in std::fs::read_dir(folder).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_in(&path, name) {
                return Some(found);
            }
        } else if path.file_name() == Some(name) {
            return Some(path);
        }
    }
    None
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
        "the ffmpeg download is damaged (its checksum does not match), so it was not used"
    );
    Ok(())
}

/// `opennote ffmpeg`: check for the video helper, or download it.
#[derive(Debug, Args)]
pub struct FfmpegArgs {
    #[command(subcommand)]
    pub command: FfmpegCommand,
}

#[derive(Debug, Subcommand)]
pub enum FfmpegCommand {
    /// Say whether ffmpeg can be found: `found <path>`, `can-download` or `install-by-hand`.
    Check,
    /// Download ffmpeg into OpenNote's own folder (Windows and Linux).
    Install {
        /// Download it even if a copy is already available, to replace a broken one.
        #[arg(long)]
        force: bool,
    },
}

pub fn run(args: FfmpegArgs) -> Result<()> {
    match args.command {
        // One word the viewer can read: where it is, or what can be done about it.
        FfmpegCommand::Check => {
            match find(Path::new("ffmpeg")) {
                Some(path) => println!("found {}", path.display()),
                None if can_install() => println!("can-download"),
                None => println!("install-by-hand"),
            }
            Ok(())
        }
        FfmpegCommand::Install { force } => {
            if let (false, Some(path)) = (force, find(Path::new("ffmpeg"))) {
                println!("ffmpeg is already available: {}", path.display());
                return Ok(());
            }
            let path = install()?;
            println!("ffmpeg installed: {}", path.display());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_is_checked_against_its_published_checksum() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("a.zip");
        std::fs::write(&file, b"hello").unwrap();
        let good = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify(&file, &format!("{good}  a.zip\n"), "a.zip").is_ok());
        assert!(verify(&file, &format!("{}  a.zip\n", "0".repeat(64)), "a.zip").is_err());
        assert!(verify(&file, &format!("{good}  b.zip\n"), "a.zip").is_err());
    }

    #[test]
    fn a_named_ffmpeg_is_used_as_given_and_the_default_looks_around() {
        assert_eq!(candidates(Path::new("/opt/my/ffmpeg")), vec![PathBuf::from("/opt/my/ffmpeg")]);
        let looked = candidates(Path::new("ffmpeg"));
        assert!(looked.len() > 2);
        assert_eq!(looked[0], PathBuf::from("ffmpeg"), "PATH first");
        assert!(looked.contains(&downloaded()));
    }

    #[test]
    fn finding_a_program_in_a_folder_looks_in_subfolders() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(folder.path().join("a/bin")).unwrap();
        std::fs::write(folder.path().join("a/bin/ffmpeg"), b"x").unwrap();
        assert!(find_in(folder.path(), std::ffi::OsStr::new("ffmpeg")).is_some());
        assert!(find_in(folder.path(), std::ffi::OsStr::new("nothing")).is_none());
    }
}
