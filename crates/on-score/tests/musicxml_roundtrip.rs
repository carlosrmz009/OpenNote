//! End-to-end check that a two-staff piano score survives import, annotation and
//! export with its fingerings attached and its own content intact.

use on_hand::{Finger, Hand};
use on_score::{Fingering, MusicXmlDocument};

/// Two bars of a two-staff piano part: a right-hand line over left-hand chords,
/// with a tie, a chord, and a dynamic to prove nothing musical gets dropped.
const SCORE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE score-partwise PUBLIC "-//Recordare//DTD MusicXML 4.0 Partwise//EN" "http://www.musicxml.org/dtds/partwise.dtd">
<score-partwise version="4.0">
  <work><work-title>Round Trip</work-title></work>
  <part-list><score-part id="P1"><part-name>Piano</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1">
      <attributes>
        <divisions>2</divisions>
        <key><fifths>0</fifths></key>
        <time><beats>4</beats><beat-type>4</beat-type></time>
        <staves>2</staves>
        <clef number="1"><sign>G</sign><line>2</line></clef>
        <clef number="2"><sign>F</sign><line>4</line></clef>
      </attributes>
      <direction placement="below"><direction-type><dynamics><p/></dynamics></direction-type><staff>2</staff></direction>
      <note><pitch><step>C</step><octave>4</octave></pitch><duration>2</duration><voice>1</voice><type>quarter</type><staff>1</staff></note>
      <note><pitch><step>D</step><octave>4</octave></pitch><duration>2</duration><voice>1</voice><type>quarter</type><staff>1</staff></note>
      <note><pitch><step>E</step><octave>4</octave></pitch><duration>2</duration><voice>1</voice><type>quarter</type><staff>1</staff></note>
      <note><pitch><step>F</step><octave>4</octave></pitch><duration>2</duration><voice>1</voice><type>quarter</type><staff>1</staff></note>
      <backup><duration>8</duration></backup>
      <note><pitch><step>C</step><octave>3</octave></pitch><duration>4</duration><voice>5</voice><type>half</type><staff>2</staff></note>
      <note><chord/><pitch><step>G</step><octave>3</octave></pitch><duration>4</duration><voice>5</voice><type>half</type><staff>2</staff></note>
      <note><pitch><step>B</step><octave>2</octave></pitch><duration>4</duration><tie type="start"/><voice>5</voice><type>half</type><staff>2</staff><notations><tied type="start"/></notations></note>
    </measure>
    <measure number="2">
      <note><pitch><step>G</step><octave>4</octave></pitch><duration>8</duration><voice>1</voice><type>whole</type><staff>1</staff></note>
      <backup><duration>8</duration></backup>
      <note><pitch><step>B</step><octave>2</octave></pitch><duration>8</duration><tie type="stop"/><voice>5</voice><type>whole</type><staff>2</staff><notations><tied type="stop"/></notations></note>
    </measure>
  </part>
</score-partwise>
"#;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("opennote-tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn load() -> MusicXmlDocument {
    let path = scratch("roundtrip-in.musicxml");
    std::fs::write(&path, SCORE).unwrap();
    MusicXmlDocument::read(&path).expect("the fixture should parse")
}

#[test]
fn a_two_staff_part_imports_with_timing_staves_and_ties() {
    let doc = load();
    let score = doc.score();

    assert_eq!(score.title.as_deref(), Some("Round Trip"));
    // Four right-hand quarters, three left-hand notes in bar 1, two notes in bar 2.
    assert_eq!(score.notes.len(), 9);

    let ppq = on_score::TICKS_PER_QUARTER as i64;

    // The backup element must rewind the cursor so the left hand starts at bar 1
    // beat 1 rather than after the right hand's four quarters.
    let lowest = score.notes.iter().find(|n| n.midi == 48).unwrap();
    assert_eq!(lowest.onset, 0, "the left-hand chord should start the bar");
    assert_eq!(lowest.duration, 2 * ppq, "a half note is two quarters");

    // The chord member shares its onset and is flagged as a chord note.
    let g3 = score.notes.iter().find(|n| n.midi == 55).unwrap();
    assert_eq!(g3.onset, 0);
    assert!(g3.chord);

    // Bar 2 begins one whole bar in.
    let g4 = score.notes.iter().find(|n| n.midi == 67).unwrap();
    assert_eq!(g4.onset, 4 * ppq);

    // The tie is seen from both ends.
    let tied: Vec<_> = score.notes.iter().filter(|n| n.midi == 47).collect();
    assert_eq!(tied.len(), 2);
    assert!(tied[0].tie.start && !tied[0].tie.stop);
    assert!(tied[1].tie.stop && tied[1].tie.is_continuation());
}

