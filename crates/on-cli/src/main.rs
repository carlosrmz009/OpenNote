//! `opennote` — read a piano score, work out the fingering, and put it back on
//! the page.

mod annotate;
mod explain;
mod learn;
mod play;
mod render;
mod scene;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use on_fingering::rules::RuleSet;
use on_fingering::FingeringOptions;
use on_hand::{HandProfile, HandSize};

/// Work out piano fingerings and write them onto the score.
#[derive(Debug, Parser)]
#[command(name = "opennote", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Print more about what is happening.
    #[arg(long, short, global = true)]
    verbose: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Add fingerings to a score and save it, optionally engraving a PDF.
    Annotate(annotate::AnnotateArgs),
    /// Explain why each note was given the finger it was.
    Explain(explain::ExplainArgs),
    /// Watch the fingering played, with falling notes and 3D hands.
    Play(play::PlayArgs),
    /// Write the performance out as a video file, with sound.
    Render(render::RenderArgs),
    /// Collect expert fingerings to learn from.
    Corpus(learn::CorpusArgs),
    /// Fit the statistical model to the corpus.
    Train(learn::TrainArgs),
    /// Measure the fingering against expert annotations.
    Eval(learn::EvalArgs),
}

/// How big the pianist's hand is, and which rule set to score with.
#[derive(Debug, Args, Clone)]
pub struct ModelArgs {
    /// Hand size, if you have not measured your hand.
    #[arg(long, value_enum, default_value_t = HandSizeArg::Medium)]
    pub hand_size: HandSizeArg,

    /// Hand length in millimetres, from the wrist crease to the tip of the middle
    /// finger. More accurate than `--hand-size`, and worth measuring once.
    #[arg(long)]
    pub hand_length: Option<f32>,

    /// Which published rule set to score with.
    #[arg(long, value_enum, default_value_t = RuleSetArg::Badgerow)]
    pub rules: RuleSetArg,

    /// Measure intervals in semitones, as the original papers do, rather than in
    /// millimetres along a real keyboard.
    #[arg(long)]
    pub chromatic: bool,

    /// A trained statistical model to blend in alongside the rules. See
    /// `opennote train` and docs/TRAINING.md.
    #[arg(long)]
    pub model: Option<std::path::PathBuf>,

    /// How far the trained model is trusted against the rules. Ignored without
    /// `--model`.
    #[arg(long, default_value_t = 1.0)]
    pub prior_scale: f32,
}

impl ModelArgs {
    /// Turn the command line into solver options.
    pub fn options(&self) -> Result<FingeringOptions> {
        let profile = match self.hand_length {
            Some(mm) if !(120.0..=260.0).contains(&mm) => {
                bail!("a hand length of {mm} mm is outside the range this model covers (120-260)")
            }
            Some(mm) => HandProfile::from_hand_length(mm),
            None => HandProfile::from_size(self.hand_size.into()),
        };
        let mut options = FingeringOptions::for_hand(profile);
        options.rule_set = self.rules.into();
        if self.chromatic {
            options.ruler = on_fingering::Ruler::Chromatic;
        }
        if self.model.is_some() {
            options.prior_scale = self.prior_scale.max(0.0);
        }
        Ok(options)
    }

    /// Load the trained model, if one was asked for.
    ///
    /// A model that was named but cannot be read is an error rather than a shrug:
    /// silently fingering the piece by the rules alone, after being told to use a
    /// model, would be the wrong answer given confidently.
    pub fn prior(&self) -> Result<Option<std::sync::Arc<on_fingering::NgramPrior>>> {
        let Some(path) = &self.model else {
            return Ok(None);
        };
        let prior = on_fingering::NgramPrior::load(path)
            .with_context(|| format!("reading the model at {}", path.display()))?;
        Ok(Some(std::sync::Arc::new(prior)))
    }
}

/// Hand size, on the command line.
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum HandSizeArg {
    /// Extra small.
    Xs,
    /// Small.
    Small,
    /// Medium; the default.
    Medium,
    /// Large.
    Large,
    /// Extra large.
    Xl,
}

impl From<HandSizeArg> for HandSize {
    fn from(value: HandSizeArg) -> Self {
        match value {
            HandSizeArg::Xs => HandSize::ExtraSmall,
            HandSizeArg::Small => HandSize::Small,
            HandSizeArg::Medium => HandSize::Medium,
            HandSizeArg::Large => HandSize::Large,
            HandSizeArg::Xl => HandSize::ExtraLarge,
        }
    }
}

/// Rule set, on the command line.
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum RuleSetArg {
    /// Parncutt et al. (1997), the original twelve rules.
    Parncutt,
    /// Jacobs (2001).
    Jacobs,
    /// Balliauw et al.
    Balliauw,
    /// Badgerow's revision; the default.
    Badgerow,
}

impl From<RuleSetArg> for RuleSet {
    fn from(value: RuleSetArg) -> Self {
        match value {
            RuleSetArg::Parncutt => RuleSet::Parncutt,
            RuleSetArg::Jacobs => RuleSet::Jacobs,
            RuleSetArg::Balliauw => RuleSet::Balliauw,
            RuleSetArg::Badgerow => RuleSet::Badgerow,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let filter = if cli.verbose { "debug" } else { "warn" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| filter.into()),
        )
        .with_target(false)
        .without_time()
        .init();

    match cli.command {
        Command::Annotate(args) => annotate::run(args),
        Command::Explain(args) => explain::run(args),
        Command::Play(args) => play::run(args),
        Command::Render(args) => render::run(args),
        Command::Corpus(args) => learn::corpus(args),
        Command::Train(args) => learn::train(args),
        Command::Eval(args) => learn::eval(args),
    }
}
