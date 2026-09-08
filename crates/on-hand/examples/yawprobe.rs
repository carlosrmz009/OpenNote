//! Which way does wrist deviation turn the hand, and by how much?
//!
//! Establishes the sign convention empirically rather than by reading quaternions.
use on_hand::skeleton::dof;
use on_hand::{Finger, Hand, HandProfile, Skeleton};

fn main() {
    for hand in Hand::ALL {
        let sk = Skeleton::new(HandProfile::default(), hand);
        println!("{hand:?}");
        for deg in [-20.0f32, -10.0, 0.0, 10.0, 20.0] {
            let mut pose = sk.rest_pose(glam::Vec3::new(600.0, -80.0, 60.0));
            pose.q[dof::WRIST_DEVIATION] = deg.to_radians();
            let posture = sk.forward(&pose);
            let tip = posture.chain[Finger::Middle.index()][3];
            let wrist = posture.wrist;
            let d = tip - wrist;
            // Angle of the hand's long axis in the keyboard plane, measured from +y
            // (straight away from the player). Positive means turned towards +x.
            let axis = d.x.atan2(d.y).to_degrees();
            println!("   q[dev]={deg:>6.1}°  tip-wrist=({:>7.1},{:>7.1})  hand axis {axis:>7.2}°",
                d.x, d.y);
        }
    }
}
