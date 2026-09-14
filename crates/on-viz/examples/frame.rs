//! Everything a renderer needs to draw one frame, and nothing that knows how to draw.
//!
//! This is the seam. `on-viz` comes apart into a half that works out what is happening
//! at time *t* and a half that puts it on a screen with Bevy; the first half is free of
//! any graphics stack and is what a port to another one — a phone, a different engine —
//! keeps. Build it with `--no-default-features` and none of Bevy is compiled at all.
//!
//! Run it to see the three calls a frame is made of:
//!
//! ```text
//! cargo run --release -p on-viz --no-default-features --example frame -- score.mid 8.0
//! ```
//!
//! It exists to be built as much as to be run. The no-GPU configuration had quietly
//! stopped compiling because nothing in the workspace asked for it; something that does
//! is how that stays fixed.
use on_fingering::biomech::BiomechWeights;
use on_fingering::FingeringOptions;
use on_hand::Hand;
use on_score::hands::HandAssignment;
use on_viz::layout::{Extent, Layout};
use on_viz::timeline::{pose_both, Timeline};

/// How far ahead of the keyboard notes are drawn, in seconds.
const LOOKAHEAD: f64 = 3.0;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| {
        eprintln!("usage: frame <score> [seconds]");
        std::process::exit(2);
    });
    let at: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(4.0);

    // 1. Read the score and decide the two things the renderer does not: which hand
    //    plays each note, and with which finger.
    let mut document = on_score::Document::open(std::path::Path::new(&path))?;
    let options = FingeringOptions::default();
    on_score::assign_hands(
        document.score_mut(),
        &HandAssignment { profile: options.profile.clone(), ..Default::default() },
    );
    let score = document.score();
    let solution = on_fingering::finger_score_consensus(score, &options, None);

    // 2. Build the timeline and the two hands. `animators` takes both hands together on
    //    purpose: where an idle hand waits depends on what the other one is doing.
    let timeline = Timeline::build_for(score, &solution.fingerings, &options.profile);
    let animators = timeline.animators(&options.profile, BiomechWeights::default());
    let layout = Layout::for_pitches(timeline.notes.iter().map(|n| n.midi), Extent::FullKeyboard);

    println!("{path}");
    println!(
        "  {} notes, {:.1} s, view {:.0} x {:.0} mm\n",
        timeline.notes.len(),
        timeline.duration,
        layout.width_mm(),
        layout.height_mm()
    );
    println!("frame at {at:.2} s");

    // 3a. The keys. Every key that is down, how far, and whose finger is on it.
    let keys = timeline.key_depression(at);
    let mut pressed: Vec<u8> = keys.pressed().collect();
    pressed.sort_unstable();
    println!("\n  keys down: {}", pressed.len());
    for midi in pressed {
        let (x0, x1, y0, y1) = layout.key_rect(midi);
        println!(
            "    {midi:3}  {:5.1} mm down  {:?}  rect ({x0:7.1},{y0:7.1})-({x1:7.1},{y1:7.1})",
            keys.depth_of(midi),
            keys.hand_on(midi)
        );
    }

    // 3b. The falling notes, as rectangles in the same millimetre space as the keys.
    let visible: Vec<_> = timeline.visible_notes(at, LOOKAHEAD).collect();
    println!("\n  notes in view over the next {LOOKAHEAD:.0} s: {}", visible.len());
    for note in visible.iter().take(8) {
        let (x0, x1, bottom, top) =
            layout.note_rect(note.midi, note.start - at, note.end - note.start);
        println!(
            "    {:3}  {:?}  finger {}  velocity {:3}  rect ({x0:7.1},{bottom:7.1})-({x1:7.1},{top:7.1})",
            note.midi,
            note.hand,
            note.finger.map_or("-".into(), |f| f.number().to_string()),
            note.velocity,
        );
    }
    if visible.len() > 8 {
        println!("    ... and {} more", visible.len() - 8);
    }

    // 3c. The hands. Both at once, because whether one has to be lifted over the other
    //     is the one thing about its posture it cannot decide by itself.
    let poses = pose_both(&animators, at);
    println!("\n  hands");
    for hand in Hand::ALL {
        let animator = &animators[hand as usize];
        let posture = animator.skeleton().forward(&poses[hand as usize]);
        let holding = animator.grip_at(at).map_or(0, |grip| grip.keys.len());
        println!(
            "    {hand:?}: wrist ({:7.1},{:7.1},{:6.1})  holding {holding} key(s)",
            posture.wrist.x, posture.wrist.y, posture.wrist.z
        );
        for (digit, name) in ["thumb", "index", "middle", "ring", "little"].iter().enumerate() {
            let tip = posture.chain[digit][3];
            println!("      {name:<7} tip ({:7.1},{:7.1},{:6.1})", tip.x, tip.y, tip.z);
        }
    }

    println!(
        "\n  Millimetres throughout, and the same space for keys, notes and hands, so a\n  \
         hand that spans an octave on a real piano spans an octave here."
    );
    Ok(())
}
