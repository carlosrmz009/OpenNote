//! `opennote train` and `opennote eval`: fit the n-gram model to the corpus, and measure a
//! model or a single score.

use super::*;

/// Fit the statistical model to the corpus.
#[derive(Debug, Args)]
pub struct TrainArgs {
    /// The corpus directory.
    #[arg(long, default_value = CORPUS_DIR)]
    pub corpus: PathBuf,

    /// Where to write the model.
    #[arg(long, short, default_value = MODEL_PATH)]
    pub out: PathBuf,

    /// The fraction of pieces held back to measure on, rather than learned from.
    #[arg(long, default_value_t = 0.2, value_parser = crate::between(0.0f32, 0.9))]
    pub held_out: f32,

    /// Also fit how far the model, the published rules and the standard chord shapes
    /// are trusted against each other. Slower, and worth it once the corpus is more
    /// than a handful of pieces.
    #[arg(long)]
    pub tune_weights: bool,

    /// Print what the model learned, in plain language.
    #[arg(long)]
    pub explain: bool,

    /// Work out the numbers but do not write the model.
    #[arg(long)]
    pub dry_run: bool,

    /// Leave out any finger a machine read with less than this confidence, from 0 to 1.
    /// A finger a person wrote is always kept. Nought, the default, keeps everything.
    ///
    /// Only matters for fingerings read out of video, and the right value is not known
    /// in advance: raise it until the benchmark stops improving, which is the point at
    /// which the readings being dropped were doing more harm than good.
    #[arg(long, default_value_t = 0.0)]
    pub min_confidence: f32,

    #[command(flatten)]
    pub model: ModelArgs,
}

/// Measure a fingering against expert annotations.
#[derive(Debug, Args)]
pub struct EvalArgs {
    /// A score to measure on its own terms: `.musicxml`, `.mxl`, `.xml`, `.mid` or
    /// `.midi`. Reports whether the fingering is playable and how much it moves the
    /// hand, neither of which needs annotations. Without it, the corpus is measured
    /// against the fingerings pianists wrote.
    pub score: Option<PathBuf>,

    /// The corpus directory.
    #[arg(long, default_value = CORPUS_DIR)]
    pub corpus: PathBuf,

    /// Measure on everything, rather than only on the pieces held back from
    /// training. Flattering, and only useful for checking that a model was written.
    #[arg(long)]
    pub all: bool,

    /// The fraction held back. Has to match what training used, or the pieces
    /// measured on are not the ones that were held back.
    #[arg(long, default_value_t = 0.2, value_parser = crate::between(0.0f32, 0.9))]
    pub held_out: f32,

    #[command(flatten)]
    pub weights: ModelArgs,
}

pub fn train(args: TrainArgs) -> Result<()> {
    let corpus = Corpus::at(&args.corpus)?;
    let pieces = trusted(corpus.load()?, args.min_confidence);
    if pieces.is_empty() {
        bail!(
            "the corpus at {} is empty, so there is nothing to learn from.\n\
             Add some expert fingerings first: opennote corpus add <path>",
            args.corpus.display()
        );
    }

    let base = args.model.options()?;
    println!("Training on {} fingerings…", pieces.len());
    let report = on_train::train(&pieces, &base, args.held_out, args.tune_weights);

    for line in report.summary() {
        println!("{line}");
    }
    if args.explain {
        println!();
        for line in report.prior.explain(8) {
            println!("{line}");
        }
    }

    if args.dry_run {
        println!("\nNothing written (--dry-run).");
        return Ok(());
    }
    report.prior.save(&args.out).with_context(|| format!("writing {}", args.out.display()))?;
    println!("\nModel written to {}.", args.out.display());
    if report.made_things_worse() {
        println!(
            "It scored below the rules alone on the held-out pieces, so it is written \
             but not worth using yet. Add more data and train again."
        );
    } else {
        println!("Use it with:  opennote play <score> --model {}", args.out.display());
    }
    Ok(())
}

pub fn eval(args: EvalArgs) -> Result<()> {
    if let Some(score) = args.score.clone() {
        return eval_score(&score, &args);
    }
    let corpus = Corpus::at(&args.corpus)?;
    let pieces = corpus.load()?;
    if pieces.is_empty() {
        bail!("the corpus at {} is empty", args.corpus.display());
    }

    let measured = if args.all {
        pieces.clone()
    } else {
        let split = on_train::split(&pieces, args.held_out);
        if split.test.is_empty() {
            println!("Not enough pieces to hold any back; measuring on all of them.");
            pieces.clone()
        } else {
            split.test
        }
    };

    let mut options = args.weights.options()?;
    options.prior_scale = 0.0;
    let rules_only = on_train::evaluate(&measured, &options, None);
    println!("rules only:  {rules_only}");

    let model_path = args.weights.model.clone().unwrap_or_else(|| PathBuf::from(MODEL_PATH));
    let model = crate::ModelArgs { model: Some(model_path.clone()), ..args.weights.clone() };
    match model.prior() {
        Ok(Some(prior)) => {
            let mut with_model = args.weights.options()?;
            if with_model.prior_scale == 0.0 {
                with_model.prior_scale = 1.0;
            }
            let rates = on_train::evaluate(&measured, &with_model, Some(&*prior));
            println!("with model:  {rates}");
            println!("{}", verdict(rules_only, rates));
        }
        Ok(None) => {}
        Err(error) => {
            println!(
                "\nNo model to compare against at {} ({error}).\n\
                 Train one with: opennote train",
                model_path.display()
            );
        }
    }
    Ok(())
}

fn verdict(before: MatchRates, after: MatchRates) -> String {
    let change = (after.general - before.general) * 100.0;
    match change {
        c if c > 0.5 => format!("The model is worth {c:.1} points of agreement."),
        c if c < -0.5 => format!("The model costs {:.1} points. Do not use it yet — it needs more data.", -c),
        _ => "The model makes almost no difference on this set.".to_string(),
    }
}

fn eval_score(path: &Path, args: &EvalArgs) -> Result<()> {
    let crate::Fingered { document: input, solution, options } = args.weights.finger(path)?;
    let score = input.score();

    let spans = options.span_model.table();
    let measured = on_fingering::measure_solution(score, &solution, spans);
    let title = score.title.clone().unwrap_or_else(|| "untitled".into());
    println!("{title}: {} notes fingered", solution.fingerings.len());
    println!("  {measured}");
    if measured.unplayable > 0 {
        println!(
            "\n  {} transition(s) no hand could make in the time the music allows.\n  \
             Either the search went wrong, or the part as assigned is not playable by\n  \
             one hand — `opennote annotate` will say if the passage is out of reach.",
            measured.unplayable
        );
        for flaw in measured.flaws.iter().filter(|f| f.unplayable()) {
            println!("    {flaw}");
        }
    }
    let cleared = measured.impossible - measured.unplayable;
    if cleared > 0 {
        println!(
            "\n  {cleared} further crossing(s) are counted by the IFR but had time to\n  \
             happen. They are in that figure because Zhao et al. define it on a corpus\n  \
             with no durations, so it cannot ask; they are not faults."
        );
    }
    Ok(())
}
