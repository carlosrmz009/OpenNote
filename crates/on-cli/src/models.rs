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
    /// Teach a model the fingerings of documented pieces, so it plays those pieces as documented.
    Remember {
        model: PathBuf,
        /// Corpus folders of fingered pieces to remember.
        #[arg(long, required = true, num_args = 1..)]
        corpus: Vec<PathBuf>,
        /// Where to write the model. Over the model itself by default.
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
    unseal(&path).map(Some)
}

fn unseal(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    ensure!(bytes.starts_with(MAGIC), "{} is not an opennote model", path.display());
    let Some(key) = RELEASE_KEY else {
        bail!("this copy of opennote was not built to open published models; use an official build");
    };
    String::from_utf8(crate::seal::seal(&bytes[MAGIC.len()..], key))
        .with_context(|| format!("{} is damaged, or was sealed for another build", path.display()))
}

fn curl(args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("curl")
        .args(["-fsSL", "--retry", "2", "--connect-timeout", "20", "--max-time", "600"])
        .args(args)
        .output()
        .context("running curl")?;
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
                .map(|e| e.path())
                .filter(|path| path.extension().is_some_and(|e| e == "onm"))
                .filter_map(|path| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
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
            ensure!(
                name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)),
                "not a model name: {name}"
            );
            std::fs::create_dir_all(dir())?;
            let path = dir().join(format!("{name}.onm"));
            let partial = dir().join(format!("{name}.onm.part"));
            let url = format!("https://github.com/{REPO}/releases/download/{TAG}/{name}.onm");
            let fetched = curl(&["-o", &partial.to_string_lossy(), &url])
                .with_context(|| format!("downloading {name}"))
                .and_then(|_| {
                    unseal(&partial)
                        .with_context(|| format!("{name} downloaded, but this build cannot open it"))
                });
            if let Err(error) = fetched {
                let _ = std::fs::remove_file(&partial);
                return Err(error);
            }
            std::fs::rename(&partial, &path)
                .with_context(|| format!("moving the download to {}", path.display()))?;
            println!("{name} -> {}\nuse it with: opennote <command> --model {name}", path.display());
        }
        ModelsCommand::Seal { model, out } => {
            let Some(key) = RELEASE_KEY else {
                bail!("this build has no release key (models/release-key.txt)")
            };
            let text = std::fs::read(&model).with_context(|| format!("reading {}", model.display()))?;
            on_fingering::NgramPrior::from_json(std::str::from_utf8(&text)?)
                .context("that is not a model")?;
            let out = out.unwrap_or_else(|| model.with_extension("onm"));
            let mut sealed = MAGIC.to_vec();
            sealed.extend(crate::seal::seal(&text, key));
            std::fs::write(&out, sealed)?;
            println!("sealed -> {}", out.display());
        }
        ModelsCommand::Remember { model, corpus, out } => {
            let mut prior = on_fingering::NgramPrior::load(&model).context("reading the model")?;
            prior.known = on_fingering::Known::default();
            for path in &corpus {
                ensure!(path.join("normalised").is_dir(), "{} is not a corpus folder", path.display());
                let mut latest = std::collections::BTreeMap::new();
                for piece in on_train::corpus::Corpus::at(path)?.load()? {
                    latest.insert((piece.piece.clone(), piece.annotator.clone()), piece);
                }
                for piece in latest.into_values() {
                    prior.known.add(
                        piece
                            .notes
                            .iter()
                            .map(|n| on_fingering::KnownNote {
                                onset: n.onset,
                                midi: n.midi,
                                hand: n.hand.into(),
                                finger: n.finger.and_then(on_hand::Finger::from_number),
                                confidence: n.confidence.unwrap_or(1.0),
                            })
                            .collect(),
                    );
                }
                println!("{}: {} pieces remembered so far", path.display(), prior.known.len());
            }
            let out = out.unwrap_or(model);
            prior.save(&out)?;
            println!("remembered {} pieces -> {}", prior.known.len(), out.display());
        }
    }
    Ok(())
}

pub fn resolve(model: &Path) -> Result<Option<String>> {
    if model.exists() {
        return std::fs::read_to_string(model)
            .map(Some)
            .with_context(|| format!("reading the model at {}", model.display()));
    }
    let name = model.to_string_lossy();
    if name == "none" {
        return Ok(None);
    }
    open(&name)?.map(Some).with_context(|| {
        format!("no model at {name}, and none of that name downloaded (see `opennote models list`)")
    })
}
