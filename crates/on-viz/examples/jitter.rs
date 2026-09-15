//! Does the animation move like a hand, frame to frame?
//!
//! Every other check here asks whether a posture is right at some instant. None of them
//! asks whether two neighbouring instants belong to the same movement, and a hand can be
//! in a perfectly good posture on one frame and a perfectly good one on the next while
//! having teleported between them. That reads as a flicker or a stutter, and it is
//! invisible to a check that looks at one frame at a time.
//!
//! The measure is a speed limit. `on-fingering`'s playability model puts a hand at about
//! 2.6 metres a second flat out, which at sixty frames a second is forty-odd millimetres
//! between frames. A joint that moves further than that has not moved, it has jumped.
use on_fingering::biomech::BiomechWeights;
use on_fingering::FingeringOptions;
use on_hand::Hand;
use on_score::hands::HandAssignment;
use on_viz::timeline::{pose_both, Timeline};

/// Frames a second, matching what the renderer draws.
const FPS: f64 = 60.0;

/// How fast a hand can be moved, in millimetres per second.
///
/// `on_fingering::playability`'s figure, measured from how long a pianist takes to cross
/// two octaves. Anything quicker than this is not a hand moving.
const HAND_SPEED_MM_PER_SECOND: f64 = on_fingering::playability::HAND_SPEED_MM_PER_SECOND;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| {
        eprintln!("usage: jitter <score>");
        std::process::exit(2);
    });

    let mut document = on_score::Document::open(std::path::Path::new(&path))?;
    let options = FingeringOptions::default();
    on_score::assign_hands(
        document.score_mut(),
        &HandAssignment { profile: options.profile.clone(), ..Default::default() },
    );
    let score = document.score();
    let solution = on_fingering::finger_score_consensus(score, &options, None);
    let timeline = Timeline::build_for(score, &solution.fingerings, &options.profile);
    let animators = timeline.animators(&options.profile, BiomechWeights::default());

    let step = 1.0 / FPS;
    let budget = (HAND_SPEED_MM_PER_SECOND * step) as f32;

    // Worst joint movement between one frame and the next, per hand, over the piece.
    let mut previous: Option<[Vec<glam::Vec3>; 2]> = None;
    let mut jumps: Vec<(f64, Hand, f32)> = Vec::new();
    let mut every: Vec<f32> = Vec::new();

    let mut at = 0.0;
    while at <= timeline.duration {
        // `ON_RAW=1` skips the collision handling, to say whether a jump is the
        // interpolation between postures or the moving-apart laid on top of it.
        let poses = if std::env::var("ON_RAW").is_ok() {
            [
                animators[Hand::Left as usize].pose_at(at),
                animators[Hand::Right as usize].pose_at(at),
            ]
        } else {
            pose_both(&animators, at)
        };
        let now = [
            animators[Hand::Left as usize].joints(&poses[0]),
            animators[Hand::Right as usize].joints(&poses[1]),
        ];
        if let Some(before) = &previous {
            for hand in Hand::ALL {
                let side = hand as usize;
                let moved = before[side]
                    .iter()
                    .zip(&now[side])
                    .map(|(was, is)| was.distance(*is))
                    .fold(0.0f32, f32::max);
                every.push(moved);
                if moved > budget {
                    jumps.push((at, hand, moved));
                }
            }
        }
        previous = Some(now);
        at += step;
    }

    every.sort_by(f32::total_cmp);
    let at_percentile = |p: f64| every[((every.len() - 1) as f64 * p) as usize];

    println!("{path}");
    println!(
        "  {} frames at {FPS:.0} fps, a hand may cover {budget:.0} mm in one of them",
        every.len() / 2
    );
    println!(
        "  worst joint movement per frame: median {:.1} mm, 99th {:.1} mm, worst {:.1} mm",
        at_percentile(0.5),
        at_percentile(0.99),
        every[every.len() - 1]
    );
    println!("  frames where a joint outran a hand: {}", jumps.len());

    // One line per episode rather than one per frame: a stutter is several frames long
    // and listing each of them says nothing the first does not.
    let mut shown = 0;
    let mut last = f64::MIN;
    for (time, hand, moved) in &jumps {
        if time - last > 0.25 {
            if shown == 12 {
                println!("    ... and more");
                break;
            }
            println!(
                "    {time:7.2}s {hand:?} a joint moved {moved:6.1} mm in one frame \
                 ({:.0} mm/s)",
                f64::from(*moved) * FPS
            );
            shown += 1;
        }
        last = *time;
    }
    Ok(())
}
