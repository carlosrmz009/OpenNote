//! `opennote corpus`, `train` and `eval` — teaching the model from examples.
//!
//! Written for somebody who has never trained a model. Every command says what it
//! did, in words, and `train` always reports what the fingering was like before and
//! what it is like after, on pieces the model was not allowed to see. That comparison
//! is the point: adding data can make a model worse, and the only protection against
//! not noticing is to put the two numbers next to each other every single time.
//!
//! See `docs/TRAINING.md` for the walkthrough.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use on_fingering::NgramPrior;
use on_train::{Corpus, MatchRates};

use crate::ModelArgs;

/// Where the corpus and the model live by default.
const CORPUS_DIR: &str = "corpus";
const MODEL_PATH: &str = "models/prior.json";

/// Manage the collection of expert fingerings to learn from.
#[derive(Debug, Args)]
pub struct CorpusArgs {
    #[command(subcommand)]
    pub command: CorpusCommand,

    /// The corpus directory.
    #[arg(long, default_value = CORPUS_DIR, global = true)]
    pub corpus: PathBuf,
}

#[derive(Debug, Subcommand)]
pub enum CorpusCommand {
    /// Read fingering data into the corpus.
    Add {
        /// A file, or a directory to search. PIG `_fingering.txt` files and MusicXML
        /// that already carries `<fingering>` marks are both understood.
        path: PathBuf,

        /// A name for this batch, used for the file it is written to.
        #[arg(long)]
        name: Option<String>,
    },
    /// Work out where the keyboard is in a video frame.
    ///
    /// Takes the four corners clicked by `tools/watch_pianist.py calibrate` and fits
    /// the projective map from pixels to millimetres of piano. Only needed once per
    /// camera position, which in practice is once per channel.
    Calibrate {
        /// The corners file the tool wrote.
        corners: PathBuf,

        /// Where to write the calibration.
        #[arg(long, short, default_value = "camera.json")]
        out: PathBuf,
    },
    /// Read fingerings out of an overhead video of somebody playing.
    ///
    /// Needs three things: the hand landmarks, which `tools/watch_pianist.py` writes
    /// from the video; a calibration saying where the keyboard is in the frame, which
    /// the same tool writes once per camera position; and a MIDI or MusicXML file of
    /// the same performance, which says which keys went down and when.
    Watch {
        /// The landmark file.
        landmarks: PathBuf,

        /// A MIDI or MusicXML file of the same performance.
        #[arg(long)]
        score: PathBuf,

        /// The calibration written by `watch_pianist.py --calibrate`.
        #[arg(long)]
        calibration: PathBuf,

        /// What to call the piece in the corpus. Defaults to the score's filename.
        #[arg(long)]
        piece: Option<String>,

        /// Who played it. Two videos of the same piece by different pianists are two
        /// fingerings of it, which is what the highest and soft match rates need.
        #[arg(long, default_value = "video")]
        annotator: String,

        /// Seconds to add to the video's clock to line it up with the score,
        /// overriding whatever the landmark file says.
        #[arg(long)]
        offset: Option<f64>,

        /// A name for this batch, used for the file it is written to.
        #[arg(long)]
        name: Option<String>,
    },
    /// Say what is in the corpus.
    List,
}

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
    #[arg(long, default_value_t = 0.2)]
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
    #[arg(long, default_value_t = 0.2)]
    pub held_out: f32,

    #[command(flatten)]
    pub weights: ModelArgs,
}

