//! Prints what the hand model thinks it can reach, so the numbers can be checked
//! against what pianists actually report. Run with `cargo run -p on-hand --example
//! calibration`.

use on_hand::ik::{reach, ReachRequest};
use on_hand::keyboard::Keyboard;
use on_hand::profile::{HandProfile, HandSize};
use on_hand::skeleton::Skeleton;
use on_hand::{Finger, Hand};

fn main() {
    let kb = Keyboard::new();
    let sizes = [
        ("XS", HandSize::ExtraSmall),
        ("S", HandSize::Small),
        ("M", HandSize::Medium),
        ("L", HandSize::Large),
        ("XL", HandSize::ExtraLarge),
    ];
    let intervals = [
        ("6th", 69u8),
        ("7th", 71),
        ("octave", 72),
        ("9th", 74),
        ("10th", 76),
        ("11th", 77),
        ("12th", 79),
    ];

    println!("Thumb on C4, little finger reaching up. Cost of holding the stretch,");
    println!("and whether the fingers land at all.\n");
    print!("{:<8}", "");
    for (name, _) in intervals {
        print!("{name:>12}");
    }
    println!();

    for (label, size) in sizes {
        let sk = Skeleton::new(HandProfile::from_size(size), Hand::Right);
        print!("{label:<8}");
        for (_, top) in intervals {
            let targets = [
                (Finger::Thumb, kb.nominal_strike(60)),
                (Finger::Little, kb.nominal_strike(top)),
            ];
            let out = reach(&sk, &ReachRequest::new(&targets));
            let cell = if out.reached {
                format!("{:.1}", out.strain_total())
            } else {
                format!("-{:.0}mm", out.max_error_mm)
            };
            print!("{cell:>12}");
        }
        println!();
    }

    println!("\nA dash means the fingers could not reach the keys, and the number is");
    println!("how far short they fell.");
}
