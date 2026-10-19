<img src="https://github.com/user-attachments/assets/53d3272c-c627-475f-b998-71ded27c1448" alt="glover" style="max-width: 100%; height: auto; width: 400px;" />

"glover" =D

# OpenNote

OpenNote works out piano fingerings for a score and writes them back onto it. It can
also play the piece with falling notes and two animated 3D hands, render that as a video,
and engrave the fingered score as a PDF.


## How it fingers a piece

1. **Read the score.** MusicXML (`.musicxml`, `.mxl`, `.xml`), MIDI (`.mid`, `.midi`, `.kar`), or
   ABC (`.abc`), Humdrum (`.krn`, `.hum`), MEI (`.mei`) and Plaine & Easie (`.pae`) through Verovio.
   Fingerings already written in a MusicXML score are kept.
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

### Download (recommended)

Get the installer for your computer. These links always give the newest version, and
[all releases](https://github.com/carlosrmz009/OpenNote/releases) are kept. Nothing else
needs installing.

| You have | Download | Then |
|---|---|---|
| **Windows** 10 or 11 | [OpenNote-windows-setup.exe](https://github.com/carlosrmz009/OpenNote/releases/latest/download/OpenNote-windows-setup.exe) | Double-click it and follow the steps. Open **OpenNote** from the Start menu. |
| **Mac** with an Apple chip (M1 or later) | [OpenNote-macos-arm64.dmg](https://github.com/carlosrmz009/OpenNote/releases/latest/download/OpenNote-macos-arm64.dmg) | Open it and drag **OpenNote** onto **Applications**. |
| **Mac** with an Intel chip | [OpenNote-macos-intel.dmg](https://github.com/carlosrmz009/OpenNote/releases/latest/download/OpenNote-macos-intel.dmg) | The same. |
| **Linux** | [OpenNote-linux-x86_64.AppImage](https://github.com/carlosrmz009/OpenNote/releases/latest/download/OpenNote-linux-x86_64.AppImage) | Make it executable (right-click > Properties > "Allow executing as a program", or `chmod +x`) and double-click it. |

Not sure which Mac you have? Apple menu > About This Mac: "Chip: Apple…" is the first row
above, "Processor: Intel…" the second.

The first time you open it, your computer may warn you, because the installers are not signed
by a registered developer:

- **Windows:** "Windows protected your PC" > **More info** > **Run anyway**.
- **Mac:** right-click (or Control-click) **OpenNote** in Applications > **Open** > **Open**.
  After that it opens normally.
- **Linux:** the first time you open the AppImage, OpenNote offers to add itself to your applications menu, with an optional desktop icon. If the AppImage will not start, install FUSE (`sudo apt install libfuse2`), or run
  it with `--appimage-extract-and-run`.

### Using it

The first time you open OpenNote, a short welcome shows how it works. It comes back from **Help > Welcome**.

1. Open OpenNote.
2. Press **Select piece…** and choose a score: MusicXML (`.musicxml`, `.mxl`), MIDI (`.mid`), ABC (`.abc`),
   Humdrum (`.krn`) or MEI (`.mei`).
   Free scores can be found on [MuseScore](https://musescore.com) (download as MusicXML) and
   [IMSLP](https://imslp.org).
3. Watch the hands play it, with the finger numbers on the notes.

Before you choose a piece you can pick a colour for each hand's notes, and how
**expressive** the playing should be (a beta: it shapes timing and touch rather than
playing exactly as written). The bar along the bottom is the progress of the piece:
click or drag anywhere on it to jump there, hold **Shift** while dragging to move slowly,
or use the mouse wheel. To practise a passage, press **A** at its start and **B** at its
end (or right-click the bar), then drag the A and B flags to fine-tune it; it repeats until
you press **Clear**.

### Updates

When a newer OpenNote or a newer model has been published, a popup appears as the app
opens. Press the yellow **Update!** button: it downloads the update, checks it, installs it
and restarts. **Not now** keeps what you have, and **Help > Check for updates…** asks again.
Set `OPENNOTE_NO_UPDATE_CHECK=1` to stop it looking.

### Making a video

Press **Make video…** on the bar at the bottom (or File > Make video…). Choose the whole piece or
just your practice section, a quality, and a speed, and press **Make video**. Each video starts with
a two-second "Made with OpenNote" card, which you can switch off in the same window. When it is
done, **Play it** or **Show in folder**.

Videos are put together by a free helper, ffmpeg. If your computer does not have it, OpenNote offers
to download it for you on Windows and Linux. On a Mac, install it once with
[Homebrew](https://brew.sh): `brew install ffmpeg`.

### Build from source

Only needed to change OpenNote, or if there is no installer for your system.

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
./target/release/opennote          # the viewer, which asks for a piece
./target/release/opennote --help   # everything it can do
```

A build from a clone has no trained model built in, so it fingers with the rules and the
hand model alone; `opennote models list` says which model a build carries.

### Sound

The installers include a sampled grand piano, **Studio Yamaha C7**, so the viewer and your videos
sound like a real piano. It is included with its owner's permission.

A build from source does not carry it, so get it once with:

```sh
opennote soundfont install
```

Or put any other SoundFont (`.sf2`) in `assets/soundfont/`, or pass `--soundfont`. Without one,
OpenNote plays a modelled piano that uses no samples.

## Use

```sh
# Write the fingering into a copy of the score (samples/fur-elise-fingered.musicxml),
# and engrave it. Run again with --force to replace the copy.
opennote annotate samples/fur-elise.musicxml --engrave fur-elise.pdf

# Several scores at once, each written beside itself.
opennote annotate scores/*.musicxml

# Why each note got its finger; only the close calls.
opennote explain samples/fur-elise.musicxml --close-calls

# Open the viewer and choose a piece, or give it one.
opennote
opennote play samples/fur-elise.musicxml

# The same, as a video: add --no-intro to leave out the card at the start, or
# --from 10 --to 25 for just those seconds.
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
| , / . | pause and step back / on a note |
| [ / ] | practise from here / up to here |
| \\ | play the whole piece again |
| F1 or ? | all of these |

The transport bar marks a section to practise (A and B) and sets the speed, and
Playback > Hands played leaves one hand to you: its notes still fall, but it is not
played or heard. The menus
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