/// Run `opennote corpus`.
pub fn corpus(args: CorpusArgs) -> Result<()> {
    let corpus = Corpus::at(&args.corpus)?;
    match args.command {
        CorpusCommand::Add { path, name } => {
            if !path.exists() {
                bail!("{} does not exist", path.display());
            }
            let report = corpus.add(&path, name.as_deref())?;
            println!(
                "Added {} fingering{} of {} piece{}, {} notes altogether.",
                report.annotations,
                plural(report.annotations),
                report.pieces,
                plural(report.pieces),
                report.notes
            );
            println!("Written to {}.", report.file.display());
            if !report.skipped.is_empty() {
                println!("\nCould not read {} file(s):", report.skipped.len());
                for line in report.skipped.iter().take(10) {
                    println!("  {line}");
                }
            }
            println!("\nNext: opennote train");
        }
        CorpusCommand::Calibrate { corners, out } => {
            let clicked = on_train::video::Corners::read(&corners)?;
            let homography = clicked.fit()?;
            homography.write(&out)?;
            println!(
                "The keyboard runs from {} to {} across the frame.",
                on_hand::keyboard::name(clicked.lowest_white),
                on_hand::keyboard::name(clicked.highest_white)
            );
            println!("Written to {}.", out.display());
            println!(
                "\nNext:  opennote corpus watch <landmarks.json> --score <file.mid> \
                 --calibration {}",
                out.display()
            );
        }
        CorpusCommand::Watch {
            landmarks,
            score,
            calibration,
            piece,
            annotator,
            offset,
            name,
        } => {
            let mut watched = on_train::Watched::read(&landmarks)?;
            if let Some(seconds) = offset {
                watched.offset = seconds;
            }
            let homography = on_train::Homography::read(&calibration)?;

            let onsets = on_train::video::onsets_of(&score)?;
            let title = piece.unwrap_or_else(|| {
                score
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("untitled")
                    .to_string()
            });
            let extracted =
                on_train::video::extract(&watched, &onsets, &homography, &title, &annotator);
            let (found, total) = on_train::video::coverage(&extracted, &onsets);
            if found == 0 {
                bail!(
                    "no fingerings could be read from {}.\n\
                     The usual cause is a calibration that does not match this video, or \
                     a score that is not the performance in it. Check that the two start \
                     together — `--offset` shifts the video's clock against the score's.",
                    landmarks.display()
                );
            }

            let file = corpus.add_piece(&extracted, name.as_deref())?;
            let hands = on_train::video::per_hand(&extracted);
            println!(
                "Read {found} of {total} notes ({:.0}%): {} left hand, {} right.",
                100.0 * found as f64 / total.max(1) as f64,
                hands.get("left").copied().unwrap_or(0),
                hands.get("right").copied().unwrap_or(0)
            );
            println!("Written to {}.", file.display());
            if found * 4 < total * 3 {
                println!(
                    "\nThat is a lot of notes with no finger on them. Either the hands \
                     leave the frame, or the video and the score have drifted apart — \
                     try `--offset`."
                );
            }
            println!("\nNext: opennote train");
        }
        CorpusCommand::List => {
            let pieces = corpus.load()?;
            if pieces.is_empty() {
                println!(
                    "The corpus at {} is empty.\n\
                     Add some expert fingerings with `opennote corpus add <path>` — \
                     see docs/TRAINING.md for where to find them.",
                    args.corpus.display()
                );
                return Ok(());
            }
            let grouped = on_train::corpus::by_piece(&pieces);
            let notes: usize = pieces.iter().map(|p| p.notes.len()).sum();
            println!(
                "{} piece{}, {} fingering{}, {} notes.",
                grouped.len(),
                plural(grouped.len()),
                pieces.len(),
                plural(pieces.len()),
                notes
            );
            for (name, annotations) in grouped.iter().take(40) {
                let people: Vec<&str> =
                    annotations.iter().map(|p| p.annotator.as_str()).collect();
                println!("  {name}  ({})", people.join(", "));
            }
            if grouped.len() > 40 {
                println!("  … and {} more", grouped.len() - 40);
            }
        }
    }
    Ok(())
}

/// Run `opennote train`.
pub fn train(args: TrainArgs) -> Result<()> {
    let corpus = Corpus::at(&args.corpus)?;
    let pieces = corpus.load()?;
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
    report
        .prior
        .save(&args.out)
        .with_context(|| format!("writing {}", args.out.display()))?;
    println!("\nModel written to {}.", args.out.display());
    if report.made_things_worse() {
        println!(
            "It scored below the rules alone on the held-out pieces, so it is written \
             but not worth using yet. Add more data and train again."
        );
    } else {
        println!(
            "Use it with:  opennote play <score> --model {}",
            args.out.display()
        );
    }
    Ok(())
}

