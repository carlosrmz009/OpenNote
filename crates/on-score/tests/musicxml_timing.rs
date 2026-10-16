use on_score::{MusicXmlDocument, Score, TICKS_PER_QUARTER};

fn read(measures: &str, divisions: u32) -> Score {
    let text = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<score-partwise version="4.0">
  <part-list><score-part id="P1"><part-name>Piano</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1">
      <attributes><divisions>{divisions}</divisions><time><beats>4</beats><beat-type>4</beat-type></time></attributes>
      {measures}
  </part>
</score-partwise>"#
    );
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("timing.musicxml");
    std::fs::write(&path, text).unwrap();
    MusicXmlDocument::read(&path).expect("the fixture should parse").score().clone()
}

fn note(step: char, octave: u8, duration: u32, voice: u8, extra: &str) -> String {
    format!(
        "<note>{extra}<pitch><step>{step}</step><octave>{octave}</octave></pitch>\
         <duration>{duration}</duration><voice>{voice}</voice></note>"
    )
}

fn grace(step: char, octave: u8, chord: bool) -> String {
    let chord = if chord { "<chord/>" } else { "" };
    format!("<note><grace slash=\"yes\"/>{chord}<pitch><step>{step}</step><octave>{octave}</octave></pitch><voice>1</voice></note>")
}

fn onset(score: &Score, midi: u8) -> i64 {
    score.notes.iter().find(|n| n.midi == midi).unwrap_or_else(|| panic!("no note {midi}")).onset
}

#[test]
fn triplets_and_a_plain_beat_meet_on_the_next_beat() {
    // Sibelius writes 256 divisions to the quarter, so a triplet eighth is 85 or 86 of
    // them, which is not a whole number of ticks.
    let triplets = [85, 85, 86]
        .iter()
        .zip(['C', 'D', 'E'])
        .map(|(d, step)| note(step, 5, *d, 1, ""))
        .collect::<String>();
    let measures = format!(
        "{triplets}{}<backup><duration>512</duration></backup>{}{}</measure>",
        note('F', 5, 256, 1, ""),
        note('C', 3, 256, 2, ""),
        note('G', 3, 256, 2, ""),
    );
    let score = read(&measures, 256);
    let q = TICKS_PER_QUARTER as i64;
    assert_eq!(onset(&score, 77), q, "F5 after the triplets");
    assert_eq!(onset(&score, 55), q, "G3 after the plain beat");
}

#[test]
fn a_grace_note_comes_just_before_the_note_it_leads_into() {
    let measures = format!("{}{}{}</measure>", note('C', 4, 1, 1, ""), grace('E', 4, false), note('D', 4, 1, 1, ""));
    let score = read(&measures, 1);
    let (grace, principal) = (onset(&score, 64), onset(&score, 62));
    assert!(grace < principal, "grace at {grace}, principal at {principal}");
    assert!(grace > onset(&score, 60), "the grace note is after the note before it");
}

#[test]
fn grace_notes_written_as_a_chord_are_played_together_and_in_order() {
    let measures = format!(
        "{}{}{}{}{}</measure>",
        note('C', 4, 1, 1, ""),
        grace('E', 4, false),
        grace('G', 4, true),
        grace('A', 4, false),
        note('D', 4, 1, 1, ""),
    );
    let score = read(&measures, 1);
    let (first, second, principal) = (onset(&score, 64), onset(&score, 69), onset(&score, 62));
    assert_eq!(onset(&score, 67), first, "the grace chord");
    assert!(first < second && second < principal, "{first}, {second}, {principal}");
}

#[test]
fn backing_up_past_the_start_of_the_measure_stops_there() {
    let measures = format!(
        "{}<backup><duration>8</duration></backup>{}</measure>",
        note('C', 4, 4, 1, ""),
        note('C', 3, 4, 2, ""),
    );
    let score = read(&measures, 1);
    assert_eq!(onset(&score, 48), 0);
}
