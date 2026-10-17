//! `opennote bench` and `opennote benchmark`: measure the fingering on a dataset, and on
//! OpenNote's own tiered benchmark into a scorecard.

use super::*;

/// Measure a model, and the rules it is built on, on OpenNote's own benchmark.
#[derive(Debug, Args)]
pub struct BenchmarkArgs {
    /// The benchmark's manifest: its tiers and the hash of every file in them.
    #[arg(long, default_value = "bench/v2/manifest.json")]
    pub manifest: PathBuf,

    /// Where to write the scorecard (JSON). Printed either way.
    #[arg(long)]
    pub out: Option<PathBuf>,

    /// How many times to resample for each interval.
    #[arg(long, default_value_t = 1000)]
    pub rounds: usize,

    /// Pieces fingered at once. One per core by default.
    #[arg(long)]
    pub threads: Option<usize>,

    /// Also measure the sealed tiers, which are kept back for releases.
    #[arg(long)]
    pub sealed: bool,

    /// Instead of measuring, write passages where the model disagrees with the rules or
    /// the people, for the training suite's blind judge (needs --model).
    #[arg(long)]
    pub excerpts: Option<PathBuf>,

    #[command(flatten)]
    pub weights: ModelArgs,
}

pub fn benchmark(args: BenchmarkArgs) -> Result<()> {
    let manifest = on_train::bench::Manifest::load(&args.manifest)?;
    let options = args.weights.options()?;
    let prior = args.weights.prior()?;
    let threads = args.threads.unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()));
    on_fingering::set_inner_threads(false);
    let name =
        args.weights.model.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned());
    if let Some(path) = &args.excerpts {
        let Some(prior) = prior.as_deref() else {
            anyhow::bail!("--excerpts compares a model with the rules: give --model")
        };
        let excerpts =
            on_train::bench::excerpts(&manifest, &options, prior as &dyn on_fingering::FingeringPrior, 30)?;
        std::fs::write(path, on_train::bench::excerpts_json(name, &excerpts)?)?;
        println!("{} passages to judge -> {}", excerpts.len(), path.display());
        return Ok(());
    }
    let card = on_train::bench::run(
        &manifest,
        &options,
        prior.as_deref().map(|p| p as &dyn on_fingering::FingeringPrior),
        name,
        args.rounds,
        threads,
        args.sealed,
        |line| eprintln!("{line}"),
    )?;
    for tier in &card.tiers {
        println!(
            "{} - {}: {} pieces from {} groups, {} notes",
            tier.id, tier.title, tier.pieces, tier.groups, tier.notes
        );
        println!(
            "  rules alone  general {:.1}%  recombined {:.1}%  4-gram {:.1}%",
            tier.rules.general, tier.rules.recombined, tier.rules.ngram
        );
        if let (Some(model), Some(gain)) = (&tier.model, &tier.gain) {
            println!(
                "  with model   general {:.1}%  recombined {:.1}%  4-gram {:.1}%",
                model.general, model.recombined, model.ngram
            );
            for (label, span) in
                [("general", gain.general), ("4-gram", gain.ngram), ("recombined", gain.recombined)]
            {
                println!(
                    "    {label:<10} {:+.1} points ({:+.1} to {:+.1}){}",
                    span.point,
                    span.low,
                    span.high,
                    if span.real { ", real" } else { "" }
                );
            }
        }
        if let Some(human) = &tier.human {
            println!(
                "  pianists against one another: general {:.1}%  recombined {:.1}%  4-gram {:.1}%",
                human.general, human.recombined, human.ngram
            );
        }
        let weakest: Vec<String> = tier.weakest.iter().take(5).map(|(n, r)| format!("{n} {r:.0}%")).collect();
        println!("  weakest: {}", weakest.join(", "));
        let line = |label: &str, c: &on_train::bench::Comfort| {
            println!(
                "  comfort, {label:<6} unplayable {} ({:.3}%)  moves {:.1}  travel {:.1} mm  stretched {:.1}%  under {:.1}  over {:.1}  thumbless {:.2}  spread {:.2}/{:.2}  4-5 on black {:.1}%",
                c.unplayable_count, c.unplayable, c.hand_moves, c.travel_mm, c.stretched, c.thumb_unders, c.finger_overs, c.thumbless,
                c.step_spread, c.chord_spread, c.weak_on_black
            );
        };
        if let Some(p) = &tier.probes {
            println!("  consistency: {:.1}% unchanged an octave away, {:.1}% under 10 ms of timing wobble ({} pieces)",
                     p.transposed * 100.0, p.jittered * 100.0, p.pieces);
        }
        println!("  speed: {:.0} ms per 1000 notes on one core", tier.ms_per_1000_notes);
        line("people", &tier.comfort_human);
        line("rules", &tier.comfort_rules);
        if let Some(c) = &tier.comfort_model {
            line("model", c);
        }
    }
    let h = &card.headline;
    match (h.score, &h.gated) {
        (Some(score), _) => println!("\nOpenNote Score ({}): {score:.1}", h.label),
        (None, Some(why)) => println!("\nOpenNote Score ({}): GATED - {why}", h.label),
        (None, None) => println!("\nOpenNote Score ({}): nothing to score", h.label),
    }
    for (name, value, weight) in &h.components {
        println!("  {name:<32} {:.3}  (weight {:.2})", value, weight);
    }
    if let Some(out) = &args.out {
        std::fs::write(out, card.to_json()?).with_context(|| format!("writing {}", out.display()))?;
        println!("Scorecard written to {}.", out.display());
    }
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
        let wanted: std::collections::BTreeSet<&str> =
            text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')).collect();
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
    if prior.is_some() {
        let mut plain = options.clone();
        plain.prior_scale = 0.0;
        let rules = on_train::eval::per_piece(&pieces, &plain, None);
        let gap = on_train::eval::paired(&measured, &rules, args.rounds);
        println!(
            "Against the rules alone, on the same pieces: {:+.1} points (95% interval {:+.1} to {:+.1}){}.",
            gap.point * 100.0,
            gap.low * 100.0,
            gap.high * 100.0,
            if gap.low > 0.0 {
                ", a real improvement"
            } else if gap.high < 0.0 {
                ", a real loss"
            } else {
                ", not distinguishable from no change"
            }
        );
    }
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
