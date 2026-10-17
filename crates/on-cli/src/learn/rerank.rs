use super::*;

/// Teach the search to finger like the pianists in the corpus.
#[derive(Debug, Args)]
pub struct RerankArgs {
    /// The corpus directories: performances and written fingerings train their own
    /// styles, so both can be given.
    #[arg(long, num_args = 1.., default_value = CORPUS_DIR)]
    pub corpus: Vec<PathBuf>,

    /// A file saying who played each piece, one `piece<TAB>group` a line (extra columns
    /// are ignored): a channel, a pianist. Pieces are then held back a whole group at a
    /// time, measuring the model on players it has never seen.
    #[arg(long)]
    pub groups: Option<PathBuf>,

    /// Hold back exactly these groups (from `--groups`), comma-separated, rather than a
    /// share of them chosen by hash.
    #[arg(long, value_delimiter = ',')]
    pub test_groups: Vec<String>,

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
    #[arg(long, default_value_t = 0.2, value_parser = crate::between(0.0f32, 0.9))]
    pub held_out: f32,

    /// Pieces each pass learns from, drawn afresh; 0 for all of them.
    #[arg(long, default_value_t = 0)]
    pub per_pass: usize,

    /// Pieces fingered at once. One per core by default.
    #[arg(long)]
    pub threads: Option<usize>,

    /// Instead of training, say what the model at `--out` learned: its strongest
    /// weights, in words, the fingerings it made cheaper first.
    #[arg(long)]
    pub explain: bool,

    /// Instead of training, measure the model at `--out` against the rules alone on the
    /// pieces a training run with these settings holds back, paired.
    #[arg(long)]
    pub measure: bool,

    /// Learn one model for everything, rather than a style per source. The sources still
    /// decide what is held back and how results are reported.
    #[arg(long)]
    pub unified: bool,

    /// How much less a piece teaches where its fingering is harder on the hand than the
    /// search's own — more position changes, more stretch. Zero learns from every
    /// fingering alike; higher leans towards comfortable, casual playing.
    #[arg(long, default_value_t = 0.0)]
    pub comfort: f32,

    #[command(flatten)]
    pub weights: ModelArgs,
}

fn print_comfort(before: &on_fingering::Playability, after: &on_fingering::Playability) {
    let line = |label: &str, p: &on_fingering::Playability| {
        println!(
            "    {label:<12} unplayable {:.2}%, hand moves {:.1} per 100 notes, travel {:.1} mm a note, \
             stretched pairs {:.1}%",
            p.unplayable_rate() * 100.0,
            p.change_rate() * 100.0,
            p.travel_mm / p.transitions.max(1) as f64,
            p.stretched_rate() * 100.0
        );
    };
    line("rules alone", before);
    line("with model", after);
}

pub fn rerank(args: RerankArgs) -> Result<()> {
    if args.explain {
        let model = NgramPrior::load(&args.out)
            .with_context(|| format!("reading the model at {}", args.out.display()))?;
        let Some(learned) = model.learned else {
            bail!("{} has no learned weights in it", args.out.display());
        };
        let mut weights: Vec<(u64, f32)> = learned.weights.into_iter().collect();
        weights.sort_by(|a, b| a.1.total_cmp(&b.1));
        println!("{} learned weights. Made cheaper (preferred):", weights.len());
        for (fact, weight) in weights.iter().take(25) {
            println!("  {weight:+7.2}  {}", on_fingering::learned::describe(*fact));
        }
        println!("\nMade dearer (avoided):");
        for (fact, weight) in weights.iter().rev().take(25) {
            println!("  {weight:+7.2}  {}", on_fingering::learned::describe(*fact));
        }
        return Ok(());
    }
    let mut pieces = Vec::new();
    for corpus in &args.corpus {
        pieces.extend(Corpus::at(corpus)?.load()?);
    }
    if pieces.is_empty() {
        bail!("the corpora given are empty");
    }
    let mut groups = std::collections::HashMap::new();
    if let Some(path) = &args.groups {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading the groups at {}", path.display()))?;
        for line in text.lines() {
            let mut fields = line.trim_start_matches('\u{feff}').split('\t');
            if let (Some(piece), Some(group)) = (fields.next(), fields.next()) {
                groups.insert(piece.trim().to_string(), group.trim().to_string());
            }
        }
    }
    let options = args.weights.options()?;
    let config = on_train::rerank::Config {
        epochs: args.epochs,
        rate: args.rate,
        floor: args.min_confidence,
        held_out: args.held_out,
        per_epoch: args.per_pass,
        threads: args.threads.unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get())),
        groups,
        test_groups: args.test_groups.clone(),
        unified: args.unified,
        comfort: args.comfort,
    };
    if args.measure {
        let model = NgramPrior::load(&args.out)
            .with_context(|| format!("reading the model at {}", args.out.display()))?;
        let Some(learned) = model.learned else {
            bail!("{} has no learned weights in it", args.out.display());
        };
        for (style, before, after, gap, hand_before, hand_after) in
            on_train::rerank::measure_model(&pieces, &options, &config, &learned.weights)
        {
            println!(
                "{style:?}: {:.1}% against {:.1}% for the rules alone; paired {:+.1} points \
                 (95% interval {:+.1} to {:+.1}){}",
                after.general * 100.0,
                before.general * 100.0,
                gap.point * 100.0,
                gap.low * 100.0,
                gap.high * 100.0,
                if gap.low > 0.0 { ", a real improvement" } else { ", not distinguishable from no change" }
            );
            print_comfort(&hand_before, &hand_after);
        }
        return Ok(());
    }
    let report = on_train::rerank::train(&pieces, &options, &config, |line| println!("{line}"));
    if report.best == 0 {
        println!("\nNo pass beat the rules alone on the held-back pieces; nothing written.");
        return Ok(());
    }
    let kept = &report.passes[report.best - 1];
    report.model.save(&args.out)?;
    println!("\nKept pass {}, on {} held-back pieces:", report.best, report.sizes.1);
    for ((style, after), (_, before)) in kept.iter().zip(&report.before) {
        println!(
            "  {style:?}: {:.1}% against {:.1}% for the rules alone",
            after.general * 100.0,
            before.general * 100.0
        );
        if let Some((_, hand_before, hand_after)) = report.comfort.iter().find(|(s, _, _)| s == style) {
            print_comfort(hand_before, hand_after);
        }
        if let Some((_, gap)) = report.gains.iter().find(|(s, _)| s == style) {
            println!(
                "    paired: {:+.1} points (95% interval {:+.1} to {:+.1}){}",
                gap.point * 100.0,
                gap.low * 100.0,
                gap.high * 100.0,
                if gap.low > 0.0 { ", a real improvement" } else { ", not distinguishable from no change" }
            );
        }
    }
    println!("Written to {}. Use it with --model {}.", args.out.display(), args.out.display());
    Ok(())
}