/// Run `opennote eval`.
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

    // `--model` comes from the shared model arguments, so `eval` and `play` name the
    // same file the same way. Eval is the one command with a sensible default for it,
    // because looking for a model is the whole point of running it.
    let model_path = args
        .weights
        .model
        .clone()
        .unwrap_or_else(|| PathBuf::from(MODEL_PATH));
    match NgramPrior::load(&model_path) {
        Ok(prior) => {
            let mut with_model = args.weights.options()?;
            if with_model.prior_scale == 0.0 {
                with_model.prior_scale = 1.0;
            }
            let rates = on_train::evaluate(&measured, &with_model, Some(&prior));
            println!("with model:  {rates}");
            println!("{}", verdict(rules_only, rates));
        }
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

/// What the two numbers mean, said plainly.
fn verdict(before: MatchRates, after: MatchRates) -> String {
    let change = (after.general - before.general) * 100.0;
    match change {
        c if c > 0.5 => format!("The model is worth {c:.1} points of agreement."),
        c if c < -0.5 => format!(
            "The model costs {:.1} points. Do not use it yet — it needs more data.",
            -c
        ),
        _ => "The model makes almost no difference on this set.".to_string(),
    }
}

/// "s", unless there is exactly one.
fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Measure one score with no annotations to compare against.
///
/// Two things can be said about a fingering without knowing what a pianist would have
/// written. Whether a hand could perform it at all — which has a right answer, zero,
/// and is a regression test rather than a benchmark. And how much it moves the hand,
/// which has no right answer but is comparable between two runs over the same music,
/// and so says whether a change made the fingering calmer or busier.
///
/// This is the only measurement available on music nobody has annotated, which is to
/// say on nearly all music.
fn eval_score(path: &Path, args: &EvalArgs) -> Result<()> {
    if !path.exists() {
        bail!("{} does not exist", path.display());
    }
    let options = args.weights.options()?;
    let mut input = on_score::Document::open(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let assignment = args.weights.hands(&options)?;
    on_score::assign_hands(input.score_mut(), &assignment);
    let score = input.score();

    let prior = args.weights.prior()?;
    let solution = on_fingering::finger_score_with_prior(
        score,
        &options,
        prior.as_deref().map(|p| p as &dyn on_fingering::FingeringPrior),
    );

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

/// Arguments to `opennote tune`.
#[derive(Debug, Args)]
pub struct TuneArgs {
    /// What to tune: which hand plays each note, or which finger.
    #[arg(long, value_enum, default_value_t = TuneTarget::Hands)]
    pub target: TuneTarget,

    /// For `--target hands`: scores or folders of scores whose source says which hand
    /// plays what — two-staff MusicXML, or piano MIDI with a track per hand.
    #[arg(long, num_args = 1..)]
    pub scores: Vec<PathBuf>,

    /// For `--target hands`: the most scores to learn from. Every one of them is
    /// fingered again for every setting tried, so this is what a generation costs;
    /// more of them generalises better and moves slower.
    #[arg(long, default_value_t = 5000)]
    pub limit: usize,

    /// For `--target hands`: keep only music where at least this share of notes falls
    /// inside the other hand's usual range. Nought, the default, learns from the
    /// dataset as it is, which is what gives the best accuracy over it; raise it to
    /// weight the search towards music where the hands share the keyboard.
    #[arg(long, default_value_t = 0.0)]
    pub min_shared: f64,

    /// For `--target fingers`: the corpus to learn from.
    #[arg(long, default_value = CORPUS_DIR)]
    pub corpus: PathBuf,

    /// Stop after this many hours. Without it, runs until stopped.
    #[arg(long)]
    pub hours: Option<f64>,

    /// Settings tried at once. One per core by default.
    #[arg(long)]
    pub threads: Option<usize>,

    /// Where the search keeps its state and its log.
    #[arg(long, default_value = "out/tune")]
    pub out: PathBuf,

    /// Where better weights are written, for `--tuned` and for review.
    #[arg(long, default_value = "models/weights.json")]
    pub weights: PathBuf,

    /// Optional: pieces no setting may do worse on than the defaults. Only for
    /// holding on to behaviour on music you have checked by hand — it refuses
    /// improvements, so it costs dataset accuracy.
    /// Scores for `--target hands`, a corpus directory for `--target fingers`.
    #[arg(long, num_args = 1..)]
    pub guard: Vec<PathBuf>,

    /// Instead of searching, measure these weights against the engine's defaults on
    /// every example given — for trying a setting on music it was not tuned on.
    #[arg(long)]
    pub measure: Option<PathBuf>,
}

/// What `opennote tune` searches.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum TuneTarget {
    /// Which hand plays each note.
    Hands,
    /// Which finger plays each note.
    Fingers,
}

/// Search the engine's weights until stopped or out of time.
pub fn tune(args: TuneArgs) -> Result<()> {
    use on_train::tune::{run, Examples, TuneConfig};

    let examples = match args.target {
        TuneTarget::Hands => {
            if args.scores.is_empty() {
                bail!("give --scores: files or folders of two-staff MusicXML or two-track piano MIDI");
            }
            println!("Reading scores...");
            let mut last = std::time::Instant::now();
            // Measuring is about the pieces given, all of them.
            let min_shared = if args.measure.is_some() { 0.0 } else { args.min_shared };
            Examples::hands(&args.scores, args.limit, min_shared, |tried, kept| {
                if last.elapsed().as_secs() >= 10 {
                    last = std::time::Instant::now();
                    println!("  {tried} files read, {kept} usable");
                }
            })?
        }
        TuneTarget::Fingers => {
            let pieces = Corpus::at(&args.corpus)?.load()?;
            if pieces.is_empty() {
                bail!(
                    "the corpus at {} is empty. Add fingered music with `opennote corpus add`.",
                    args.corpus.display()
                );
            }
            Examples::fingers(pieces)
        }
    };
    if let Some(path) = &args.measure {
        let tuned = on_train::tune::Weights::load(path)?;
        let defaults = on_train::tune::Weights::default();
        let [a, b, c] = examples.sizes();
        println!("{} examples", a + b + c);
        println!("  defaults:  {:.2}%", examples.agreement(&defaults) * 100.0);
        println!("  {}:  {:.2}%", path.display(), examples.agreement(&tuned) * 100.0);
        return Ok(());
    }
    let guard = if args.guard.is_empty() {
        None
    } else {
        Some(match args.target {
            TuneTarget::Hands => Examples::hands(&args.guard, usize::MAX, 0.0, |_, _| {})?,
            TuneTarget::Fingers => {
                let mut pieces = Vec::new();
                for path in &args.guard {
                    pieces.extend(Corpus::at(path)?.load()?);
                }
                Examples::fingers(pieces)
            }
        })
    };
    let config = TuneConfig {
        guard,
        directory: args.out,
        weights: args.weights,
        hours: args.hours,
        threads: args.threads.unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(4, |n| n.get())
        }),
    };
    run(&examples, &config, |line| println!("{line}"))
}
