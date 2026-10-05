use glam::Vec3;
use on_fingering::FingeringOptions;
use on_hand::keyboard::{is_black, Keyboard, BLACK_KEY_DIP, BLACK_KEY_HEIGHT, KEY_DIP};
use on_hand::skeleton::dof;
use on_hand::{Hand, HandPose};
use on_score::hands::HandAssignment;
use on_viz::timeline::{pose_both, HandAnimator, Timeline};

const FPS: f64 = 60.0;

const FINE: f64 = 240.0;

const HAND_SPEED_MM_PER_SECOND: f64 = on_fingering::playability::HAND_SPEED_MM_PER_SECOND;

const JOLT_MM: f32 = 5.0;

const BREAK_MM: f32 = 1.0;

const TWITCH_SECONDS: f64 = 0.08;

const TWITCH_MM: f32 = 1.0;

fn raw() -> bool {
    std::env::var("ON_RAW").is_ok()
}

fn poses(animators: &[HandAnimator], at: f64) -> [HandPose; 2] {
    if raw() {
        [animators[0].pose_at(at), animators[1].pose_at(at)]
    } else {
        pose_both(animators, at)
    }
}

fn drawn_at(animators: &[HandAnimator], at: f64) -> Vec<Vec3> {
    let posed = poses(animators, at);
    let mut out = animators[0].joints(&posed[0]);
    out.extend(animators[1].joints(&posed[1]));
    out
}

fn percentiles(mut values: Vec<f32>) -> String {
    if values.is_empty() {
        return "-".into();
    }
    values.sort_by(f32::total_cmp);
    let at = |p: f64| values[((values.len() - 1) as f64 * p) as usize];
    format!("p50 {:8.0}  p95 {:8.0}  p99 {:8.0}  max {:8.0}", at(0.5), at(0.95), at(0.99), values[values.len() - 1])
}

