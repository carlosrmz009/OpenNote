//! How far a finger actually rises before it strikes a repeated note.
//!
//! The lift is what makes a repeated note look repeated, and it is easy to overdo: too
//! much and the knuckle straightens, the finger stops looking like a finger, and the
//! hand appears to flail at the keyboard. This reports the height of the striking
//! fingertip over one strike, in millimetres above the key, so the size of the gesture
//! is a number rather than an impression.
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
    println!("Fingertip height above the key, through one repeated note.\n");
    println!("{:>10}  {:>8}  {:>8}", "time", "quiet", "loud");
    let mut peak = (0.0f32, 0.0f32);
    let height = |velocity: u8, at: f64| {
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
        posture.chain[Finger::Middle.index()][3].z
    };
    let mut t = 0.72;
    while t <= 1.02 {
        let (q, l) = (height(30, t), height(120, t));
        peak.0 = peak.0.max(q);
        peak.1 = peak.1.max(l);
        println!("{t:>10.3}  {q:>8.1}  {l:>8.1}");
        t += 0.03;
    }
    println!("\n  highest the tip gets: quiet {:.1} mm, loud {:.1} mm", peak.0, peak.1);
    println!("  (a key travels 10 mm when it is pressed, for scale)");
    let _ = Grip::new(vec![(60, Finger::Middle)]);
}
