//! Scores in the formats Verovio reads open like any other.

use on_score::Document;

const ABC: &str = "X:1
T:Two hands
M:4/4
L:1/4
Q:1/4=120
K:C
%%score {1 | 2}
V:1 clef=treble
V:2 clef=bass
[V:1] c d e f | g4 |]
[V:2] C,2 G,,2 | C,4 |]
";

const KERN: &str = "**kern\t**kern
*staff2\t*staff1
*clefF4\t*clefG2
*M4/4\t*M4/4
*MM120\t*MM120
=1\t=1
2C\t4c
.\t4d
2G\t4e
.\t4f
=2\t=2
1C\t1g
==\t==
*-\t*-
";

fn open(name: &str, text: &str) -> Document {
    on_engrave::register_importers();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join(name);
    std::fs::write(&path, text).unwrap();
    Document::open(&path).unwrap_or_else(|error| panic!("{name}: {error:#}"))
}

fn pitches(document: &Document) -> Vec<u8> {
    let mut pitches: Vec<u8> = document.score().notes.iter().map(|note| note.midi).collect();
    pitches.sort_unstable();
    pitches
}

#[test]
fn abc_notation_opens_with_both_hands_notes() {
    let document = open("two-hands.abc", ABC);
    let pitches = pitches(&document);
    // In ABC, lower case `c` is the C above middle C.
    assert!(pitches.contains(&72) && pitches.contains(&79), "the right hand: {pitches:?}");
    assert!(pitches.iter().any(|p| *p < 55), "the left hand: {pitches:?}");
    assert!(document.score().notes.len() >= 8, "{pitches:?}");
}

#[test]
fn humdrum_kern_opens_too() {
    let document = open("two-hands.krn", KERN);
    let pitches = pitches(&document);
    assert!(pitches.contains(&60) && pitches.contains(&67), "{pitches:?}");
    assert!(pitches.iter().any(|p| *p < 55), "{pitches:?}");
}

#[test]
fn the_formats_are_listed_among_those_that_can_be_opened() {
    on_engrave::register_importers();
    let all = Document::extensions();
    for extension in ["musicxml", "mid", "abc", "krn", "mei"] {
        assert!(all.contains(&extension), "{extension} missing from {all:?}");
    }
}

#[test]
fn a_file_that_is_not_a_score_says_so_instead_of_crashing() {
    on_engrave::register_importers();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("nothing.abc");
    std::fs::write(&path, "this is not music at all").unwrap();
    assert!(Document::open(&path).is_err());
}
