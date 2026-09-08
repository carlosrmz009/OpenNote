//! The player the hands belong to.
//!
//! A hand does not float above the keyboard. It is on the end of an arm, and the arm
//! hangs from a shoulder that belongs to somebody sitting on a bench in one place. That
//! is the difference between the middle of the keyboard and the ends of it, and it is
//! why a note two octaves below middle C belongs to the left hand even when the right
//! hand happens to be free: the right hand can get there, but not without the player
//! reaching across themselves to do it.
//!
//! The rest of [`crate`] describes what a hand can do. This describes where a hand can
//! *be*, which is the other half of the same question and was previously stood in for
//! by a flat charge per octave either side of middle C — a reasonable guess, but one
//! with no shoulder in it, so it said the same thing about a note a fourth below middle
//! C as about the bottom of the keyboard.
//!
//! # Sitting at the piano
//!
//! Everything is in the keyboard's own frame, in millimetres: `x` runs along the keys,
//! `y` runs along a key towards the player, and `z` is height above the white keys.
//!
//! A pianist sits centred on the instrument with the bench set so the forearms come in
//! roughly level. That puts the shoulders above and behind the keys, a little under
//! half a metre apart. The numbers below are ordinary adult dimensions, scaled with the
//! hand: a player with bigger hands has longer arms and sits further back.
//!
//! # Leaning
//!
//! The shoulders are not nailed down. Nobody plays the top octave from the middle of
//! the bench — the torso leans, and at the extremes it turns and the player shifts.
//! That is modelled as the shoulder sliding along `x`, freely for the first little way
//! and then at a rising cost, which is what leaning actually feels like. Without it a
//! rigid torso declares the outer two octaves unreachable, which is false; with it the
//! ends of the keyboard cost something to get to, which is true.

use glam::Vec3;

use crate::profile::HandProfile;
use crate::Hand;

/// Shoulder half-separation, as a fraction of hand length.
///
/// A little over one hand length either side of the spine: an adult's shoulders are
/// about 390 mm apart and an adult hand is about 189 mm long.
const SHOULDER_APART: f32 = 1.03;

/// How far the shoulder sits above the white keys, as a fraction of hand length.
///
/// Bench height and keyboard height are both standardised, and the difference between
/// them puts a seated player's shoulder about two hand lengths above the keys.
const SHOULDER_ABOVE: f32 = 2.01;

/// How far the shoulder sits behind the keys, towards the player, as a fraction of
/// hand length. Measured to the front edge of the white keys, which is `y = 0`.
const SHOULDER_BEHIND: f32 = 2.12;

/// Upper arm and forearm lengths, as fractions of hand length.
///
/// Acromion to lateral epicondyle, and elbow to wrist crease. Both scale with stature
/// closely enough that deriving them from the hand is better than fixing them, since
/// the hand is the one measurement this program is given.
const UPPER_ARM: f32 = 1.75;
const FOREARM: f32 = 1.40;

/// How far the torso leans before leaning costs anything, in millimetres.
///
/// A hand's width or so. Everybody drifts this much without noticing.
const FREE_LEAN_MM: f32 = 90.0;

/// How far the torso can lean at all, in millimetres.
///
/// Beyond this the player has run out of bench and out of spine.
const MAX_LEAN_MM: f32 = 320.0;

/// How much of the arm's length is comfortably usable before the elbow straightens.
///
/// An arm at full stretch is a locked elbow, which nobody plays with. The last of the
/// reach is available but it costs.
const EASY_REACH: f32 = 0.86;

/// Where a hand sits when it is playing: a little in front of the keys' back edge, and
/// a little above them.
const PLAYING_Y_MM: f32 = -80.0;
const PLAYING_Z_MM: f32 = 60.0;

/// A seated player, and what their arms can get to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Torso {
    /// Where the middle of the player is, along the keyboard.
    centre_x: f32,
    /// Half the distance between the shoulders.
    half_apart: f32,
    /// Shoulder height above the white keys, and distance behind them.
    above: f32,
    behind: f32,
    /// How far a shoulder is from a wrist when the arm is straight.
    reach: f32,
}

impl Torso {
    /// A player of the size implied by their hands, sitting at the middle of the
    /// keyboard.
    pub fn new(profile: &HandProfile, keyboard_centre_x: f32) -> Self {
        let hand = profile.hand_length;
        Self {
            centre_x: keyboard_centre_x,
            half_apart: hand * SHOULDER_APART,
            above: hand * SHOULDER_ABOVE,
            behind: hand * SHOULDER_BEHIND,
            reach: hand * (UPPER_ARM + FOREARM),
        }
    }

    /// How far a straight arm reaches, in millimetres.
    pub fn reach_mm(&self) -> f32 {
        self.reach
    }

    /// Where a shoulder is, with the player leaning by `lean` millimetres along the
    /// keyboard.
    pub fn shoulder(&self, hand: Hand, lean: f32) -> Vec3 {
        let outward = match hand {
            Hand::Left => -self.half_apart,
            Hand::Right => self.half_apart,
        };
        Vec3::new(self.centre_x + outward + lean, -self.behind, self.above)
    }

    /// How far the player has to lean to put this hand where it is going.
    ///
    /// Only along the keyboard, and only as far as the bench allows. Leaning towards
    /// the hand is what shortens the reach, so the answer is whatever brings the
    /// shoulder closest to the wrist without exceeding what a spine will do.
    pub fn lean_for(&self, hand: Hand, wrist: Vec3) -> f32 {
        let shoulder = self.shoulder(hand, 0.0);
        (wrist.x - shoulder.x).clamp(-MAX_LEAN_MM, MAX_LEAN_MM)
    }

