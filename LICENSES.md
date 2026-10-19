# Licences

OpenNote's own code is licensed under the **PolyForm Noncommercial License 1.0.0**;
see `LICENSE`.

It builds on other people's work. This file records what, and under what terms.

## Ported code

**`pydactyl`** — David A. Randolph, MIT.
<https://github.com/dvdrndlph/pydactyl>

The finger-span tables in `crates/on-fingering/src/spans.rs` and the rule
implementations in `crates/on-fingering/src/rules.rs` are transcribed and ported from
pydactyl's reference implementations of the Parncutt, Jacobs, Balliauw and Badgerow
models. MIT permits this; the attribution is repeated in those files' headers.

Two deliberate departures from the reference, both documented at the site and covered
by tests:

* the thumb-passing rule no longer fires when the thumb plays both notes, since a
  thumb with no finger over it is not passing under anything
* the large-span rule's left-hand test asked for rising finger numbers with rising
  pitch, which is the right hand's pattern, so the rule never fired for a left hand in
  its natural shape

## Research this implements

* Parncutt, Sloboda, Clarke, Raekallio & Desain, *An Ergonomic Model of Keyboard
  Fingering for Melodic Fragments*, Music Perception 14(4), 1997
* Jacobs, *Refinements to the Ergonomic Model for Keyboard Fingering*, 2001
* Balliauw, Herremans, Palhazi Cuervo & Sörensen, *A variable neighborhood search
  algorithm to generate piano fingerings for polyphonic sheet music*
* Buryanov & Kotiuk, *Proportions of Hand Segments*, Int. J. Morphol. 28(3), 2010 —
  the measured bone lengths the hand model is built on
* Nakamura, Saito & Yoshii, *Statistical Learning and Estimation of Piano Fingering*,
  Information Sciences 517, 2020 — the statistical formulation `on-train` follows, and
  the match rates `opennote eval` reports
* Fletcher & Rossing, *The Physics of Musical Instruments*, 2nd ed., ch. 12 — the
  string model the synthesiser in `on-audio` is built on
* Qian, Urain, Zakka & Peters, *PianoMime: Learning a Generalist, Dexterous Piano
  Player from Internet Demonstrations*, 2024 — the video-to-fingering method
  `on-train::video` reimplements: a homography onto the keyboard, then nearest-finger
  assignment against the notes that are sounding
* Zakka et al., *RoboPianist: Dexterous Piano Playing with Deep Reinforcement
  Learning*, 2023 (Apache-2.0) — cross-checked the keyboard's dimensions and where on
  a white key a finger actually strikes it

## Dependencies of note

| Project | Licence | How it is used |
|---|---|---|
| [Verovio](https://www.verovio.org/) | LGPL-3.0 | Engraving, through the `verovioxide` bindings. Linked, not vendored. |
| [Bevy](https://bevy.org/) | MIT or Apache-2.0 | The visualizer. |
| [`musicxml`](https://crates.io/crates/musicxml) | MIT | Reading and writing MusicXML. |
| [`midly`](https://crates.io/crates/midly) | MIT | Reading and writing MIDI. |
| [`svg2pdf`](https://crates.io/crates/svg2pdf), [`resvg`](https://crates.io/crates/resvg) | MIT or Apache-2.0 | Turning engraved SVG into PDF and PNG. |
| [`bevy_egui`](https://crates.io/crates/bevy_egui), [`egui`](https://crates.io/crates/egui) | MIT or Apache-2.0 | The menu bar and transport bar. |
| [`rfd`](https://crates.io/crates/rfd) | MIT | The native file dialogs. |
| [`cpal`](https://crates.io/crates/cpal) | Apache-2.0 | Getting the synthesiser to the sound card. |
| [`rustysynth`](https://crates.io/crates/rustysynth) | MIT | Playing a SoundFont, when one is supplied. |

**ffmpeg** is not a dependency and is not bundled. `opennote render` runs whatever
ffmpeg is on the machine as a separate process, so its licence — LGPL or GPL depending
on how that copy was built — applies to that copy, not to this program. Nothing here
downloads it.

The installers include the **Studio Yamaha C7** SoundFont, which OpenNote plays by default. It is
licensed under the **Free Art License 1.3**, whose text is in
`assets/soundfont/LICENCE-Free-Art-1.3.txt` (official: <https://artlibre.org/licence/lal/en/>). It is not in the
repository: `opennote soundfont install` downloads it and checks its checksum, and the release
workflow does the same before packaging. A build without it plays a model of a struck string, so
there are no samples and no sample licence.

Any other SoundFont you supply is played through `rustysynth` (MIT). What that asks of you depends
on which one you chose. Check its licence before you publish audio made with it; whether audio
rendered from a sampled instrument is a derivative work is a question for a lawyer and not for this
file. `on-audio`'s modelled piano uses no samples and avoids it entirely.
`assets/soundfont/*.sf2` is gitignored, so a downloaded instrument stays in your working copy.

The interface is set in **Nunito**, under the SIL Open Font License 1.1, whose text is in
`assets/fonts/OFL-Nunito.txt`. The font files are embedded in the program.

The full dependency tree and its licences: `cargo tree` and `cargo about`.

## The hand model

`assets/hands/hand-left.glb` and `hand-right.glb` are generated from MakeHuman's base
mesh, rig and skin through MPFB2, which MakeHuman releases under **CC0**: no attribution
is required and any use is fine. `assets/hands/PROVENANCE.md` records how they were made.
If you replace them, record the new ones there and honour their terms; some alternatives
are CC-BY and need visible attribution.

Do not use MANO, SMPL-X or NIMBLE unless your use really is non-commercial academic
research; their licences are limited to that.

## Training data

The engine's weights are searched against **PDMX**, and the numbers that ship were
found with it:

> Long, P., Novack, Z., Berg-Kirkpatrick, T. and McAuley, J. (2025). *PDMX: A
> Large-Scale Public Domain MusicXML Dataset for Symbolic Music Processing.*
> <https://github.com/pnlong/PDMX>

PDMX is licensed **CC BY 4.0**, which asks for attribution and nothing else, and every
one of its 254,077 scores is public domain (210,364) or CC0 (43,713) in its own right —
its authors filtered for that when they built it. So this attribution is what using it
requires, and it is given gladly: without it the engine would be tuned against a handful
of files somebody happened to have.

Nothing from PDMX is redistributed here. What a search takes out of a score is a number:
how often the engine agreed with the hand division the score was written with, or how
often the fingering it produced could be played. `tools/pdmx_piano.py` pulls the piano
scores out of a copy you download yourself.

The PIG dataset (Nakamura et al.) is free but **registration-gated and licensed for
nonprofit academic use only**. Nothing derived from it is included here. `opennote
corpus add` reads a copy you obtained yourself; `corpus/` is gitignored, nothing in
this repository downloads PIG, and a model trained on it is yours and stays local.

The same goes for any edition you mark up yourself: the corpus is your data, kept in
your working copy.

## Sample scores

`samples/` is generated by `tools/make_samples.py` from note lists. The music —
Beethoven's Bagatelle in A minor, and exercises — is public domain.
