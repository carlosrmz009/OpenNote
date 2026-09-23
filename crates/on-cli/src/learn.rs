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

        /// Refuse to add the piece if fewer than this share of its notes could be given
        /// a finger, from 0 to 1.
        ///
        /// This is the check that the calibration is right. A keyboard found in the frame
        /// is assumed to be a full 88 keys, because counting keys off a picture is not
        /// reliable; if it is really a shorter one, every fingertip lands keys away from
        /// the notes that sounded and almost nothing is recovered. Refusing then, rather
        /// than writing a handful of fingerings that are all shifted, is what keeps a
        /// wrong calibration out of the corpus when nobody is watching.
        #[arg(long, default_value_t = 0.5)]
        min_coverage: f64,

        /// Measure how much could be read, and write nothing. How the harvester tries
        /// each keyboard size and keeps the one the notes agree with.
        #[arg(long)]
        dry_run: bool,
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
    #[arg(long, default_value_t = 0.2)]
    pub held_out: f32,

    #[command(flatten)]
    pub weights: ModelArgs,
}

/// Teach the search to finger like the pianists in the corpus.
#[derive(Debug, Args)]
pub struct RerankArgs {
    /// The corpus directory.
    #[arg(long, default_value = CORPUS_DIR)]
    pub corpus: PathBuf,

    /// Where to write the model. Used with `--model`, like any other.
    #[arg(long, short, default_value = "models/rerank.json")]
    pub out: PathBuf,

    /// Passes over the training pieces.
    #[arg(long, default_value_t = 6)]
    pub epochs: usize,

    /// How far one disagreement moves a weight, in the cost function's own units.
    #[arg(long, default_value_t = 0.2)]
    pub rate: f32,

    /// Fingers read with less confidence than this are neither learned from nor
    /// measured against. 0.4 is where the harvested readings were measured 98.8% right.
    #[arg(long, default_value_t = 0.4)]
    pub min_confidence: f32,

    /// The fraction of pieces held back to measure on.
    #[arg(long, default_value_t = 0.2)]
    pub held_out: f32,

    /// Pieces each pass learns from, drawn afresh; 0 for all of them.
    #[arg(long, default_value_t = 0)]
    pub per_pass: usize,

    /// Pieces fingered at once. One per core by default.
    #[arg(long)]
    pub threads: Option<usize>,

    #[command(flatten)]
    pub weights: ModelArgs,
}

/// Run `opennote rerank`.
pub fn rerank(args: RerankArgs) -> Result<()> {
    let pieces = Corpus::at(&args.corpus)?.load()?;
    if pieces.is_empty() {
        bail!("the corpus at {} is empty", args.corpus.display());
    }
    let options = args.weights.options()?;
    let config = on_train::rerank::Config {
        epochs: args.epochs,
        rate: args.rate,
        floor: args.min_confidence,
        held_out: args.held_out,
        per_epoch: args.per_pass,
        threads: args
            .threads
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get())),
    };
    let report = on_train::rerank::train(&pieces, &options, &config, |line| println!("{line}"));
    if report.best == 0 {
        println!("\nNo pass beat the rules alone on the held-back pieces; nothing written.");
        return Ok(());
    }
    let kept = &report.passes[report.best - 1];
    report.model.save(&args.out)?;
    println!(
        "\nKept pass {}: {:.1}% against {:.1}% for the rules alone, on {} held-back pieces.",
        report.best,
        kept.general * 100.0,
        report.before.general * 100.0,
        report.sizes.1
    );
    println!("Written to {}. Use it with --model {}.", args.out.display(), args.out.display());
    Ok(())
}

/// Measure the fingering against a dataset, without learning anything from it.
#[derive(Debug, Args)]
pub struct BenchArgs {
    /// The dataset: a folder of PIG `_fingering.txt` files, or of MusicXML carrying
    /// printed fingerings.
    pub data: PathBuf,

