use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use on_fingering::NgramPrior;
use on_train::{Corpus, MatchRates};

use crate::ModelArgs;

mod bench;
mod corpus;
mod rerank;
mod train;
mod tune;

pub use bench::*;
pub use corpus::*;
pub use rerank::*;
pub use train::*;
pub use tune::*;

const CORPUS_DIR: &str = "corpus";

const MODEL_PATH: &str = "models/prior.json";

fn trusted(pieces: Vec<on_train::Piece>, floor: f32) -> Vec<on_train::Piece> {
    if floor <= 0.0 {
        return pieces;
    }
    let labelled = |ps: &[on_train::Piece]| -> usize {
        ps.iter().flat_map(|p| &p.notes).filter(|n| n.finger.is_some()).count()
    };
    let before = labelled(&pieces);
    let kept: Vec<on_train::Piece> = pieces.iter().map(|p| p.trusted(floor)).collect();
    let after = labelled(&kept);
    println!(
        "Keeping {after} of {before} fingerings at a confidence of {floor} or more; \
         {} were machine readings below it.",
        before - after
    );
    kept
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}