    /// How much worse this hand is than the other one for a key at `x`.
    ///
    /// Zero for whichever hand the key belongs to, and rising for the other as the note
    /// gets further onto the wrong side — which is the question the hand assignment is
    /// actually asking. The absolute effort is the wrong thing to ask there: a right
    /// hand playing high up is leaning, and it should not be charged for that against a
    /// left hand which would have to lean past it to do the same job.
    pub fn preference(&self, hand: Hand, x: f32) -> f32 {
        let other = match hand {
            Hand::Left => Hand::Right,
            Hand::Right => Hand::Left,
        };
        (self.effort_along(hand, x) - self.effort_along(other, x)).max(0.0)
    }

    /// What it costs this player to put a hand over a key at `x`.
    ///
    /// The height and depth of a playing hand barely vary — it is along the keyboard
    /// that the question is asked — so callers who only know where along the keys they
    /// are going need not invent the other two.
    pub fn effort_along(&self, hand: Hand, x: f32) -> f32 {
        self.effort(hand, Vec3::new(x, PLAYING_Y_MM, PLAYING_Z_MM))
    }

    /// What it costs this player to put a hand here.
    ///
    /// Zero for anywhere the arm falls naturally, rising as the elbow straightens and
    /// as the player has to lean to help it, and rising steeply once the arm is at full
    /// stretch — which is where a hand stops being able to play and starts merely being
    /// able to touch.
    ///
    /// In units of hand lengths of over-reach, so it is comparable across player sizes
    /// and does not need recalibrating when the hand does.
    pub fn effort(&self, hand: Hand, wrist: Vec3) -> f32 {
        let lean = self.lean_for(hand, wrist);
        let shoulder = self.shoulder(hand, lean);
        let away = (wrist - shoulder).length();

        // Leaning is free for the first little way and then is not.
        let leaned = (lean.abs() - FREE_LEAN_MM).max(0.0) / self.reach;

        let easy = self.reach * EASY_REACH;
        let stretched = if away <= easy {
            0.0
        } else {
            // Doubling back on itself past full stretch, so the last few millimetres
            // cost far more than the first few — an arm that has run out has run out.
            let past = (away - easy) / (self.reach - easy);
            past * past * 2.0
        };
        leaned + stretched
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player() -> Torso {
        // A medium hand, at the middle of an 88-key keyboard.
        Torso::new(&HandProfile::default(), 611.0)
    }

    #[test]
    fn a_seated_player_is_shaped_like_one() {
        let torso = player();
        let left = torso.shoulder(Hand::Left, 0.0);
        let right = torso.shoulder(Hand::Right, 0.0);
        assert!(
            (right.x - left.x - 390.0).abs() < 40.0,
            "shoulders should be about 390 mm apart, got {}",
            right.x - left.x
        );
        assert!(left.z > 300.0 && left.z < 450.0, "shoulder height {}", left.z);
        assert!(left.y < 0.0, "the player sits in front of the keys");
        assert!(
            torso.reach_mm() > 500.0 && torso.reach_mm() < 700.0,
            "an arm is about 600 mm, got {}",
            torso.reach_mm()
        );
    }

    #[test]
    fn the_middle_of_the_keyboard_costs_nothing_and_the_ends_cost_something() {
        // The whole point: where a hand is matters, and it matters more the further out
        // it goes. A flat charge per octave either side of middle C said the same thing
        // about a note a fourth below it as about the bottom of the keyboard.
        let torso = player();
        let at = |x: f32| Vec3::new(x, -80.0, 60.0);

        let under_the_shoulder = torso.shoulder(Hand::Right, 0.0).x;
        let home = torso.effort(Hand::Right, at(under_the_shoulder));
        let across = torso.effort(Hand::Right, at(1150.0));
        let far = torso.effort(Hand::Right, at(60.0));
        assert_eq!(home, 0.0, "a hand in front of its own shoulder is free");
        assert!(across > 0.0, "the top of the keyboard costs something");
        assert!(
            far > across,
            "and the far end costs more: {far} reaching down against {across} reaching up"
        );
    }

    #[test]
    fn every_key_is_comfortable_for_one_hand_or_the_other() {
        // Leaning is what makes this true. A torso nailed to the middle of the bench
        // declares the outer octaves out of reach even for the hand they belong to,
        // which anyone who has seen a pianist knows is false — they lean, and at the
        // extremes they turn.
        //
        // The far hand is a different matter. A seated player's right hand genuinely
        // cannot play the bottom A, and a model that says it can is not worth having.
        let torso = player();
        for x in [0.0f32, 300.0, 611.0, 900.0, 1222.0] {
            let best = Hand::ALL
                .into_iter()
                .map(|hand| torso.effort(hand, Vec3::new(x, -80.0, 60.0)))
                .fold(f32::INFINITY, f32::min);
            assert!(best < 1.0, "no hand can comfortably play {x} mm: best was {best}");
        }
    }

    #[test]
    fn a_hand_reaches_across_the_body_less_comfortably_than_to_its_own_side() {
        let torso = player();
        let at = |x: f32| Vec3::new(x, -80.0, 60.0);
        // Same distance from the middle, one to each side.
        let own_side = torso.effort(Hand::Right, at(611.0 + 400.0));
        let across = torso.effort(Hand::Right, at(611.0 - 400.0));
        assert!(
            across > own_side,
            "reaching across should cost more: across {across}, own side {own_side}"
        );
    }
}