    /// Only the pieces named in this file, one per line, `#` for a comment. This is how
    /// a published split is reproduced: the literature reports its numbers on a
    /// particular division of PIG, and a number measured on a different division is not
    /// comparable to it however carefully it was measured.
    #[arg(long)]
    pub pieces: Option<PathBuf>,

    /// How many times to resample the pieces for the confidence interval.
    #[arg(long, default_value_t = 1000)]
    pub rounds: usize,

    #[command(flatten)]
    pub weights: ModelArgs,
}

/// Run `opennote bench`.
///
/// Reads the dataset and measures against it. It never writes to the corpus, and that is
/// the point rather than an implementation detail: PIG is free but licensed for
/// nonprofit academic use, so the engine may be *measured* against it and must never be
/// *trained* on it. `opennote corpus add` is the command that stores things; this one
/// has no path that can.
pub fn bench(args: BenchArgs) -> Result<()> {
    if !args.data.exists() {
        bail!("{} does not exist", args.data.display());
    }
    let (mut pieces, skipped) = on_train::corpus::read(&args.data)?;
    if pieces.is_empty() {
        bail!(
            "found no fingering data under {}.\n\
             This reads PIG `_fingering.txt` files and MusicXML that already has \
             <fingering> marks on its notes.",
            args.data.display()
        );
    }

    if let Some(list) = &args.pieces {
        let text = std::fs::read_to_string(list)
            .with_context(|| format!("reading the piece list at {}", list.display()))?;
        let wanted: std::collections::BTreeSet<&str> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect();
        let before = pieces.len();
        pieces.retain(|piece| wanted.contains(piece.piece.as_str()));
        if pieces.is_empty() {
            bail!(
                "none of the {before} annotations under {} are of the {} pieces named in {}",
                args.data.display(),
                wanted.len(),
                list.display()
            );
        }
        println!(
            "{} of {} annotations are of the {} pieces named in {}.",
            pieces.len(),
            before,
            wanted.len(),
            list.display()
        );
    }

    if !skipped.is_empty() {
        println!("Could not read {} file(s); the first few:", skipped.len());
        for line in skipped.iter().take(5) {
            println!("  {line}");
        }
    }

    let options = args.weights.options()?;
    let prior = args.weights.prior()?;
    let measured = on_train::eval::per_piece(
        &pieces,
        &options,
        prior.as_deref().map(|p| p as &dyn on_fingering::FingeringPrior),
    );
    if measured.is_empty() {
        bail!("nothing could be measured: no piece had a note both fingered and solved");
    }
    let confidence = on_train::eval::confidence(&measured, args.rounds);

    println!("\n{confidence}");
    println!(
        "\nTwo pianists asked to finger the same music agree about 71% of the time, so \n\
         that is what a perfect score looks like, not 100%."
    );
    println!(
        "\nNothing was written. This command cannot add to the corpus, because a dataset \n\
         that may be measured against is not always one that may be learned from."
    );
    Ok(())
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
            min_coverage,
            dry_run,
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
            let share = found as f64 / total.max(1) as f64;
            // How sure the readings were, on average. Coverage alone cannot tell a right
            // finger from a wrong one: it only asks whether *a* finger was near each note,
            // and a keyboard mapped a size too small still puts one near most of them —
            // the neighbouring one. A mapping that is right puts the fingertips dead on
            // the keys that sounded, which is what the confidence measures.
            let read: Vec<f32> = extracted.notes.iter().filter_map(|n| n.confidence).collect();
            let belief = read.iter().sum::<f32>() / read.len().max(1) as f32;
            // On a line of its own and in a fixed form, for a program to read: this is how
            // `tools/harvest.py` compares one calibration against another.
            println!("coverage={share:.4} found={found} total={total} confidence={belief:.4}");
            if dry_run {
                return Ok(());
            }
            if found == 0 {
                bail!(
                    "no fingerings could be read from {}.\n\
                     The usual cause is a calibration that does not match this video, or \
                     a score that is not the performance in it. Check that the two start \
                     together — `--offset` shifts the video's clock against the score's.",
                    landmarks.display()
                );
            }

            if share < min_coverage {
                bail!(
                    "only {found} of {total} notes ({:.0}%) could be given a finger, below the \
                     {:.0}% this needs, so nothing was added.\n\
                     The usual cause is a keyboard that is not the size the calibration \
                     assumed — every note then lands keys away from the finger that played it. \
                     Pass --lowest and --highest to `watch_pianist.py calibrate`, or let \
                     `tools/harvest.py` try the standard sizes. If the calibration is right \
                     and the hands really do leave the frame, lower --min-coverage.",
                    share * 100.0,
                    min_coverage * 100.0
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

/// A corpus with every finger a machine was less than `floor` sure of taken out, saying
/// how many went so a floor that throws away most of the data is not a silent one.
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

    /// For `--target hands` and `--target play`: scores or folders of scores whose
    /// source says which hand plays what — two-staff MusicXML, or piano MIDI with a
    /// track per hand.
    #[arg(long, num_args = 1..)]
    pub scores: Vec<PathBuf>,

    /// The most scores to keep. Nought, the default, keeps every one there is: memory
    /// is the only limit, and fifteen thousand of them is about a gigabyte. What a
    /// generation costs is `--batch`, not this.
    #[arg(long, default_value_t = 0)]
    pub limit: usize,

    /// How many scores each generation is judged on, drawn afresh every generation.
    /// This is what a generation costs. Nought uses every one kept, which is slower by
    /// however many times more there are and buys nothing: a few hundred drawn again
    /// each time says as much, and over a night the whole dataset is seen many times.
    #[arg(long, default_value_t = 600)]
    pub batch: usize,

    /// For `--target taught`: how many exercises to generate. They cost nothing to
    /// make and there is no dataset to download, so this is only how long a generation
    /// takes against how well it generalises.
    #[arg(long, default_value_t = 4000)]
    pub exercises: usize,

    /// For `--target play`: cut each score to this many notes, from the middle.
    /// Fingering a score costs about three hundred times what deciding its hands does,
    /// and a passage from each of six thousand pieces says more per second than the
    /// whole of six hundred. Nought keeps whole pieces.
    #[arg(long, default_value_t = 200)]
    pub notes: usize,

    /// For `--target hands`: keep only music where at least this share of notes falls
    /// inside the other hand's usual range. Nought, the default, learns from the
    /// dataset as it is, which is what gives the best accuracy over it; raise it to
    /// weight the search towards music where the hands share the keyboard.
    #[arg(long, default_value_t = 0.0)]
    pub min_shared: f64,

    /// For `--target fingers`: the corpus to learn from.
    #[arg(long, default_value = CORPUS_DIR)]
    pub corpus: PathBuf,

    /// Leave out any finger a machine read with less than this confidence, from 0 to 1.
    /// A finger a person wrote is always kept. Nought, the default, keeps everything.
    ///
    /// Only matters for fingerings read out of video, and the right value is not known
    /// in advance: raise it until the benchmark stops improving, which is the point at
    /// which the readings being dropped were doing more harm than good.
    #[arg(long, default_value_t = 0.0)]
    pub min_confidence: f32,

    /// Stop after this many hours. Without it, runs until stopped.
    #[arg(long)]
    pub hours: Option<f64>,

    /// Settings tried at once. One per core by default.
    #[arg(long)]
    pub threads: Option<usize>,

    /// Generations without finding anything worth keeping before this target is called
    /// finished and the command returns. 0 runs on regardless. Counted in generations
    /// rather than hours so that it scales with what a target costs to search.
    #[arg(long, default_value_t = on_train::tune::DEFAULT_GIVE_UP)]
    pub give_up: u64,

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

    /// Instead of searching, say how long has been searched altogether and what it
    /// found. The hours add up over every run there has ever been.
    #[arg(long)]
    pub report: bool,

    /// Instead of searching, measure these weights against the engine's defaults on
    /// every example given — for trying a setting on music it was not tuned on.
    #[arg(long)]
    pub measure: Option<PathBuf>,
}

/// What `opennote tune` searches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum TuneTarget {
    /// Which hand plays each note.
    Hands,
    /// Which finger plays each note, against fingerings a pianist wrote.
    Fingers,
    /// Which finger plays each note, against what a hand can actually do.
    Play,
    /// Which finger plays each note, against the fingerings that are taught.
    Taught,
}

/// Search the engine's weights until stopped or out of time.
pub fn tune(args: TuneArgs) -> Result<()> {
    use on_train::tune::{report, run, Examples, TuneConfig};

    if args.report {
        for line in report(&args.out)? {
            println!("{line}");
        }
        return Ok(());
    }

    let playing = args.target == TuneTarget::Play;
    let load = on_train::tune::Load {
        limit: args.limit,
        min_shared: args.min_shared,
        // Whole pieces for the hand target, where a score costs almost nothing to work
        // through, and a passage for the playing one, where it costs three hundred times
        // as much.
        notes: if playing { args.notes } else { 0 },
        truth_only: args.target == TuneTarget::Hands,
    };
    let examples = match args.target {
        TuneTarget::Hands | TuneTarget::Play => {
            if args.scores.is_empty() {
                bail!("give --scores: files or folders of piano MusicXML or MIDI");
            }
            println!("Reading scores...");
            let mut last = std::time::Instant::now();
            let read = Examples::scores(&args.scores, &load, |tried, kept| {
                if last.elapsed().as_secs() >= 10 {
                    last = std::time::Instant::now();
                    println!("  {tried} files read, {kept} usable");
                }
            })?;
            if playing { read.playing() } else { read }
        }
        TuneTarget::Taught => {
            println!("Generating {} exercises...", args.exercises);
            Examples::taught(args.exercises)
        }
        TuneTarget::Fingers => {
            let pieces = trusted(Corpus::at(&args.corpus)?.load()?, args.min_confidence);
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
            TuneTarget::Hands => Examples::scores(&args.guard, &load, |_, _| {})?,
            TuneTarget::Play => Examples::scores(&args.guard, &load, |_, _| {})?.playing(),
            TuneTarget::Taught => bail!("--guard makes no sense for --target taught"),
            TuneTarget::Fingers => {
                let mut pieces = Vec::new();
                for path in &args.guard {
                    pieces.extend(Corpus::at(path)?.load()?);
                }
                Examples::fingers(pieces)
            }
        })
    };
    // The two fingering targets pull on the same knobs from opposite ends, so each is
    // told what the other says and may not spend it. Exercises cost nothing to make, so
    // the playing target always gets one; the playing measure needs scores, and is
    // capped because this is a gate asked on every promotion rather than something to
    // climb.
    const GATE_SCORES: usize = 2000;
    const GATE_EXERCISES: usize = 1500;
    let counterpart = match args.target {
        TuneTarget::Play => Some(Examples::taught(args.exercises.min(GATE_EXERCISES))),
        TuneTarget::Taught if !args.scores.is_empty() => {
            println!("Reading scores to watch what the fingering costs a hand...");
            let gate = on_train::tune::Load {
                limit: if args.limit == 0 { GATE_SCORES } else { args.limit.min(GATE_SCORES) },
                notes: args.notes,
                truth_only: false,
                ..load.clone()
            };
            Some(Examples::scores(&args.scores, &gate, |_, _| {})?.playing())
        }
        TuneTarget::Taught => {
            println!(
                "No --scores, so nothing is watching what these fingerings cost a hand.                  Pass the PDMX folder to gate on that too."
            );
            None
        }
        _ => None,
    };
    let config = TuneConfig {
        counterpart,
        guard,
        directory: args.out,
        weights: args.weights,
        hours: args.hours,
        batch: args.batch,
        give_up: args.give_up,
        threads: args.threads.unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(4, |n| n.get())
        }),
    };
    run(&examples, &config, |line| println!("{line}"))
}
