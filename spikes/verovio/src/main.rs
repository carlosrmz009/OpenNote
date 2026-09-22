//! Spike: can verovioxide engrave a MusicXML score that carries <fingering>?

const SCORE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE score-partwise PUBLIC "-//Recordare//DTD MusicXML 4.0 Partwise//EN" "http://www.musicxml.org/dtds/partwise.dtd">
<score-partwise version="4.0">
  <part-list><score-part id="P1"><part-name>Piano</part-name></score-part></part-list>
  <part id="P1">
    <measure number="1">
      <attributes>
        <divisions>1</divisions>
        <key><fifths>0</fifths></key>
        <time><beats>4</beats><beat-type>4</beat-type></time>
        <clef><sign>G</sign><line>2</line></clef>
      </attributes>
      <note><pitch><step>C</step><octave>4</octave></pitch><duration>1</duration><type>quarter</type>
        <notations><technical><fingering>1</fingering></technical></notations></note>
      <note><pitch><step>D</step><octave>4</octave></pitch><duration>1</duration><type>quarter</type>
        <notations><technical><fingering>2</fingering></technical></notations></note>
      <note><pitch><step>E</step><octave>4</octave></pitch><duration>1</duration><type>quarter</type>
        <notations><technical><fingering>3</fingering></technical></notations></note>
      <note><pitch><step>F</step><octave>4</octave></pitch><duration>1</duration><type>quarter</type>
        <notations><technical><fingering>1</fingering></technical></notations></note>
    </measure>
  </part>
</score-partwise>
"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut tk = verovioxide::Toolkit::new()?;
    println!("verovio version: {}", tk.version());

    tk.load_data(SCORE)?;
    println!("pages: {}", tk.page_count());

    let svg = tk.render_to_svg(1)?;
    println!("svg bytes: {}", svg.len());
    std::fs::write("out.svg", &svg)?;

    // Verovio marks each engraved fingering with @class="fing"; if those are
    // present the digits made it through MusicXML import into the engraving.
    let fings = svg.matches("fing").count();
    println!("fingering marks in svg: {fings}");

    // Does it also give us PDF straight out?
    match tk.render_to("out.pdf") {
        Ok(()) => println!("pdf: ok ({} bytes)", std::fs::metadata("out.pdf")?.len()),
        Err(e) => println!("pdf: unsupported ({e})"),
    }
    match tk.render_to("out.png") {
        Ok(()) => println!("png: ok ({} bytes)", std::fs::metadata("out.png")?.len()),
        Err(e) => println!("png: unsupported ({e})"),
    }

    if fings == 0 {
        eprintln!("FAIL: fingerings did not survive into the engraving");
        std::process::exit(1);
    }
    println!("OK");
    Ok(())
}
