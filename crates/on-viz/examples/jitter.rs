use glam::Vec3;
use on_fingering::FingeringOptions;
use on_hand::keyboard::{is_black, Keyboard, BLACK_KEY_DIP, BLACK_KEY_HEIGHT, KEY_DIP};
use on_hand::skeleton::dof;
use on_hand::{Hand, HandPose};
use on_score::hands::HandAssignment;
use on_viz::timeline::{pose_both_decided, Decisions, HandAnimator, Timeline};

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

static DECISIONS: std::sync::OnceLock<Decisions> = std::sync::OnceLock::new();

fn poses(animators: &[HandAnimator], at: f64) -> [HandPose; 2] {
    if raw() {
        [animators[0].pose_at(at), animators[1].pose_at(at)]
    } else {
        pose_both_decided(animators, DECISIONS.get().expect("decided"), at)
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

const STRAY_MM: f32 = 1.5;

fn key_under(keyboard: &Keyboard, tip: Vec3) -> Option<u8> {
    use on_hand::keyboard::{BLACK_KEY_FRONT_Y, MIDI_HIGHEST, MIDI_LOWEST, WHITE_KEY_LENGTH};
    if tip.y < 0.0 || tip.y > WHITE_KEY_LENGTH {
        return None;
    }
    let (mut near, mut high) = (MIDI_LOWEST, MIDI_HIGHEST);
    while near < high {
        let middle = near + (high - near) / 2;
        if keyboard.centre_x(middle) < tip.x {
            near = middle + 1;
        } else {
            high = middle;
        }
    }
    let around = near.saturating_sub(2).max(MIDI_LOWEST)..=near.saturating_add(2).min(MIDI_HIGHEST);
    let inside = |midi: &u8| {
        let (x0, x1, y0, y1) = keyboard.footprint(*midi);
        tip.x >= x0 && tip.x <= x1 && tip.y >= y0 && tip.y <= y1
    };
    if tip.y > BLACK_KEY_FRONT_Y {
        if let Some(black) = around.clone().filter(|m| is_black(*m)).find(inside) {
            return Some(black);
        }
    }
    around.filter(|m| !is_black(*m)).find(inside)
}

fn twitches(track: &[Vec3]) -> Vec<usize> {
    let window = (TWITCH_SECONDS * FINE).round() as usize;
    let mut found = Vec::new();
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
                found.push(q);
            }
        }
    }
    found.sort_unstable();
    found
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
    let _ = DECISIONS.set(Decisions::new(&animators, timeline.duration + 2.0));
    let load_seconds = loading.elapsed().as_secs_f64();

    if std::env::var("ON_PHRASES").is_ok() {
        let phrases = on_score::expression::phrases(score);
        let lengths: Vec<f64> = phrases.iter().map(|(a, b)| score.tempo.seconds_at(*b) - score.tempo.seconds_at(*a)).collect();
        let short = lengths.iter().filter(|l| **l < 2.0).count();
        println!("  {} phrases, {} shorter than 2 s, median {:.1} s", phrases.len(), short, {
            let mut l = lengths.clone();
            l.sort_by(f64::total_cmp);
            l.get(l.len() / 2).copied().unwrap_or(0.0)
        });
        return Ok(());
    }
    if std::env::var("ON_BENCH").is_ok() {
        let frames = 600;
        let started = std::time::Instant::now();
        for n in 0..frames {
            let at = timeline.duration * f64::from(n) / f64::from(frames);
            std::hint::black_box(poses(&animators, at));
        }
        let each = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
        let started = std::time::Instant::now();
        for n in 0..frames {
            let at = timeline.duration * f64::from(n) / f64::from(frames);
            for animator in &animators {
                for k in -6..=6 {
                    std::hint::black_box(animator.unpressed(at + f64::from(k) * 0.2));
                }
            }
        }
        let lean = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
        println!("  per frame: hands {each:.2} ms, lean {lean:.2} ms");
        return Ok(());
    }
    if let Ok(at) = std::env::var("ON_ANGLES") {
        let at: f64 = at.parse()?;
        let posed = poses(&animators, at);
        for side in 0..2 {
            let raw = animators[side].unpressed(at);
            let deg = |q: &HandPose| -> Vec<String> { (6..on_hand::skeleton::DOF).map(|i| format!("{:.0}", q.q[i].to_degrees())).collect() };
            println!("{:?} shown  thumb {:?} | index/middle/ring/little (mcp spread pip) {:?}", Hand::ALL[side], &deg(&posed[side])[..4], &deg(&posed[side])[4..]);
            println!("{:?} planned thumb {:?} | {:?}", Hand::ALL[side], &deg(&raw)[..4], &deg(&raw)[4..]);
        }
        return Ok(());
    }
    if let Ok(at) = std::env::var("ON_GRIPS") {
        let (from, to) = match at.split_once(',') {
            Some((a, b)) => (a.parse::<f64>()?, b.parse::<f64>()?),
            None => {
                let at: f64 = at.parse()?;
                (at - 0.4, at + 0.4)
            }
        };
        let at = 0.5 * (from + to);
        let reach = 0.5 * (to - from);
        for hand in Hand::ALL {
            for e in timeline.hand_grips(hand).iter().filter(|e| (e.time - at).abs() < reach) {
                println!("{hand:?} {:.3} until {:.3} {:?} struck {:?}", e.time, e.release, e.grip.keys, e.struck);
            }
        }
        for n in timeline.notes.iter().filter(|n| (n.start - at).abs() < reach && std::env::var("ON_NOTES").is_ok()) {
            println!("note {:?} {} {:?} {:.3}-{:.3} key {:.3}..{:.3}", n.hand, n.midi, n.finger, n.start, n.end, n.key_moves, n.key_rises);
        }
        return Ok(());
    }
    if let Ok(span) = std::env::var("ON_TIPS") {
        let (from, to) = span.split_once(',').expect("ON_TIPS=from,to");
        let (from, to): (f64, f64) = (from.parse()?, to.parse()?);
        let mut at = from;
        while at <= to {
            let posed = poses(&animators, at);
            for side in 0..2 {
                let joints = animators[side].joints(&posed[side]);
                let tips: Vec<String> = (0..5).map(|f| format!("{:5.1}", joints[1 + 4 * f + 3].z)).collect();
                print!("{at:8.3} {:?} wrist z {:5.1} flex {:5.1}  tips z {}   ", Hand::ALL[side], posed[side].q[dof::WRIST_Z], posed[side].q[dof::WRIST_FLEXION].to_degrees(), tips.join(" "));
            }
            println!();
            at += 0.02;
        }
        return Ok(());
    }
    if let Ok(span) = std::env::var("ON_TRACK") {
        let (from, to) = span.split_once(',').expect("ON_TRACK=from,to");
        let (from, to): (f64, f64) = (from.parse()?, to.parse()?);
        let mut at = from;
        while at <= to {
            let decided = poses(&animators, at);
            let line: Vec<String> = (0..2)
                .map(|side| {
                    let raw = animators[side].pose_at(at).wrist_position();
                    let shown = decided[side].wrist_position();
                    format!("{:7.1} {:6.1} {:5.1} | {:+5.1} {:+5.1} {:+5.1}", shown.x, shown.y, shown.z, shown.x - raw.x, shown.y - raw.y, shown.z - raw.z)
                })
                .collect();
            println!("{at:8.3}  L {}   R {}", line[0], line[1]);
            at += std::env::var("ON_STEP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0 / 60.0);
        }
        return Ok(());
    }
    if let Ok(probe) = std::env::var("ON_PROBE") {
        let at: f64 = probe.parse()?;
        for side in 0..2 {
            let (a, b) = (poses(&animators, at - 0.0005)[side], poses(&animators, at + 0.0005)[side]);
            let moved: Vec<String> = (0..on_hand::skeleton::DOF)
                .filter(|i| (a.q[*i] - b.q[*i]).abs() > 0.01)
                .map(|i| format!("{i}:{:.3}->{:.3}", a.q[i], b.q[i]))
                .collect();
            println!("{:?}: {}", Hand::ALL[side], moved.join("  "));
            let (a, b) = (animators[side].unpressed(at - 0.0005), animators[side].unpressed(at + 0.0005));
            let moved: Vec<String> = (0..on_hand::skeleton::DOF)
                .filter(|i| (a.q[*i] - b.q[*i]).abs() > 0.01)
                .map(|i| format!("{i}:{:.3}->{:.3}", a.q[i], b.q[i]))
                .collect();
            println!("  unpressed: {}", moved.join("  "));
        }
        return Ok(());
    }
    let step = 1.0 / FPS;
    let budget = (HAND_SPEED_MM_PER_SECOND * step) as f32;

    let mut previous: Option<[Vec<Vec3>; 2]> = None;
    let mut corrections: Vec<[Vec<Vec3>; 2]> = Vec::new();
    let mut jolts: Vec<(f64, Hand, f32)> = Vec::new();
    let mut breaks: Vec<(f64, f32)> = Vec::new();
    let mut jumps: Vec<(f64, Hand, f32)> = Vec::new();
    let mut every: Vec<f32> = Vec::new();
    let mut overlaps: Vec<f32> = Vec::new();

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
        let depth = on_viz::timeline::overlap_depth(&now[0], &now[1]);
        if depth > 2.0 {
            overlaps.push(depth);
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
    let mut strays: Vec<(f64, Hand, on_hand::Finger, u8, f32)> = Vec::new();
    let mut straying: [[Option<u8>; 5]; 2] = [[None; 5]; 2];
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
        for side in 0..2 {
            for finger in on_hand::Finger::ALL {
                let tip = joints[side][1 + 4 * finger.index() + 3];
                let under = key_under(&keyboard, tip);
                let stray = under.and_then(|midi| {
                    let (top, dip) = if is_black(midi) { (BLACK_KEY_HEIGHT, BLACK_KEY_DIP) } else { (0.0, KEY_DIP) };
                    let surface = top - keys.depth_of(midi) * dip;
                    let into = surface - tip.z;
                    (into > STRAY_MM && keys.depth_of(midi) < 0.05).then_some((midi, into))
                });
                let slot = &mut straying[side][finger.index()];
                match stray {
                    Some((midi, into)) if *slot != Some(midi) => {
                        strays.push((t, Hand::ALL[side], finger, midi, into));
                        *slot = Some(midi);
                    }
                    Some(_) => {}
                    None => *slot = None,
                }
            }
        }
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
    let own = |t: f64, hand: Hand, finger: on_hand::Finger, midi: u8, early: bool| {
        timeline.notes.iter().any(|n| {
            n.hand == hand && n.finger == Some(finger) && n.midi == midi && if early {
                n.key_moves >= t - 0.01 && n.key_moves <= t + 0.08
            } else {
                n.key_rises <= t && n.key_rises >= t - 0.25
            }
        })
    };
    let early = strays.iter().filter(|s| own(s.0, s.1, s.2, s.3, true)).count();
    let late = strays.iter().filter(|s| !own(s.0, s.1, s.2, s.3, true) && own(s.0, s.1, s.2, s.3, false)).count();
    let strays: Vec<_> = strays.into_iter().filter(|s| !own(s.0, s.1, s.2, s.3, true) && !own(s.0, s.1, s.2, s.3, false)).collect();
    println!("  stray touches (a finger pushing a key nobody is playing): {}  (+{early} early on its own key, {late} late off it)", strays.len());
    for (t, hand, finger, midi, into) in strays.iter().take(if std::env::var("ON_STRAYS").is_ok() { 40 } else { 0 }) {
        println!("    {t:8.3}s {hand:?} {finger:?} on {midi}, {into:.1} mm in");
    }
    println!("  built in {load_seconds:.2} s{}", if prior.is_some() { " (fingered with the model)" } else { "" });
    every.sort_by(f32::total_cmp);
    let at_percentile = |p: f64| every[((every.len() - 1) as f64 * p) as usize];
    let (median, p99) = (at_percentile(0.5), at_percentile(0.99));
    println!("  {} frames at {FPS:.0} fps, a hand may cover {budget:.0} mm in one of them", every.len() / 2);
    println!("  worst joint movement per frame: median {median:.1} mm, 99th {p99:.1} mm, worst {:.1} mm", every[every.len() - 1]);
    println!("  frames where a joint outran a hand: {}", jumps.len());
    println!("  frames where moving the hands apart jolted one: {}", jolts.len());
    println!(
        "  frames where the hands pass through each other: {} (deepest {:.0} mm)",
        overlaps.len(),
        overlaps.iter().copied().fold(0.0f32, f32::max)
    );
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
            let mut when: Vec<usize> = Vec::new();
            for k in range {
                let p = &tracks[side][k];
                for i in 1..p.len().saturating_sub(2) {
                    speed.push(((p[i + 1] - p[i - 1]) / (2.0 * fine as f32)).length());
                    accel.push(((p[i + 1] - 2.0 * p[i] + p[i - 1]) / (fine * fine) as f32).length());
                    jerk.push(((p[i + 2] - 3.0 * p[i + 1] + 3.0 * p[i] - p[i - 1]) / (fine * fine * fine) as f32).length());
                }
                let found = twitches(p);
                twitch += found.len();
                when.extend(found);
            }
            println!("    {hand:?} {label:5} twitches {twitch:5}");
            if std::env::var("ON_WHEN").is_ok() && label == "wrist" {
                when.sort_unstable();
                when.dedup();
                let times: Vec<String> = when.iter().take(12).map(|i| format!("{:.3}", *i as f64 * fine)).collect();
                println!("      at {}", times.join(" "));
            }
            println!("      speed  {}", percentiles(speed));
            println!("      accel  {}", percentiles(accel));
            println!("      jerk   {}", percentiles(jerk));
        }
        let wrist = &tracks[side][0];
        let fastest = (1..wrist.len()).max_by(|a, b| {
            wrist[*a].distance(wrist[*a - 1]).total_cmp(&wrist[*b].distance(wrist[*b - 1]))
        });
        if let Some(at) = fastest {
            println!("    {hand:?} fastest wrist at {:.3} s", at as f64 * fine);
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
    let (mut tip_drops, mut hand_drops, mut shares, mut pivots) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for note in &timeline.notes {
        let Some(finger) = note.finger else { continue };
        let side = note.hand as usize;
        let tip_of = |pose: &HandPose| animators[side].joints(pose)[1 + 4 * finger.index() + 3].z;
        let low = poses(&animators, note.key_bottoms())[side];
        let high = (0..=72)
            .map(|i| poses(&animators, note.key_moves - 0.3 + f64::from(i) / 240.0)[side])
            .max_by(|a, b| tip_of(a).total_cmp(&tip_of(b)))
            .expect("sampled");
        let tip = tip_of(&high) - tip_of(&low);
        if tip < 2.0 {
            continue;
        }
        let mut carried = low;
        carried.q[dof::THUMB_CMC_FLEX..].copy_from_slice(&high.q[dof::THUMB_CMC_FLEX..]);
        let hand = tip_of(&high) - tip_of(&carried);
        tip_drops.push(tip);
        pivots.push((high.q[dof::WRIST_FLEXION] - low.q[dof::WRIST_FLEXION]).to_degrees().abs());
        hand_drops.push(hand);
        shares.push(100.0 * (hand / tip).clamp(-1.0, 2.0));
    }
    let wrist_led = shares.iter().filter(|s| **s >= 50.0).count();
    let spread = |mut v: Vec<f32>| {
        if v.is_empty() {
            return "-".to_string();
        }
        v.sort_by(f32::total_cmp);
        let at = |p: f64| v[((v.len() - 1) as f64 * p) as usize];
        format!("p10 {:5.1}  p50 {:5.1}  p90 {:5.1}", at(0.1), at(0.5), at(0.9))
    };
    println!("  strokes: fingertip drop (mm) {}", spread(tip_drops));
    println!("           by the hand (mm)    {}", spread(hand_drops));
    println!("           wrist pivot (deg)   {}", spread(pivots));
    println!("           hand share (%)      {}  hand-led {wrist_led}/{}", spread(shares.clone()), shares.len());
    for (label, picked) in [("bend", [dof::MCP_FLEX, dof::PIP_FLEX]), ("spread", [dof::MCP_SPREAD, dof::MCP_SPREAD])] {
        let mut rates: Vec<f32> = Vec::new();
        for side in 0..2 {
            for slot in 0..4 {
                for j in picked.iter().map(|o| dof::finger(slot) + o).collect::<std::collections::BTreeSet<_>>() {
                    rates.extend(angles[side].windows(2).map(|w| 100.0 * (w[1][j] - w[0][j]).abs() / fine as f32));
                }
            }
        }
        println!("  finger {label} speed (centirad/s): {}", percentiles(rates));
    }
    if std::env::var("ON_PEAKS").is_ok() {
        let mut peaks: Vec<(f32, f64, usize, usize)> = Vec::new();
        for side in 0..2 {
            for k in 1..6 {
                let p = &tracks[side][k];
                for i in 1..p.len().saturating_sub(1) {
                    let speed = ((p[i + 1] - p[i - 1]) / (2.0 * fine as f32)).length();
                    peaks.push((speed, i as f64 * fine, side, k - 1));
                }
            }
        }
        let mut joints: Vec<(f32, f64, usize, usize)> = Vec::new();
        for side in 0..2 {
            for j in dof::THUMB_CMC_FLEX..on_hand::skeleton::DOF {
                for (i, w) in angles[side].windows(2).enumerate() {
                    joints.push(((w[1][j] - w[0][j]).abs() / fine as f32, i as f64 * fine, side, j));
                }
            }
        }
        joints.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut seen: Vec<f64> = Vec::new();
        for (rate, t, side, j) in joints {
            if seen.iter().any(|s| (s - t).abs() < 0.1) {
                continue;
            }
            println!("    joint peak {t:8.3}s {:?} dof {j} {rate:5.1} rad/s", Hand::ALL[side]);
            seen.push(t);
            if seen.len() == 12 {
                break;
            }
        }
        peaks.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut shown: Vec<f64> = Vec::new();
        for (speed, t, side, finger) in peaks {
            if shown.iter().any(|s| (s - t).abs() < 0.1) {
                continue;
            }
            println!("    tip peak {t:8.3}s {:?} finger {} {speed:6.0} mm/s", Hand::ALL[side], finger + 1);
            shown.push(t);
            if shown.len() == 15 {
                break;
            }
        }
    }
    for side in 0..2 {
        let (mut hooked, mut flat, mut thumb_out) = (0usize, 0usize, 0usize);
        for q in &angles[side] {
            let fingers = (0..4).map(|slot| (q[dof::finger(slot) + dof::MCP_FLEX], q[dof::finger(slot) + dof::PIP_FLEX]));
            let fingers: Vec<(f32, f32)> = fingers.collect();
            if fingers.iter().any(|(mcp, pip)| *mcp < -0.2 && *pip > 1.0) {
                hooked += 1;
            }
            if fingers.iter().filter(|(mcp, pip)| *mcp < 0.1 && *pip < 0.2).count() >= 3 {
                flat += 1;
            }
            if q[dof::THUMB_CMC_ABD] > 0.25 && q[dof::THUMB_CMC_FLEX] > 0.1 {
                thumb_out += 1;
            }
        }
        let per: Vec<String> = (0..4)
            .map(|slot| {
                let count = angles[side]
                    .iter()
                    .filter(|q| q[dof::finger(slot) + dof::MCP_FLEX] < -0.2 && q[dof::finger(slot) + dof::PIP_FLEX] > 1.0)
                    .count();
                format!("{}:{:.1}%", slot + 2, 100.0 * count as f32 / angles[side].len().max(1) as f32)
            })
            .collect();
        if std::env::var("ON_HOOKS").is_ok() {
            let mut last = -1.0f64;
            let mut shown = 0;
            for (i, q) in angles[side].iter().enumerate() {
                let t = i as f64 * fine;
                let hooked: Vec<usize> = (0..4).filter(|slot| q[dof::finger(*slot) + dof::MCP_FLEX] < -0.2 && q[dof::finger(*slot) + dof::PIP_FLEX] > 1.0).collect();
                if !hooked.is_empty() && t - last > 0.5 && shown < 8 {
                    println!("    hook {t:.3}s side {side} fingers {:?}", hooked.iter().map(|s| s + 2).collect::<Vec<_>>());
                    shown += 1;
                    last = t;
                }
            }
        }
        let thumb_hook = angles[side].iter().filter(|q| q[dof::THUMB_MCP_FLEX] < -0.25 && q[dof::THUMB_IP_FLEX] > 0.8).count();
        println!("  side {side} hooks by finger: 1:{:.1}% {}", 100.0 * thumb_hook as f32 / angles[side].len().max(1) as f32, per.join(" "));
        let n = angles[side].len().max(1) as f32 / 100.0;
        println!(
            "  side {side} posture: hooked finger {:.1}%  flat hand {:.1}%  thumb out {:.1}%",
            hooked as f32 / n, flat as f32 / n, thumb_out as f32 / n
        );
    }

    let mut strikes: Vec<(f32, f64, Hand, on_hand::Finger, u8)> = Vec::new();
    for note in &timeline.notes {
        let Some(finger) = note.finger else { continue };
        let t = note.start;
        let posed = poses(&animators, t);
        let joints = animators[note.hand as usize].joints(&posed[note.hand as usize]);
        let tip = joints[1 + 4 * finger.index() + 3];
        let (x0, x1, y0, y1) = keyboard.footprint(note.midi);
        let depth = timeline.key_depression(t).depth_of(note.midi);
        let (surface, dip) = if is_black(note.midi) { (BLACK_KEY_HEIGHT, BLACK_KEY_DIP) } else { (0.0, KEY_DIP) };
        let across = (x0 - tip.x).max(tip.x - x1).max(0.0);
        let along = (y0 - tip.y).max(tip.y - y1).max(0.0);
        let down = (tip.z - (surface - depth * dip)).abs();
        strikes.push(((across * across + along * along + down * down).sqrt(), t, note.hand, finger, note.midi));
    }
    let misses = |limit: f32| strikes.iter().filter(|s| s.0 > limit).count();
    println!(
        "  fingertip to its key as it sounds (mm): {}   over 5 mm: {}  over 20 mm: {} of {}",
        percentiles(strikes.iter().map(|s| s.0).collect()).replace("p", " p"),
        misses(5.0),
        misses(20.0),
        strikes.len()
    );
    strikes.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (miss, t, hand, finger, midi) in strikes.iter().take(10) {
        println!("    {t:8.3}s {hand:?} {finger:?} {midi} missed by {miss:.0} mm");
    }

    let mut shown = 0;
    let mut last = f64::MIN;
    for (time, hand, moved) in &jumps {
        if time - last > 0.25 {
            if shown == 12 {
                println!("    ... and more");
                break;
            }
            println!("    {time:7.2}s {hand:?} a joint moved {moved:6.1} mm in one frame ({:.0} mm/s)", f64::from(*moved) * FPS);
            shown += 1;
        }
        last = *time;
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
    Ok(())
}
