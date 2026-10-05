use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, ensure, Context, Result};
use clap::{Args, Subcommand};

const REPO: &str = "carlosrmz009/OpenNote";
const TAG: &str = "models";
const MAGIC: &[u8; 4] = b"ONM1";

const RELEASE_KEY: Option<u64> = include!(concat!(env!("OUT_DIR"), "/release_key.rs"));

#[derive(Debug, Args)]
pub struct ModelsArgs {
    #[command(subcommand)]
    pub command: ModelsCommand,
}

#[derive(Debug, Subcommand)]
pub enum ModelsCommand {
    /// The models published on GitHub, and the ones downloaded here.
    List,
    /// Download a published model; then use it with `--model <name>`.
    Get { name: String },
    /// Seal a model file for publishing (official builds only).
    Seal {
        model: PathBuf,
        /// Where to write it. Next to the model, as `<name>.onm`, by default.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

pub fn dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("OpenNote").join("models")
}

pub fn open(name: &str) -> Result<Option<String>> {
    let path = dir().join(format!("{name}.onm"));
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    ensure!(bytes.starts_with(MAGIC), "{} is not an opennote model", path.display());
    let Some(key) = RELEASE_KEY else {
        bail!("this copy of opennote was not built to open published models; use an official build");
    };
    String::from_utf8(crate::seal::seal(&bytes[MAGIC.len()..], key))
        .map(Some)
        .with_context(|| format!("{} is damaged, or was sealed for another build", path.display()))
}

fn curl(args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("curl").args(["-fsSL", "--retry", "2"]).args(args).output().context("running curl")?;
    ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(out.stdout)
}

fn published() -> Result<Vec<String>> {
    let body = match curl(&[&format!("https://api.github.com/repos/{REPO}/releases/tags/{TAG}")]) {
        Err(e) if e.to_string().contains("404") => return Ok(Vec::new()),
        other => other?,
    };
    let text = String::from_utf8_lossy(&body);
    let mut names: Vec<String> = text
        .split("\"name\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').nth(1))
        .filter_map(|n| n.strip_suffix(".onm"))
        .map(str::to_string)
        .collect();
    names.dedup();
    Ok(names)
}

pub fn run(args: ModelsArgs) -> Result<()> {
    match args.command {
        ModelsCommand::List => {
            let here: Vec<String> = std::fs::read_dir(dir())
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect();
            match published() {
                Ok(names) if !names.is_empty() => {
                    println!("published:");
                    for n in &names {
                        println!("  {n}{}", if here.contains(n) { "  (downloaded)" } else { "" });
                    }
                }
                Ok(_) => println!("nothing published yet"),
                Err(e) => println!("could not reach GitHub: {e}"),
            }
            let built = env!("OPENNOTE_MODEL_NAME");
            println!("built in: {}", if built.is_empty() { "none (the rules alone)" } else { built });
            println!("downloaded models are kept in {}", dir().display());
        }
        ModelsCommand::Get { name } => {
            ensure!(name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)), "not a model name: {name}");
            std::fs::create_dir_all(dir())?;
            let path = dir().join(format!("{name}.onm"));
            let url = format!("https://github.com/{REPO}/releases/download/{TAG}/{name}.onm");
            curl(&["-o", &path.to_string_lossy(), &url]).with_context(|| format!("downloading {name}"))?;
            ensure!(open(&name).is_ok(), "{name} downloaded, but this build cannot open it");
            println!("{name} -> {}\nuse it with: opennote <command> --model {name}", path.display());
        }
        ModelsCommand::Seal { model, out } => {
            let Some(key) = RELEASE_KEY else { bail!("this build has no release key (models/release-key.txt)") };
            let text = std::fs::read(&model).with_context(|| format!("reading {}", model.display()))?;
            on_fingering::NgramPrior::from_json(std::str::from_utf8(&text)?).context("that is not a model")?;
            let out = out.unwrap_or_else(|| model.with_extension("onm"));
            let mut sealed = MAGIC.to_vec();
            sealed.extend(crate::seal::seal(&text, key));
            std::fs::write(&out, sealed)?;
            println!("sealed -> {}", out.display());
        }
    }
    Ok(())
}

pub fn resolve(model: &Path) -> Result<Option<String>> {
    if model.exists() {
        return std::fs::read_to_string(model).map(Some).with_context(|| format!("reading the model at {}", model.display()));
    }
    let name = model.to_string_lossy();
    if name == "none" {
        return Ok(None);
    }
    open(&name)?.map(Some).with_context(|| {
        format!("no model at {name}, and none of that name downloaded (see `opennote models list`)")
    })
}
