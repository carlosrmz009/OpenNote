//! `opennote tune`: searches the engine's weights for as long as it is left running,
//! resuming where it stopped.

use super::*;

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

    /// Threads to evaluate settings on. One per core by default; the results do not depend
    /// on it.
    #[arg(long)]
    pub threads: Option<usize>,

    /// How many settings to try each generation. The same on every machine, whatever its
    /// thread count, so a search can be repeated.
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u64).range(1..=256))]
    pub children: u64,

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
            if playing {
                read.playing()
            } else {
                read
            }
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
                "No --scores, so nothing is watching what these fingerings cost a hand. \
                 Pass the PDMX folder to gate on that too."
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
        children: args.children as usize,
        threads: args.threads.unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get())),
    };
    run(&examples, &config, |line| println!("{line}"))
}
