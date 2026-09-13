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
//! louder note. From the camera's point of view that sink is almost end-on and so reads
//! faintly; a number says what a frame cannot.
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
        "{:>8}  {:>17}  {:>17}",
        "", "fingertip", "wrist"
    );
    println!(
        "{:>8}  {:>8}{:>9}  {:>8}{:>9}",
        "time", "quiet", "loud", "quiet", "loud"
    );
    let mut peak = (0.0f32, 0.0f32);
    let mut sink = (0.0f32, 0.0f32);
    let measure = |velocity: u8, at: f64| -> (f32, f32) {
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
        (posture.chain[Finger::Middle.index()][3].z, posture.wrist.z)
    };

    // Where the wrist sits once it has finished taking the note, so the sink below is
    // measured against rest rather than against the bottom of its own dip.
    let settled = (measure(30, 0.70).1, measure(120, 0.70).1);

    let mut t = 0.72;
    while t <= 1.10 {
        let (quiet_tip, quiet_wrist) = measure(30, t);
        let (loud_tip, loud_wrist) = measure(120, t);
        peak.0 = peak.0.max(quiet_tip);
        peak.1 = peak.1.max(loud_tip);
        sink.0 = sink.0.max(settled.0 - quiet_wrist);
        sink.1 = sink.1.max(settled.1 - loud_wrist);
        println!(
            "{t:>8.3}  {quiet_tip:>8.1}{loud_tip:>9.1}  {:>8.2}{:>9.2}",
            quiet_wrist - settled.0,
            loud_wrist - settled.1,
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
    println!("  (a key travels 10 mm when it is pressed, for scale)");
    let _ = Grip::new(vec![(60, Finger::Middle)]);
}
