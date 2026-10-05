use on_fingering::biomech::BiomechWeights;
use on_fingering::FingeringOptions;
use on_hand::Hand;
use on_score::hands::HandAssignment;
use on_viz::timeline::{pose_both, Timeline};

const FPS: f64 = 60.0;

const HAND_SPEED_MM_PER_SECOND: f64 = on_fingering::playability::HAND_SPEED_MM_PER_SECOND;

const JOLT_MM: f32 = 5.0;

const BREAK_MM: f32 = 1.0;

fn drawn_at(animators: &[on_viz::timeline::HandAnimator], at: f64) -> Vec<glam::Vec3> {
    let posed = if std::env::var("ON_RAW").is_ok() {
        [animators[0].pose_at(at), animators[1].pose_at(at)]
    } else {
        pose_both(animators, at)
    };
    let mut out = animators[0].joints(&posed[0]);
    out.extend(animators[1].joints(&posed[1]));
    out
}

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

    let mut previous: Option<[Vec<glam::Vec3>; 2]> = None;
    let mut corrections: Vec<[Vec<glam::Vec3>; 2]> = Vec::new();
    let mut jolts: Vec<(f64, Hand, f32)> = Vec::new();
    let mut breaks: Vec<(f64, f32)> = Vec::new();
    let mut jumps: Vec<(f64, Hand, f32)> = Vec::new();
    let mut every: Vec<f32> = Vec::new();

    let mut at = 0.0;
    while at <= timeline.duration {
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
        let alone = [
            animators[Hand::Left as usize].joints(&animators[Hand::Left as usize].pose_at(at)),
            animators[Hand::Right as usize].joints(&animators[Hand::Right as usize].pose_at(at)),
        ];
        let correction = [0, 1].map(|side| {
            now[side].iter().zip(&alone[side]).map(|(a, b)| *a - *b).collect::<Vec<_>>()
        });
        if let [.., two_ago, one_ago] = corrections.as_slice() {
            for hand in Hand::ALL {
                let side = hand as usize;
                let jolt = (0..correction[side].len())
                    .map(|j| (correction[side][j] - 2.0 * one_ago[side][j] + two_ago[side][j]).length())
                    .fold(0.0f32, f32::max);
                if jolt > JOLT_MM {
                    jolts.push((at, hand, jolt));
                }
            }
        }
        if at > 0.0 {
            let change = |a: f64, b: f64| {
                let (x, y) = (drawn_at(&animators, a), drawn_at(&animators, b));
                x.iter().zip(&y).map(|(p, q)| p.distance(*q)).fold(0.0f32, f32::max)
            };
            let (mut lo, mut hi) = (at - step, at);
            if change(lo, hi) > BREAK_MM {
                for _ in 0..12 {
                    let mid = (lo + hi) / 2.0;
                    if change(lo, mid) >= change(mid, hi) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                let size = change(lo, hi);
                if size > BREAK_MM {
                    breaks.push((lo, size));
                }
            }
        }
        corrections.push(correction);
        if std::env::var("ON_MODE").is_ok() {
            let raw = [animators[0].pose_at(at), animators[1].pose_at(at)];
            let j = [animators[0].joints(&raw[0]), animators[1].joints(&raw[1])];
            let near = on_viz::timeline::nearest(&j[0], &j[1]) < 30.0;
            let f0 = animators[0].grip_at(at).is_none();
            let f1 = animators[1].grip_at(at).is_none();
            println!("MODE {} {}{}", if near { 1 } else { 0 }, i32::from(f0), i32::from(f1));
        }
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
    println!("  frames where moving the hands apart jolted one: {}", jolts.len());
    println!("  places a hand steps rather than moves: {}", breaks.len());
    for (time, size) in breaks.iter().take(8) {
        println!("    {time:9.4}s  {size:5.1} mm");
    }

    let mut shown = 0;
    let mut last = f64::MIN;
    for (time, hand, jolt) in &jolts {
        if time - last > 0.25 {
            if shown == 12 {
                println!("    ... and more");
                break;
            }
            println!("    {time:7.2}s {hand:?} jolted {jolt:5.1} mm");
            shown += 1;
        }
        last = *time;
    }
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
