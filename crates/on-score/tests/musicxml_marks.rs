use on_score::{art, MusicXmlDocument};

const SCORE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE score-partwise PUBLIC "-//Recordare//DTD MusicXML 4.0 Partwise//EN" "http://www.musicxml.org/dtds/partwise.dtd">
<score-partwise version="4.0">
  <part-list><score-part id="P1"><part-name>Piano</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1">
      <attributes><divisions>1</divisions><time><beats>4</beats><beat-type>4</beat-type></time></attributes>
      <direction><direction-type><dynamics><pp/></dynamics></direction-type></direction>
      <note><pitch><step>C</step><octave>4</octave></pitch><duration>1</duration><voice>1</voice><type>quarter</type><notations><slur type="start" number="1"/></notations></note>
      <direction><direction-type><wedge type="crescendo"/></direction-type></direction>
      <note><pitch><step>D</step><octave>4</octave></pitch><duration>1</duration><voice>1</voice><type>quarter</type></note>
      <note><pitch><step>E</step><octave>4</octave></pitch><duration>1</duration><voice>1</voice><type>quarter</type><notations><slur type="stop" number="1"/></notations></note>
      <direction><direction-type><wedge type="stop"/></direction-type></direction>
      <note><pitch><step>F</step><octave>4</octave></pitch><duration>1</duration><voice>1</voice><type>quarter</type><notations><articulations><staccato/></articulations></notations></note>
    </measure>
    <measure number="2">
      <direction><direction-type><dynamics><f/></dynamics></direction-type></direction>
      <note><pitch><step>G</step><octave>4</octave></pitch><duration>2</duration><voice>1</voice><type>half</type><notations><articulations><accent/></articulations></notations></note>
      <note><pitch><step>C</step><octave>5</octave></pitch><duration>2</duration><voice>1</voice><type>half</type><notations><fermata/></notations></note>
    </measure>
  </part>
</score-partwise>
"#;

fn load() -> MusicXmlDocument {
    let path = std::env::temp_dir().join(format!("opennote-marks-{}.musicxml", std::process::id()));
    std::fs::write(&path, SCORE).unwrap();
    MusicXmlDocument::read(&path).expect("the fixture should parse")
}

#[test]
fn printed_dynamics_set_how_hard_the_notes_are_played() {
    let doc = load();
    let score = doc.score();
    let at = |midi: u8| score.notes.iter().find(|n| n.midi == midi).unwrap();
    assert_eq!(at(60).velocity, 40, "pp");
    assert_eq!(at(67).velocity, 90, "f");
    assert_eq!(score.marks.dynamics.len(), 2);
}

#[test]
fn hairpins_slurs_and_articulations_are_kept() {
    let doc = load();
    let score = doc.score();
    let q = on_score::TICKS_PER_QUARTER as i64;
    assert_eq!(score.marks.hairpins, vec![(q, 3 * q, 1)]);
    assert_eq!(score.marks.slurs, vec![(0, 2 * q)]);
    assert_eq!(score.marks.measures, vec![0, 4 * q]);
    let flags = |midi: u8| score.marks.articulation(score.notes.iter().find(|n| n.midi == midi).unwrap().source);
    assert_eq!(flags(65), art::STACCATO);
    assert_eq!(flags(67), art::ACCENT);
    assert_eq!(flags(72), art::FERMATA);
    assert_eq!(flags(60), 0);
}