fn twitches(track: &[Vec3]) -> usize {
    let window = (TWITCH_SECONDS * FINE).round() as usize;
    let mut count = 0;
    for axis in 0..3 {
        let x: Vec<f32> = track.iter().map(|p| p[axis]).collect();
        let mut turns: Vec<usize> = Vec::new();
        for i in 1..x.len().saturating_sub(1) {
            let (a, b) = (x[i] - x[i - 1], x[i + 1] - x[i]);
            if a * b < 0.0 {
                turns.push(i);
            }
        }
        for pair in turns.windows(3) {
            let (p, q, r) = (pair[0], pair[1], pair[2]);
            if r - p <= window && (x[q] - x[p]).abs() > TWITCH_MM && (x[r] - x[q]).abs() > TWITCH_MM {
                count += 1;
            }
        }
    }
    count
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| {
        eprintln!("usage: jitter <score>   (ON_MODEL=<model.json> to finger with a model, as the app does)");
        std::process::exit(2);
    });

    let loading = std::time::Instant::now();
    let mut document = on_score::Document::open(std::path::Path::new(&path))?;
    let options = FingeringOptions::default();
    on_score::assign_hands(
        document.score_mut(),
        &HandAssignment { profile: options.profile.clone(), ..Default::default() },
    );
    let score = document.score();
    let prior = match std::env::var("ON_MODEL") {
        Ok(model) => Some(on_fingering::NgramPrior::load(std::path::Path::new(&model))?),
        Err(_) => None,
    };
    let solution = on_fingering::finger_score_with_prior(
        score,
        &options,
        prior.as_ref().map(|p| p as &dyn on_fingering::FingeringPrior),
    );
    let timeline = Timeline::build_for(score, &solution.fingerings, &options.profile);
    let animators = timeline.animators(&options.profile, on_fingering::biomech::BiomechWeights::default());
    let load_seconds = loading.elapsed().as_secs_f64();

    let step = 1.0 / FPS;
    let budget = (HAND_SPEED_MM_PER_SECOND * step) as f32;

    let mut previous: Option<[Vec<Vec3>; 2]> = None;
    let mut corrections: Vec<[Vec<Vec3>; 2]> = Vec::new();
    let mut jolts: Vec<(f64, Hand, f32)> = Vec::new();
    let mut breaks: Vec<(f64, f32)> = Vec::new();
    let mut jumps: Vec<(f64, Hand, f32)> = Vec::new();
    let mut every: Vec<f32> = Vec::new();

    let mut at = 0.0;
    while at <= timeline.duration {
        let posed = poses(&animators, at);
        let now = [
            animators[Hand::Left as usize].joints(&posed[0]),
            animators[Hand::Right as usize].joints(&posed[1]),
        ];
        let alone = [
            animators[Hand::Left as usize].joints(&animators[Hand::Left as usize].pose_at(at)),
            animators[Hand::Right as usize].joints(&animators[Hand::Right as usize].pose_at(at)),
        ];
        let correction = [0, 1].map(|side| now[side].iter().zip(&alone[side]).map(|(a, b)| *a - *b).collect::<Vec<_>>());
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
        if let Some(before) = &previous {
            for hand in Hand::ALL {
                let side = hand as usize;
                let moved = before[side].iter().zip(&now[side]).map(|(was, is)| was.distance(*is)).fold(0.0f32, f32::max);
                every.push(moved);
                if moved > budget {
                    jumps.push((at, hand, moved));
                }
            }
        }
        previous = Some(now);
        at += step;
    }

    let fine = 1.0 / FINE;
    let samples = (timeline.duration / fine) as usize + 1;
    let mut tracks: [[Vec<Vec3>; 6]; 2] = Default::default();
    let mut angles: [Vec<[f32; on_hand::skeleton::DOF]>; 2] = Default::default();
    let mut contact: Vec<f32> = Vec::new();
    let keyboard = Keyboard::new();
    for n in 0..samples {
        let t = n as f64 * fine;
        let posed = poses(&animators, t);
        let joints = [animators[0].joints(&posed[0]), animators[1].joints(&posed[1])];
        for side in 0..2 {
            for (k, index) in [0usize, 4, 8, 12, 16, 20].into_iter().enumerate() {
                tracks[side][k].push(joints[side][index]);
            }
            angles[side].push(posed[side].q);
        }
        let keys = timeline.key_depression(t);
        for note in timeline.notes.iter().filter(|note| note.sounds_at(t)) {
            let Some(finger) = note.finger else { continue };
            let depth = keys.depth_of(note.midi);
            if depth < 0.5 {
                continue;
            }
            let tip = joints[note.hand as usize][1 + 4 * finger.index() + 3];
            let (surface, dip) = if is_black(note.midi) { (BLACK_KEY_HEIGHT, BLACK_KEY_DIP) } else { (0.0, KEY_DIP) };
            let half = if is_black(note.midi) { 13.7 / 2.0 } else { 23.5 / 2.0 };
            let across = ((tip.x - keyboard.centre_x(note.midi)).abs() - half).max(0.0);
            let down = (tip.z - (surface - depth * dip)).abs();
            contact.push((across * across + down * down).sqrt());
        }
    }

    println!("{path}");
    println!("  built in {load_seconds:.2} s{}", if prior.is_some() { " (fingered with the model)" } else { "" });
    every.sort_by(f32::total_cmp);
    let at_percentile = |p: f64| every[((every.len() - 1) as f64 * p) as usize];
    let (median, p99) = (at_percentile(0.5), at_percentile(0.99));
    println!("  {} frames at {FPS:.0} fps, a hand may cover {budget:.0} mm in one of them", every.len() / 2);
    println!("  worst joint movement per frame: median {median:.1} mm, 99th {p99:.1} mm, worst {:.1} mm", every[every.len() - 1]);
    println!("  frames where a joint outran a hand: {}", jumps.len());
    println!("  frames where moving the hands apart jolted one: {}", jolts.len());
    println!("  places a hand steps rather than moves: {}", breaks.len());
    for (time, size) in breaks.iter().take(8) {
        println!("    {time:9.4}s  {size:5.1} mm");
    }

    println!("  motion at {FINE:.0} Hz (speed mm/s, acceleration mm/s², jerk mm/s³):");
    for hand in Hand::ALL {
        let side = hand as usize;
        for (label, range) in [("wrist", 0..1), ("tips", 1..6)] {
            let (mut speed, mut accel, mut jerk) = (Vec::new(), Vec::new(), Vec::new());
            let mut twitch = 0;
            for k in range {
                let p = &tracks[side][k];
                for i in 1..p.len().saturating_sub(2) {
                    speed.push(((p[i + 1] - p[i - 1]) / (2.0 * fine as f32)).length());
                    accel.push(((p[i + 1] - 2.0 * p[i] + p[i - 1]) / (fine * fine) as f32).length());
                    jerk.push(((p[i + 2] - 3.0 * p[i + 1] + 3.0 * p[i] - p[i - 1]) / (fine * fine * fine) as f32).length());
                }
                twitch += twitches(p);
            }
            println!("    {hand:?} {label:5} twitches {twitch:5}");
            println!("      speed  {}", percentiles(speed));
            println!("      accel  {}", percentiles(accel));
            println!("      jerk   {}", percentiles(jerk));
        }
        let joint_speed = |index: usize| {
            angles[side].windows(2).map(|w| (w[1][index] - w[0][index]).abs() / fine as f32).fold(0.0f32, f32::max)
        };
        let mcp = (0..4).map(|slot| joint_speed(dof::finger(slot) + dof::MCP_FLEX)).fold(0.0f32, f32::max);
        println!(
            "    {hand:?} peak angular speed: wrist flexion {:.1} rad/s, knuckle {:.1} rad/s",
            joint_speed(dof::WRIST_FLEXION),
            mcp
        );
    }
    println!("  fingertip to its pressed key (mm): {}", percentiles(contact).replace("p", " p"));

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
    Ok(())
}
