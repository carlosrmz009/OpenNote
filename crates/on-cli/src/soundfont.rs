//! The sampled piano OpenNote plays by default: Studio Yamaha C7. The installers carry it, and
//! source builds download it with `opennote soundfont install`.

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use clap::{Args, Subcommand};
use sha2::{Digest, Sha256};

use crate::models;

/// Where it is published.
const URL: &str = "https://www.dropbox.com/scl/fi/22nztj1tc81pa1x91lp4r/Studio_Yamaha_C7.7z?rlkey=0w3hsg3b9dd6os35j3hus78ol&st=5aeobxbk&dl=1";

/// The SHA-256 of that download, so that only exactly this file is ever used. If the file at the
/// link is replaced, installing stops until this is updated.
const SHA256: &str = "b375467baf88baf632b32a45d245030b0feeb4f99a01341a091f90c175888864";

/// The name of the soundfont inside the download.
const FILE: &str = "Studio Yamaha C7.sf2";

/// `opennote soundfont`: get the sampled piano.
#[derive(Debug, Args)]
pub struct SoundfontArgs {
    #[command(subcommand)]
    pub command: SoundfontCommand,
}

#[derive(Debug, Subcommand)]
pub enum SoundfontCommand {
    /// Download the Studio Yamaha C7 piano (about 30 MB; 52 MB once unpacked) into the assets folder.
    Install {
        /// The assets folder, if it is not where OpenNote would look.
        #[arg(long)]
        assets: Option<PathBuf>,
    },
}

fn sha256_of(path: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut hasher)?;
    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

fn find_file(folder: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(folder).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}

/// Downloads, checks and unpacks the soundfont into `<assets>/soundfont`.
pub fn install(assets: &Path) -> Result<PathBuf> {
    let work = std::env::temp_dir().join("opennote-soundfont");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let archive = work.join("piano.7z");
    models::curl_for(1800, &["-o", &archive.to_string_lossy(), URL]).context("downloading the soundfont")?;
    let found = sha256_of(&archive)?;
    ensure!(
        found == SHA256,
        "the soundfont download is not the file that was expected (checksum {found}), so it was not used"
    );

    let unpacked = work.join("unpacked");
    std::fs::create_dir_all(&unpacked)?;
    sevenz_rust2::decompress_file(&archive, &unpacked).context("unpacking the soundfont")?;
    let Some(file) = find_file(&unpacked, FILE) else {
        bail!("the download did not contain {FILE}");
    };

    let target_folder = assets.join("soundfont");
    std::fs::create_dir_all(&target_folder)?;
    let target = target_folder.join(FILE);
    std::fs::copy(&file, &target).context("putting the soundfont in place")?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(target)
}

pub fn run(args: SoundfontArgs) -> Result<()> {
    match args.command {
        SoundfontCommand::Install { assets } => {
            let assets = on_viz::session::assets_root(assets)?;
            let target = install(&assets)?;
            println!("Installed {}.", target.display());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_hashed_the_standard_way() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("a");
        std::fs::write(&file, b"hello").unwrap();
        assert_eq!(
            sha256_of(&file).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn the_pinned_checksum_is_a_sha256() {
        assert_eq!(SHA256.len(), 64);
        assert!(SHA256.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_file_is_found_in_a_subfolder() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(folder.path().join("a/b")).unwrap();
        std::fs::write(folder.path().join("a/b").join(FILE), b"x").unwrap();
        assert!(find_file(folder.path(), FILE).is_some());
        assert!(find_file(folder.path(), "nothing").is_none());
    }
}
