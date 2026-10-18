//! The `opennote` command line: the subcommands, and the hand, rules and model options they
//! share.

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod annotate;
mod explain;
mod learn;
mod models;
mod play;
mod render;
mod scene;
mod seal;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use on_fingering::rules::RuleSet;
use on_fingering::FingeringOptions;
use on_hand::{HandProfile, HandSize};

/// Work out piano fingerings and write them onto the score.
#[derive(Debug, Parser)]
#[command(name = "opennote", version, about, long_about = None, args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// A score to open in the viewer. With no score and no command, the viewer opens
    /// and asks for one.
    score: Option<std::path::PathBuf>,

    /// Print more about what is happening.
    #[arg(long, short, global = true, conflicts_with = "quiet")]
    verbose: bool,

    /// Print only errors.
    #[arg(long, short, global = true)]
    quiet: bool,
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
    /// Search the engine's weights for as long as it is left running.
    Tune(learn::TuneArgs),
    /// Measure against a dataset without learning anything from it.
    Bench(learn::BenchArgs),
    /// Measure a model on OpenNote's own benchmark, into a scorecard.
    Benchmark(learn::BenchmarkArgs),
    /// Older models published on GitHub: list them, download one.
    Models(models::ModelsArgs),
    /// Teach the search to finger like the pianists in the corpus: the rules propose,
    /// what is learned decides between them.
    Rerank(learn::RerankArgs),
    /// Print a shell completion script, e.g. `opennote completions bash > ~/.local/share/bash-completion/completions/opennote`.
    Completions {
        /// The shell to complete for.
        shell: clap_complete::Shell,
    },
    /// Print the manual page, e.g. `opennote manual > opennote.1`.
    Manual,
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

    /// Which rule set to score with. The default combines all four.
    #[arg(long, value_enum, default_value_t = RuleSetArg::Consensus)]
    pub rules: RuleSetArg,

    /// Measure intervals in semitones, as the original papers do, rather than in
    /// millimetres along a real keyboard.
    #[arg(long)]
    pub chromatic: bool,

    /// A trained statistical model to blend in alongside the rules: a file, the name of a
    /// model downloaded with `opennote models get`, or `none` for the rules alone. Without
    /// it, the model built into this copy of opennote, if it has one.
    #[arg(long)]
    pub model: Option<std::path::PathBuf>,

    /// Which of a model's learned weights to finger with: `unified`, one model for
    /// everything (the default); or, for a model trained with separate styles,
    /// `performance` (how pianists play) or `classical` (how editions are fingered).
    /// Ignored without `--model`.
    #[arg(long, value_enum, default_value_t = StyleArg::Unified)]
    pub style: StyleArg,

    /// How far the trained model is trusted against the rules. Ignored without
    /// `--model`.
    #[arg(long, default_value_t = 1.0, value_parser = between(0.0f32, 100.0))]
    pub prior_scale: f32,

    /// Weights found by `opennote tune`, to try them before they become the defaults.
    #[arg(long)]
    pub tuned: Option<std::path::PathBuf>,
}

impl ModelArgs {
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
            options.prior_scale = self.prior_scale;
        }
        if let Some(weights) = self.weights()? {
            weights.apply_to_fingering(&mut options);
        }
        Ok(options)
    }

    pub fn hands(&self, options: &FingeringOptions) -> Result<on_score::hands::HandAssignment> {
        let mut assignment =
            on_score::hands::HandAssignment { profile: options.profile.clone(), ..Default::default() };
        if let Some(weights) = self.weights()? {
            weights.apply_to_hands(&mut assignment);
        }
        Ok(assignment)
    }

    fn weights(&self) -> Result<Option<on_train::tune::Weights>> {
        self.tuned.as_deref().map(on_train::tune::Weights::load).transpose()
    }

    /// Reads a score, gives each note a hand and fingers it: what every command that
    /// fingers a score file starts with.
    pub fn finger(&self, path: &std::path::Path) -> Result<Fingered> {
        if !path.exists() {
            bail!("{} does not exist", path.display());
        }
        let options = self.options()?;
        let mut document =
            on_score::Document::open(path).with_context(|| format!("reading {}", path.display()))?;
        on_score::assign_hands(document.score_mut(), &self.hands(&options)?);
        let prior = self.prior()?;
        let solution = on_fingering::finger_score_with_prior(
            document.score(),
            &options,
            prior.as_deref().map(|p| p as &dyn on_fingering::FingeringPrior),
        );
        Ok(Fingered { document, solution, options })
    }

    pub fn prior(&self) -> Result<Option<std::sync::Arc<on_fingering::NgramPrior>>> {
        let text = match &self.model {
            Some(model) => models::resolve(model)?,
            None => built_in()?,
        };
        let Some(text) = text else { return Ok(None) };
        let mut prior = on_fingering::NgramPrior::from_json(&text).context("reading the model")?;
        if let Some(learned) = &mut prior.learned {
            learned.style = self.style.into();
        }
        Ok(Some(std::sync::Arc::new(prior)))
    }
}

