//! `opennote corpus`: adds expert fingerings to the corpus, from PIG files, fingered
//! MusicXML or video of a pianist, and lists what it holds.

use super::*;

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
    /// Takes the four corners clicked in the hand-tracking tool and fits
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
    /// Needs three things: the hand landmarks, which the hand-tracking tool writes
    /// from the video; a calibration saying where the keyboard is in the frame, which
    /// the same tool writes once per camera position; and a MIDI or MusicXML file of
    /// the same performance, which says which keys went down and when.
    Watch {
        /// The landmark file.
        landmarks: PathBuf,

        /// A MIDI or MusicXML file of the same performance.
        #[arg(long)]
        score: PathBuf,

        /// The calibration written by `opennote corpus calibrate`.
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
                score.file_stem().and_then(|s| s.to_str()).unwrap_or("untitled").to_string()
            });
            let extracted = on_train::video::extract(&watched, &onsets, &homography, &title, &annotator);
            let (found, total) = on_train::video::coverage(&extracted, &onsets);
            let share = found as f64 / total.max(1) as f64;
            let read: Vec<f32> = extracted.notes.iter().filter_map(|n| n.confidence).collect();
            let belief = read.iter().sum::<f32>() / read.len().max(1) as f32;
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
                     Calibrate again with the keyboard's lowest and highest keys, or try \
                     the standard keyboard sizes. If the calibration is right \
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
                     Add some expert fingerings with `opennote corpus add <path>`: \
                     PIG `_fingering.txt` files, or MusicXML with fingerings written in.",
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
                let people: Vec<&str> = annotations.iter().map(|p| p.annotator.as_str()).collect();
                println!("  {name}  ({})", people.join(", "));
            }
            if grouped.len() > 40 {
                println!("  … and {} more", grouped.len() - 40);
            }
        }
    }
    Ok(())
}
