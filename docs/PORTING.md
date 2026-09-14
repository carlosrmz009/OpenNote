# Porting OpenNote into an application

For an engineer starting a new app that uses OpenNote as its engine, and who wants to
keep taking fixes from it afterwards.

OpenNote is MIT OR Apache-2.0. You may ship it in a closed commercial product. Read
[Licensing](#licensing) before you ship anything, because two of the things *around* the
engine are not as permissive as the engine is.

## What you take

The crates come apart into a half that works out what is happening and a half that puts
it on a desktop screen with Bevy. The first half is what you port.

| | crate | lines | take it? |
| --- | --- | --- | --- |
| Anatomy, kinematics, strain | `on-hand` | 3 100 | yes |
| MIDI and MusicXML, hand assignment | `on-score` | 2 500 | yes |
| Fingering search, biomechanics, playability | `on-fingering` | 7 000 | yes |
| What is happening at time *t* | `on-viz` (`timeline`, `layout`, `rig`) | 3 200 | yes |
| Drawing it | `on-viz` (`render`, `ui`, `export`, `skin`, `glove`) | 4 800 | no — Bevy |
| Sound | `on-audio` | 1 300 | maybe — see below |
| Training the statistical prior | `on-train` | 2 400 | no |
| Engraving to PDF | `on-engrave` | 340 | **no — LGPL** |

That is about 15 800 lines you keep and never edit, and 50 crates of dependencies. The
Bevy half is 371 crates, which is the other reason not to bring it.

There is no model file. The solver is rules and biomechanics, in code. The optional
statistical prior is trained locally and is the only data artefact, and you almost
certainly cannot ship one — again, see [Licensing](#licensing).

## Depending on it

Your app has a Rust layer even if your app is not a Rust app. That layer depends on
OpenNote by tag, and taking a fix upstream is bumping the tag:

```toml
[dependencies]
on-hand      = { git = "https://github.com/carlosrmz009/OpenNote", tag = "v0.1.0" }
on-score     = { git = "https://github.com/carlosrmz009/OpenNote", tag = "v0.1.0" }
on-fingering = { git = "https://github.com/carlosrmz009/OpenNote", tag = "v0.1.0" }
on-viz       = { git = "https://github.com/carlosrmz009/OpenNote", tag = "v0.1.0", default-features = false }
```

`default-features = false` on `on-viz` is what leaves Bevy out. Do not omit it: the
default feature is the desktop renderer.

Pin a tag rather than a branch. You want to choose when the engine changes under you.

## The seam

`crates/on-viz/examples/frame.rs` is the whole interface, and it runs:

```bash
cargo run --release -p on-viz --no-default-features --example frame -- score.mid 8.0
```

A frame is three calls. Everything is in millimetres, in one space shared by the keys,
the falling notes and the hands, so a hand that spans an octave on a real piano spans an
octave on screen.

```rust
// Once, when a score is loaded.
on_score::assign_hands(document.score_mut(), &assignment);
let solution = on_fingering::finger_score_consensus(score, &options, None);
let timeline = Timeline::build_for(score, &solution.fingerings, &options.profile);
let animators = timeline.animators(&options.profile, BiomechWeights::default());
let layout = Layout::for_pitches(timeline.notes.iter().map(|n| n.midi), Extent::FullKeyboard);

// Every frame, at time `at`.
let keys = timeline.key_depression(at);          // which keys are down, how far, whose
let notes = timeline.visible_notes(at, 3.0);     // what to draw falling
let poses = pose_both(&animators, at);           // both hands' postures
```

`layout.key_rect(midi)` and `layout.note_rect(midi, start - at, duration)` turn those
into rectangles. `animator.skeleton().forward(&pose)` turns a posture into world joint
positions: `posture.wrist`, and `posture.chain[digit][joint]` for five digits of four
joints each, the last of which is the fingertip.

### Axes

* `x` runs along the keyboard, low notes negative.
* `y` runs along the keys towards the player. The keyboard occupies `-150..0`; the lane
  the notes fall down is `y > 0`, so notes arrive at `y = 0`, the far end of the keys.
* `z` is height, with the top of a white key at zero. A pressed key is 10 mm down.

### Both hands together

`pose_both` takes both hands rather than one. That is deliberate and not a convenience:
whether a hand has to be lifted over the other, or the wrists angled apart, is the one
thing about a hand's posture that hand cannot decide by itself. Calling `pose_at` per
hand instead will draw the hands through each other.

## What you build

**The renderer.** `render.rs` is 1 900 lines of Bevy scene graph and is the part that does
not port. Rewrite it against `wgpu`, which is what Bevy draws through anyway, targets
Metal and Vulkan/GLES, and — unlike Bevy — is built to draw into a surface the host hands
it. That is the embedding a phone app needs.

**The shell.** Bevy is the wrong shape for a consumer app: it wants to own the window and
the event loop, and its UI toolkit is a developer tool. Every screen that is not the
piano roll would be a fight. Use a UI framework for the app and give the piano roll a
surface.

The risky part is compositing your `wgpu` output into that framework. Spike it before
writing any product code. Native shells (SwiftUI, Jetpack Compose) make it easy —
handing a `CAMetalLayer` or a `SurfaceView` to Rust is well trodden — at the cost of
writing the UI twice.

**Audio.** `on-audio` wraps `rustysynth`, which is pure Rust and portable, and `cpal`,
which is shakier on mobile. Take the synthesiser, supply your own output, and build it
with `default-features = false` to leave `cpal` out.

## Keeping up with upstream

Bump the tag and read the commit messages; they say what moved and what it measured.

What moves most is the hand-assignment search and the animation in `timeline.rs` — the
`pose_both` contract and the collision handling changed several times in one week. What
has been stable is the shape of the data: `TimelineNote`, `KeyStates`, `Posture`.

Check your port still holds after a bump by running the diagnostics below. They build
without a GPU, so they run in CI.

## Checking a port

The engine carries its own harnesses, and they are the reason to trust it. Both run in
the no-GPU configuration.

```bash
# Physical honesty: does every finger reach its key, is every joint inside its range,
# are the hands ever drawn inside one another, does a hand go through the keyboard.
cargo run --release -p on-viz --no-default-features --example anatomy -- score.mid

# Does the hand-assignment search agree with whoever wrote the music down? Point it at
# engraved MusicXML or a two-track piano MIDI.
cargo run --release -p on-score --example handtruth -- score.musicxml score.mid
```

`anatomy` should report zero for unreachable fingers, out-of-range joints and silent
strikes on any score. Collisions are not zero and are not meant to be; what matters is
the ratio it prints against no handling at all.

`handtruth` sits around 92% and is a movement detector, not a score: a change that sends
it down has broken something.

If you change nothing in the engine, both should read exactly as they do upstream. That
is the point of them.

### Guarding the boundary

The no-GPU configuration once stopped compiling for weeks because nothing asked for it.
One command, in your CI or upstream's:

```bash
cargo check -p on-viz --no-default-features --all-targets
```

## Licensing

The engine and everything under it is permissive — 50 crates, no copyleft. These three
are the traps:

* **`on-engrave` is LGPL-3.0** by way of Verovio. Nothing in the engine depends on it, so
  leaving it out costs nothing. Do not add it to a closed product without understanding
  what LGPL asks of you.
* **Do not ship a prior trained on PIG.** The standard corpus (Nakamura, Saito & Yoshii)
  is registration-gated and licensed for nonprofit academic use only; PianoVAM is
  CC BY-NC. A trained prior is a derivative of the corpus it learned from and carries its
  licence. Today there is no prior file at all, so the question does not arise — it
  arises the moment someone trains one. If you want a shipped prior, train it on a corpus
  you are licensed to use commercially.
* **SoundFonts are licensed separately** from the synthesiser that plays them. `rustysynth`
  is permissive; the `.sf2` files are usually not, and none are committed here.

## Getting started

```bash
git clone https://github.com/carlosrmz009/OpenNote
cd OpenNote
cargo run --release -p on-viz --no-default-features --example frame -- path/to/score.mid 8.0
```

That builds 50 crates, no graphics stack, and prints a frame. If it prints keys, note
rectangles and two hands' worth of joints, you have everything the renderer needs and the
rest is drawing.