/// A command-line number that must lie within a range, refused with a clear message
/// rather than quietly moved into it.
pub fn between<T>(
    low: T,
    high: T,
) -> impl Fn(&str) -> std::result::Result<T, String> + Clone + Send + Sync + 'static
where
    T: std::str::FromStr + PartialOrd + std::fmt::Display + Copy + Send + Sync + 'static,
{
    move |text: &str| {
        let value: T = text.trim().parse().map_err(|_| format!("{text} is not a number"))?;
        if value >= low && value <= high {
            Ok(value)
        } else {
            Err(format!("{value} is outside {low} to {high}"))
        }
    }
}

/// A score with its hands assigned, the fingering found for it, and the options used.
pub struct Fingered {
    pub document: on_score::Document,
    pub solution: on_fingering::Solution,
    pub options: FingeringOptions,
}

fn built_in() -> Result<Option<String>> {
    const SEALED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/model.bin"));
    if SEALED.is_empty() {
        return Ok(None);
    }
    String::from_utf8(seal::seal(SEALED, include!(concat!(env!("OUT_DIR"), "/model_key.rs"))))
        .map(Some)
        .context("the model built into opennote is damaged")
}

/// Fingering style, on the command line.
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum StyleArg {
    /// One model for everything.
    Unified,
    /// How pianists play: learned from performances.
    Performance,
    /// How editions are fingered: learned from written fingerings.
    Classical,
}

impl From<StyleArg> for on_fingering::learned::Style {
    fn from(value: StyleArg) -> Self {
        match value {
            StyleArg::Unified => Self::Unified,
            StyleArg::Performance => Self::Performance,
            StyleArg::Classical => Self::Classical,
        }
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
    /// Every rule any of the four published sets charges for, weighted by how many
    /// of them endorse it; the default, and what the engine is tuned around.
    Consensus,
    /// Parncutt et al. (1997), the original twelve rules.
    Parncutt,
    /// Jacobs (2001).
    Jacobs,
    /// Balliauw et al.
    Balliauw,
    /// Badgerow's revision.
    Badgerow,
}

impl From<RuleSetArg> for RuleSet {
    fn from(value: RuleSetArg) -> Self {
        match value {
            RuleSetArg::Consensus => RuleSet::Consensus,
            RuleSetArg::Parncutt => RuleSet::Parncutt,
            RuleSetArg::Jacobs => RuleSet::Jacobs,
            RuleSetArg::Balliauw => RuleSet::Balliauw,
            RuleSetArg::Badgerow => RuleSet::Badgerow,
        }
    }
}

/// Started by double-clicking on Windows, there is a console window nobody asked for;
/// close it. Started from a terminal, it is shared with the shell and stays.
#[cfg(windows)]
fn leave_the_console_if_alone() {
    extern "system" {
        fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
        fn FreeConsole() -> i32;
    }
    let mut list = [0u32; 2];
    // SAFETY: both are plain Win32 calls; `list` holds the two entries asked for.
    unsafe {
        if GetConsoleProcessList(list.as_mut_ptr(), 2) <= 1 {
            FreeConsole();
        }
    }
}

fn main() -> Result<()> {
    #[cfg(windows)]
    if std::env::args_os().len() <= 1 {
        leave_the_console_if_alone();
    }
    let cli = Cli::parse();
    // Debug output from OpenNote's own crates; a dependency's debug output, Bevy's above all,
    // drowns it. RUST_LOG still overrides this.
    let filter = match (cli.verbose, cli.quiet) {
        (true, _) => "warn,opennote=debug,on_score=debug,on_hand=debug,on_fingering=debug,on_train=debug,on_engrave=debug,on_audio=debug,on_viz=debug",
        (_, true) => "error",
        _ => "warn",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()),
        )
        .with_target(false)
        .without_time()
        .init();

    let Some(command) = cli.command else {
        return play::launch(cli.score);
    };
    match command {
        Command::Annotate(args) => annotate::run(args),
        Command::Explain(args) => explain::run(args),
        Command::Play(args) => play::run(args),
        Command::Render(args) => render::run(args),
        Command::Corpus(args) => learn::corpus(args),
        Command::Train(args) => learn::train(args),
        Command::Eval(args) => learn::eval(args),
        Command::Tune(args) => learn::tune(args),
        Command::Bench(args) => learn::bench(args),
        Command::Benchmark(args) => learn::benchmark(args),
        Command::Models(args) => models::run(args),
        Command::Rerank(args) => learn::rerank(args),
        Command::Completions { shell } => {
            use clap::CommandFactory;
            clap_complete::generate(shell, &mut Cli::command(), "opennote", &mut std::io::stdout());
            Ok(())
        }
        Command::Manual => {
            use clap::CommandFactory;
            clap_mangen::Man::new(Cli::command()).render(&mut std::io::stdout())?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_well_formed() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn a_number_out_of_range_is_refused_not_moved() {
        let expression = between(0.0f32, 2.0);
        assert_eq!(expression("1.5"), Ok(1.5));
        assert!(expression("2.5").is_err());
        assert!(expression("lots").is_err());
    }
}
