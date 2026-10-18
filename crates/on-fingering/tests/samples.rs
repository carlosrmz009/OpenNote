//! The fingering of the sample scores, held against a snapshot so that any change to the
//! engine that changes a real piece's fingering shows up in review.
//!
//! After a deliberate change, update the snapshots with
//! `UPDATE_SNAPSHOTS=1 cargo test -p on-fingering --test samples` and review the diff.

use std::path::{Path, PathBuf};

use on_fingering::{finger_score, FingeringOptions};
use on_hand::keyboard::name;

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// One line per note, in order: when, which key, which hand and which finger.
fn fingering_of(file: &str) -> String {
    let mut document = on_score::Document::open(&samples().join(file)).expect("the sample opens");
    on_score::assign_hands(document.score_mut(), &Default::default());
    let score = document.score();
    let solution = finger_score(score, &FingeringOptions::default());
    let mut out = String::new();
    for note in &score.notes {
        let hand = note.hand.map_or("-", |hand| if hand == on_hand::Hand::Left { "L" } else { "R" });
        let finger = solution.finger_of(note.id).map_or("-".to_string(), |f| f.number().to_string());
        out.push_str(&format!("{:>8} {:<4} {hand} {finger}\n", note.onset, name(note.midi)));
    }
    out
}

fn check(file: &str) {
    let found = fingering_of(file);
    let snapshot = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots").join(format!("{file}.txt"));
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&snapshot, &found).unwrap();
        return;
    }
    // A Windows checkout may have turned the snapshot's newlines into CRLF.
    let expected = std::fs::read_to_string(&snapshot)
        .unwrap_or_else(|_| panic!("no snapshot at {}; run with UPDATE_SNAPSHOTS=1", snapshot.display()))
        .replace("\r\n", "\n");
    if found != expected {
        let changed: Vec<String> = expected
            .lines()
            .zip(found.lines())
            .filter(|(was, now)| was != now)
            .take(12)
            .map(|(was, now)| format!("  was {was}\n  now {now}"))
            .collect();
        panic!(
            "the fingering of {file} changed ({} of {} lines); if that is intended, update the snapshot\n{}",
            expected.lines().zip(found.lines()).filter(|(a, b)| a != b).count(),
            expected.lines().count(),
            changed.join("\n")
        );
    }
}

#[test]
fn fur_elise() {
    check("fur-elise.musicxml");
}

#[test]
fn scales_and_chords() {
    check("scales-and-chords.musicxml");
}
