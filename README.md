<img src="https://github.com/user-attachments/assets/53d3272c-c627-475f-b998-71ded27c1448" alt="glover" style="max-width: 100%; height: auto; width: 400px;" />

"glover" =Ↄ

# OpenNote

OpenNote works out piano fingerings for a score and writes them back onto it. It can
also play the piece with falling notes and two animated 3D hands, render that as a video,
and engrave the fingered score as a PDF.

A fun project that aims to test out the capabilities of opus 5 ;)

## How it fingers a piece

1. **Read the score.** MusicXML (`.musicxml`, `.mxl`, `.xml`) or MIDI (`.mid`, `.midi`).
   Fingerings already written in the score are kept.
2. **Split it between the hands.** It uses the staves, or two MIDI tracks; failing both,
   it searches for where to split each moment of the piece.
3. **Search each hand's fingering.** A Viterbi search weighs:
   - **Published rule sets:** Parncutt et al. (1997), Jacobs (2001), Balliauw et al. and
     Badgerow, by default combined by how many of them endorse each rule.
   - **A biomechanical model of the hand:** the posture and the effort of moving between
     notes, measured in millimetres on a real keyboard and scaled to your hand size.
   - **A trained model, `helios-beta1.8`:** learned from fingerings pianists wrote and
     played. It is built into official builds and is off with `--model none`.
4. **Explain it.** `opennote explain` shows, note by note, why each finger won and how
   close the runner-up came.

## Showcases

Six pieces played by `helios-beta1.8`, in [`showcases/`](showcases/).

## Install

### Build from source

You need:
- **Rust:** 1.95 or later for the whole program. The engine crates alone build on 1.85.
- **A C++ compiler and CMake:** Verovio, which engraves the scores, is compiled from source
  on the first build. That takes several minutes, and needs internet access to fetch its
  source, or `VEROVIO_SOURCE_DIR` pointing at a checkout of Verovio 5.7.0.
- **On Linux, system libraries for audio, input and windowing:**

  ```sh
  sudo apt install pkg-config libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev
  ```

  File dialogs in the viewer go through `xdg-desktop-portal`.
- **To render videos,** [ffmpeg](https://ffmpeg.org/) on your `PATH` (or pass `--ffmpeg`).

Then:

```sh
cargo build --release
./target/release/opennote --help
```

A build from a clone has no trained model built in, so it fingers with the rules and the
hand model alone; `opennote models list` says which model a build carries.

### Sound

The viewer and the videos use a modelled piano by default. For a sampled piano, put a
SoundFont (`.sf2`) in `assets/soundfont/`, or pass `--soundfont`. The YDP Grand Piano,
whose licence is in that folder, works well.

## Use

```sh
# Write the fingering into a copy of the score (samples/fur-elise-fingered.musicxml),
# and engrave it.
opennote annotate samples/fur-elise.musicxml --engrave fur-elise.pdf

# Why each note got its finger; only the close calls.
opennote explain samples/fur-elise.musicxml --close-calls

# Watch it played, with falling notes and 3D hands.
opennote play samples/fur-elise.musicxml

# The same, as a video.
opennote render samples/fur-elise.musicxml -o fur-elise.mp4
```

The fingering is chosen for a medium hand. Measure yours once, from the crease of the
wrist to the tip of the middle finger, and pass it as `--hand-length <mm>`. Alternatively,
pick `--hand-size xs|small|medium|large|xl`.

In the viewer:

| Key | Does |
|---|---|
| Space | play / pause |
| ← / → | back / forward two seconds |
| Home | back to the start |
| ↑ / ↓ | faster / slower |
| L | loop on / off |
| [ / ] | practise from here / up to here |
| \\ | play the whole piece again |

The transport bar marks a section to practise (A and B) and sets the speed. The menus
open other scores, change the hand size, the sound and the view, and export the video or
the fingered score.

### Measuring the fingering

`opennote corpus`, `train`, `eval`, `rerank`, `tune`, `bench` and `benchmark` collect
expert fingerings, train models on them and measure the results. The datasets and most
of the training pipeline are not part of this repository.

## The code

| Crate | What it does |
|---|---|
| `on-score` | The score: MusicXML and MIDI in and out, tempo, splitting between the hands, expression |
| `on-hand` | The hand: keyboard geometry, a 22-joint skeleton, inverse kinematics, strain |
| `on-fingering` | The fingering search: rules, biomechanical cost, the trained model, explanations |
| `on-train` | Corpora, training, evaluation, tuning and the benchmark |
| `on-engrave` | Writing fingerings into the score and engraving it with Verovio |
| `on-audio` | The modelled piano, SoundFont playback, audio output and WAV rendering |
| `on-viz` | The Bevy viewer: falling notes, the keyboard, the animated hands, video export |
| `on-cli` | The `opennote` command |

```sh
cargo test --workspace
```

## Licence

OpenNote is under the [PolyForm Noncommercial License 1.0.0](LICENSE): you may use,
change and share it for any noncommercial purpose. [`LICENSES.md`](LICENSES.md) covers
the research it builds on and the assets it ships, and [`NOTICE.md`](NOTICE.md) lists the
licences of its dependencies.