#[test]
fn staff_numbers_decide_the_hands() {
    let mut doc = load();
    on_score::assign_hands(doc.score_mut(), &Default::default());
    for note in &doc.score().notes {
        let expected = if note.staff == Some(1) { Hand::Right } else { Hand::Left };
        assert_eq!(note.hand, Some(expected), "midi {} on staff {:?}", note.midi, note.staff);
    }
}

#[test]
fn fingerings_are_written_back_onto_the_original_notes() {
    let mut doc = load();
    on_score::assign_hands(doc.score_mut(), &Default::default());

    // Right hand C-D-E-F takes 1-2-3-4; the left-hand chord takes 5 and 2.
    let plan: Vec<Fingering> = doc
        .score()
        .notes
        .iter()
        .map(|n| {
            let finger = match n.midi {
                60 => Finger::Thumb,
                62 => Finger::Index,
                64 => Finger::Middle,
                65 => Finger::Ring,
                67 => Finger::Little,
                48 => Finger::Little,
                55 => Finger::Index,
                _ => Finger::Middle,
            };
            Fingering::new(n.id, finger)
        })
        .collect();

    let written = doc.annotate(&plan).unwrap();
    assert_eq!(written, plan.len(), "every note should have been annotated");

    let out = scratch("roundtrip-out.musicxml");
    doc.write(&out, false).unwrap();

    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains("<fingering"), "no fingerings in the output");
    // Nothing else should have been lost on the way through.
    assert!(text.contains("Round Trip"));
    assert!(text.contains("<backup>"));
    assert!(text.contains("<chord"));
    assert!(text.contains("tie"));

    // Reading the annotated file back must recover the same fingerings, which is
    // what makes an annotated score usable as training data later.
    let reloaded = MusicXmlDocument::read(&out).unwrap();
    assert_eq!(reloaded.score().notes.len(), 9);
    for note in &reloaded.score().notes {
        assert!(
            note.given_finger.is_some(),
            "midi {} came back without its fingering",
            note.midi
        );
    }
    let c4 = reloaded.score().notes.iter().find(|n| n.midi == 60).unwrap();
    assert_eq!(c4.given_finger, Some(Finger::Thumb));
}

#[test]
fn annotating_twice_replaces_rather_than_duplicates() {
    let mut doc = load();
    on_score::assign_hands(doc.score_mut(), &Default::default());
    let ids: Vec<_> = doc.score().notes.iter().map(|n| n.id).collect();

    doc.annotate(&[Fingering::new(ids[0], Finger::Thumb)]).unwrap();
    doc.annotate(&[Fingering::new(ids[0], Finger::Ring)]).unwrap();

    let out = scratch("roundtrip-twice.musicxml");
    doc.write(&out, false).unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    assert_eq!(text.matches("<fingering").count(), 1, "fingering was duplicated");
    assert!(text.contains(">4<"), "the second fingering should have replaced the first");
}

#[test]
fn compressed_mxl_round_trips() {
    let mut doc = load();
    on_score::assign_hands(doc.score_mut(), &Default::default());
    let ids: Vec<_> = doc.score().notes.iter().map(|n| n.id).collect();
    doc.annotate(&[Fingering::new(ids[0], Finger::Middle)]).unwrap();

    let out = scratch("roundtrip.mxl");
    doc.write(&out, true).unwrap();
    let reloaded = MusicXmlDocument::read(&out).unwrap();
    assert_eq!(reloaded.score().notes.len(), 9);
}
