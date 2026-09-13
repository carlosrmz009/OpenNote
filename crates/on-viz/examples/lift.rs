//! How far the hand actually moves to strike a note.
//!
//! Two heights, both in millimetres and both through one strike of a repeated note:
//! the striking fingertip, and the wrist it hangs from.
//!
//! The lift is what makes a repeated note look repeated, and it is easy to overdo: too
//! much and the knuckle straightens, the finger stops looking like a finger, and the
//! hand appears to flail at the keyboard.
//!
//! The wrist is the other half of it. A note is played with arm weight arriving through
//! the finger, so the wrist sinks into the key and releases afterwards, further for a
//! louder note.
//!
//! The third column is the one that decides whether any of it can be *seen*. The camera
//! looks straight down through an orthographic projection, so height is exactly the view
//! axis and a wrist that only changes height moves zero pixels. What reaches the screen
//! is the hand's length as projected onto the keyboard plane — wrist to fingertip, flat
//! — which shortens as the wrist rolls back into the note. That number, not the height,
//! is what the viewer has to be able to notice.
//!
//! A key travels 10 mm when pressed, which is the scale to read both against.
//!
//!     cargo run -p on-viz --example lift
use on_fingering::biomech::{BiomechModel, Grip};
use on_hand::{Finger, Hand, HandProfile};
use on_score::{Fingering, Note, NoteId, Score, SourceRef, TieState, TICKS_PER_QUARTER};
use on_viz::timeline::{HandAnimator, Timeline};

fn score_of(velocity: u8) -> Score {
    let q = TICKS_PER_QUARTER as i64;
    let mut score = Score::default();
    for i in 0..4 {
        score.notes.push(Note {
            id: NoteId(i),
            midi: 60,
            onset: i as i64 * q,
            duration: q / 2,
            onset_seconds: 0.0,
            duration_seconds: 0.0,
            staff: None,
            voice: None,
            hand: Some(Hand::Right),
            tie: TieState::default(),
            grace: false,
            chord: false,
            velocity,
            given_finger: None,
            source: SourceRef::Midi { track: 0, event: i as usize },
        });
    }
    score.finalise();
    score
}

fn main() {
    println!("Height through one strike of a repeated note, in millimetres.\n");
    println!(
        "{:>8}  {:>17}  {:>17}  {:>17}",
        "", "fingertip height", "wrist height", "seen from above"
    );
    println!(
        "{:>8}  {:>8}{:>9}  {:>8}{:>9}  {:>8}{:>9}",
        "time", "quiet", "loud", "quiet", "loud", "quiet", "loud"
    );
    let mut peak = (0.0f32, 0.0f32);
    let mut sink = (0.0f32, 0.0f32);
    let mut seen = (0.0f32, 0.0f32);
    let measure = |velocity: u8, at: f64| -> (f32, f32, f32) {
        let score = score_of(velocity);
        let fingerings: Vec<Fingering> = (0..4)
            .map(|i| Fingering { note: NoteId(i), finger: Finger::Middle, substitute: None })
            .collect();
        let timeline = Timeline::build(&score, &fingerings);
        let model = BiomechModel::new(HandProfile::default(), Hand::Right, Default::default());
        let animator = HandAnimator::new(
            Hand::Right,
            model,
            timeline.hand_grips(Hand::Right).to_vec(),
        );
        let pose = animator.pose_at(at);
        let posture = animator.skeleton().forward(&pose);
        let tip = posture.chain[Finger::Middle.index()][3];
        // What the camera actually gets: everything flattened onto the keyboard plane.
        let flat = (tip.truncate() - posture.wrist.truncate()).length();
        (tip.z, posture.wrist.z, flat)
    };

    // Where the hand sits once it has finished taking the note, so the sink below is
    // measured against rest rather than against the bottom of its own dip.
    let rest = (measure(30, 0.70), measure(120, 0.70));
    let settled = (rest.0 .1, rest.1 .1);
    let flat_rest = (rest.0 .2, rest.1 .2);

    let mut t = 0.72;
    while t <= 1.10 {
        let (quiet_tip, quiet_wrist, quiet_flat) = measure(30, t);
        let (loud_tip, loud_wrist, loud_flat) = measure(120, t);
        peak.0 = peak.0.max(quiet_tip);
        peak.1 = peak.1.max(loud_tip);
        sink.0 = sink.0.max(settled.0 - quiet_wrist);
        sink.1 = sink.1.max(settled.1 - loud_wrist);
        seen.0 = seen.0.max((quiet_flat - flat_rest.0).abs());
        seen.1 = seen.1.max((loud_flat - flat_rest.1).abs());
        println!(
            "{t:>8.3}  {quiet_tip:>8.1}{loud_tip:>9.1}  {:>8.2}{:>9.2}  {:>8.2}{:>9.2}",
            quiet_wrist - settled.0,
            loud_wrist - settled.1,
            quiet_flat - flat_rest.0,
            loud_flat - flat_rest.1,
        );
        t += 0.03;
    }
    println!(
        "\n  highest the tip gets:   quiet {:.1} mm, loud {:.1} mm",
        peak.0, peak.1
    );
    println!(
        "  deepest the wrist sinks: quiet {:.2} mm, loud {:.2} mm",
        sink.0, sink.1
    );
    println!(
        "  most the hand shortens on screen: quiet {:.2} mm, loud {:.2} mm",
        seen.0, seen.1
    );
    println!("  (a key travels 10 mm when it is pressed, and a white key is 23.5 mm wide)");
    let _ = Grip::new(vec![(60, Finger::Middle)]);
}
