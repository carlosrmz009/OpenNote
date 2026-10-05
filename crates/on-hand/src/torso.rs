use glam::Vec3;

use crate::profile::HandProfile;
use crate::Hand;

const SHOULDER_APART: f32 = 1.03;

const SHOULDER_ABOVE: f32 = 2.01;

const SHOULDER_BEHIND: f32 = 2.12;

const UPPER_ARM: f32 = 1.75;
const FOREARM: f32 = 1.40;

const FREE_LEAN_MM: f32 = 90.0;

const MAX_LEAN_MM: f32 = 320.0;

const EASY_REACH: f32 = 0.86;

const ELBOW_OUT: f32 = 0.6;

const PLAYING_Y_MM: f32 = -80.0;
const PLAYING_Z_MM: f32 = 60.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Torso {
    centre_x: f32,
    half_apart: f32,
    above: f32,
    behind: f32,
    reach: f32,
}

impl Torso {
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

    pub fn reach_mm(&self) -> f32 {
        self.reach
    }

    pub fn arm_mm(&self) -> (f32, f32) {
        let share = UPPER_ARM / (UPPER_ARM + FOREARM);
        (self.reach * share, self.reach * (1.0 - share))
    }

    pub fn elbow(&self, hand: Hand, shoulder: Vec3, wrist: Vec3) -> Vec3 {
        let (upper, fore) = self.arm_mm();
        let to_wrist = wrist - shoulder;
        let distance = to_wrist.length().clamp((upper - fore).abs() + 1.0, upper + fore - 1.0);
        let along = to_wrist.normalize_or_zero();
        let a = (upper * upper - fore * fore + distance * distance) / (2.0 * distance);
        let h = (upper * upper - a * a).max(0.0).sqrt();
        let outward = match hand {
            Hand::Left => -1.0,
            Hand::Right => 1.0,
        };
        let pole = Vec3::new(outward * ELBOW_OUT, 0.0, -1.0);
        let side = (pole - along * pole.dot(along)).normalize_or_zero();
        shoulder + along * a + side * h
    }

    pub fn shoulder(&self, hand: Hand, lean: f32) -> Vec3 {
        let outward = match hand {
            Hand::Left => -self.half_apart,
            Hand::Right => self.half_apart,
        };
        Vec3::new(self.centre_x + outward + lean, -self.behind, self.above)
    }

    pub fn lean_for(&self, hand: Hand, wrist: Vec3) -> f32 {
        let shoulder = self.shoulder(hand, 0.0);
        (wrist.x - shoulder.x).clamp(-MAX_LEAN_MM, MAX_LEAN_MM)
    }

    pub fn preference(&self, hand: Hand, x: f32) -> f32 {
        let other = match hand {
            Hand::Left => Hand::Right,
            Hand::Right => Hand::Left,
        };
        (self.effort_along(hand, x) - self.effort_along(other, x)).max(0.0)
    }

    pub fn effort_along(&self, hand: Hand, x: f32) -> f32 {
        self.effort(hand, Vec3::new(x, PLAYING_Y_MM, PLAYING_Z_MM))
    }

    pub fn effort(&self, hand: Hand, wrist: Vec3) -> f32 {
        let lean = self.lean_for(hand, wrist);
        let shoulder = self.shoulder(hand, lean);
        let away = (wrist - shoulder).length();

        let leaned = (lean.abs() - FREE_LEAN_MM).max(0.0) / self.reach;

        let easy = self.reach * EASY_REACH;
        let stretched = if away <= easy {
            0.0
        } else {
            let past = (away - easy) / (self.reach - easy);
            past * past * 2.0
        };
        leaned + stretched
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_elbow_joins_an_upper_arm_and_a_forearm_of_the_right_length() {
        let torso = player();
        for (hand, x) in [(Hand::Left, 450.0), (Hand::Right, 800.0), (Hand::Right, 300.0)] {
            let wrist = Vec3::new(x, -60.0, 50.0);
            let shoulder = torso.shoulder(hand, torso.lean_for(hand, wrist));
            let elbow = torso.elbow(hand, shoulder, wrist);
            let (upper, fore) = torso.arm_mm();
            assert!(((elbow - shoulder).length() - upper).abs() < 1.0);
            assert!(((wrist - elbow).length() - fore).abs() < 1.0);
            assert!(elbow.z < shoulder.z, "the elbow hangs below the shoulder");
            let outward = if hand == Hand::Left { -1.0 } else { 1.0 };
            let line = shoulder + (wrist - shoulder) * ((elbow - shoulder).dot(wrist - shoulder) / (wrist - shoulder).length_squared());
            assert!((elbow.x - line.x) * outward > 0.0, "the elbow points away from the body");
        }
    }

    fn player() -> Torso {
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
        let own_side = torso.effort(Hand::Right, at(611.0 + 400.0));
        let across = torso.effort(Hand::Right, at(611.0 - 400.0));
        assert!(
            across > own_side,
            "reaching across should cost more: across {across}, own side {own_side}"
        );
    }
}
