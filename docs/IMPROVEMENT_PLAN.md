# OpenNote improvement plan

## Context

OpenNote is a Rust workspace of about 32k lines in 8 crates (`on-score`, `on-hand`, `on-fingering`, `on-audio`, `on-train`, `on-engrave`, `on-viz`, `on-cli`). It fingers piano scores using:
- published rule sets (Parncutt, Jacobs, Balliauw, Badgerow);
- an IK-based biomechanical cost;
- a learned prior/reranker (the sealed `helios-beta1.8` model).

It renders the result with a Bevy 0.19 viewer (3D hands, falling notes) and exports video and engraved PDFs.

The engine is ambitious and well tested in places: 333 tests pass, including finite-difference Jacobian checks and golden fingerings. Around that core:
- **No CI.** `cargo fmt --check` fails.
- **One flaky test.** `on-score/tests/musicxml_marks.rs` has two tests that write and read the same `/tmp/opennote-marks-<pid>.musicxml` in parallel. It failed on the first run.
- **Almost no comments or rustdoc** in on-score, on-hand, on-fingering and on-viz.
- **The README is 6 lines.** The help text points at docs that don't exist.
- **Several real bugs** in import, engraving, solver error handling and the viewer's reload path.
- **The git pack is 1.33 GiB**, almost all of it showcase MP4s re-committed for each model version.
- **Rendering realism is limited by the assets and the pipeline, not by the shader code.** The hand GLB has no textures (one flat material; PROVENANCE.md wrongly claims a baked 4096px texture). Skin is procedural vertex colour with sine "mottle". Tonemapping is off, while emissive values run 4–9.5×.

**Goal:** better fingerings on real scores, a codebase that is safe to change, a better viewer and better output, credible distribution, and noticeably more realistic piano, notes and skin.

**Decisions already made:**
- Repository size is fixed going forward only, with no history rewrite.
- The model and training pipeline stay private. Sealing stays, but builds become reproducible and the help text stops pointing at files that aren't there.

Work is grouped into phases. Each phase is a series of small PRs on `claude/focused-gates-bj62je` (or branches cut from it), in order. Phase 0 must land first, because everything after it relies on CI.

---

## Phase 0: Safety net (CI, formatting, flaky test)

1. **Fix the flaky test.** In `crates/on-score/tests/musicxml_marks.rs:28`, give each test its own path. Add `tempfile` as a dev-dependency and use `tempfile::NamedTempFile`. Apply the same fix to the other fixed temp paths:
   - `on-fingering/src/prior.rs:641`
   - `on-train/src/corpus.rs:412,468,507,533,606,635`
   - `on-train/src/bench.rs:587,646`
   - `on-audio/src/wav.rs:254`
2. **Formatting.** Add `rustfmt.toml` matching the current style (the code uses ~110-column lines: check before choosing `max_width`). Run `cargo fmt --all` in a commit of its own.
3. **CI.** Add `.github/workflows/ci.yml`:
   - `fmt --check`;
   - `clippy --workspace --all-targets -D warnings`, starting from a baseline `[workspace.lints]` in `Cargo.toml` with temporary allows where needed;
   - `cargo test --workspace` on Linux, with the Bevy system deps installed;
   - the MSRV job, `cargo +1.85 check`;
   - `cargo check --examples` so the ~25 diagnostic examples stay compiling.
   - Also test on Windows (the dev machine appears to be Windows: `.ps1`/`.bat` tooling).
   - Use `Swatinem/rust-cache`.
4. **Advisories and licences.** Add `cargo-deny` (`deny.toml`), and a CI check that `NOTICE.md` is current (`cargo about generate about.hbs` diffed against the committed file).
5. **Dependency hygiene.**
   - Remove unused dependencies: `thiserror` in on-score, on-hand and on-fingering (or start using it, see 2.3); `toml` in on-fingering; `tracing` in on-score.
   - Set `bevy` to `default-features = false` with only the features used; bevy_audio isn't needed because audio goes through cpal.

## Phase 1: Correctness bugs (highest value per line)

These are verified or near-certain bugs. Each gets a regression test.

| # | Bug | Location | Fix |
|---|---|---|---|
| 1 | Octave constant is 164 mm; the real value is 7×23.5 = 164.5 | `on-fingering/src/biomech.rs:17`, `playability.rs:11` | Use `on_hand::keyboard::WHITE_KEY_WIDTH * 7.0`. Re-run the benchmark, because costs shift slightly |
| 2 | Solver threads use `join().ok()`, so a panic silently drops a whole hand or a voter | `on-fingering/src/solver.rs:217,358` | Propagate (`resume_unwind`) or return `Result`. Never return a half-solution silently |
| 3 | Engraving an `.mxl` output fails (`read_to_string` on a zip) | `on-engrave/src/lib.rs:122` | Read through `on_score` (handles `.mxl`), or pass bytes to Verovio |
| 4 | Grace notes share the principal note's onset, so they're fingered as a chord; grace notes inside chords use the wrong `last_onset` | `on-score/src/musicxml_io.rs:338-340,479` | Give grace notes a small negative time offset (acciaccatura) or steal time (appoggiatura). The solver treats them as separate events |
| 5 | Tick truncation per duration drifts when `divisions` doesn't divide 960, which splits chords | `musicxml_io.rs:215-220` | Keep the cursor in divisions (a rational) and convert once per onset |
| 6 | `<backup>` can push ticks negative, so `to_smf` wraps them | `musicxml_io.rs`, `midi_io.rs:260` | Clamp to the measure start, and use a saturating delta |
| 7 | MIDI pedal state is shared across tracks while ticks reset per track | `on-score/src/midi_io.rs:120,152-166` | Track pedal state per track. Also filter channel 10 |
| 8 | Chords of more than 5 notes, or pins that can't be met together, fall back to `spread_fingers`, which reuses fingers and ignores pins | `solver.rs:577,1066` | Split the excess to the other hand or roll it, warn, and keep pins where possible |
| 9 | `Context::from_key` panics on an empty or non-ASCII key in a model file | `on-fingering/src/prior.rs:48` | Return an error. Add a fuzz/property test for `NgramPrior::from_json` |
| 10 | Global `RwLock` caches `.unwrap()`; one poisoned lock breaks every later solve | `biomech.rs:122-301` | Use `parking_lot` or recover from poisoning. Replace clear-at-200k with an LRU |
| 11 | `eval` loads the model with `NgramPrior::load` and ignores the built-in model, model names, `none` and `--style` | `on-cli/src/learn.rs:808-813` | Go through `ModelArgs::prior()` |
| 12 | `eval::paired` `.expect` panics when two runs cover different pieces | `on-train/src/eval.rs:252` | Return an error |
| 13 | `corpus add --name ../x` escapes the corpus directory | `on-train/src/corpus.rs:166` | Reuse the sanitiser from `add_piece` |
| 14 | `tune` silently overwrites state or weights it can't parse | `on-train/src/tune.rs:799,1020,720` | Fail loudly, or move the bad file aside as `.bak` |
| 15 | A missing `\` continuation prints ~18 stray spaces | `on-cli/src/learn.rs:1117` | Add the `\` |
| 16 | Viewer reload leaks quads (octave lines, case, hit line; additive blending, so they get brighter) and doesn't rebuild flashes | `on-viz/src/render.rs:277-288,604-652` | Add a `SceneEntity` marker on everything `setup_*` spawns and despawn by marker. Call `setup_flashes` on rebuild |
| 17 | Dragging the lookahead slider runs a full reload (re-solve) on every change | `on-viz/src/ui.rs:239-247` | Lookahead is a view parameter: update the layout only |
| 18 | Changing hand size doesn't rescale the hand meshes | `on-viz/src/hands.rs:401-407` | Recompute rig scale when the animator's profile changes |
| 19 | Exporting from the UI drops hand size, SoundFont, speed and colours | `on-viz/src/ui.rs:415,474-494` | Serialize the whole `SessionSettings` into args |
| 20 | Keyboard shortcuts fire while egui has focus (Space double-toggles) | `render.rs:1142` | Check `EguiContexts::ctx().wants_keyboard_input()` |
| 21 | U16 audio output wraps for \|s\|>1; the `.sf2` is loaded inside the audio callback | `on-audio/src/output.rs:164,224` | Clamp; load the SoundFont off-thread and swap it in through a channel |
| 22 | Models are downloaded without a timeout, partial files are left on disk, and a 404 is detected by string matching | `on-cli/src/models.rs:67-121` | Add `--connect-timeout/--max-time`, download to `.part` and rename, check a sha256 published next to the release |

**Check intent with the maintainer, not a straight fix:** with the built-in model, `prior_scale` stays 0 (`main.rs:119`, `solver.rs:766`), so the n-gram part of helios never contributes. Either that is intended (helios is a reranker) and the `--prior-scale` help should say so, or it's a bug.

## Phase 2: Code health and architecture

1. **Split the giant files** without changing behaviour. Each split is its own PR, verified by tests and a benchmark scorecard that is byte-identical.
   - `on-cli/src/learn.rs` (1136 lines, 7 subcommands) → `cmd/{corpus,train,eval,rerank,tune,bench,benchmark}.rs`.
   - `on-viz/src/render.rs` (1523) → `camera`, `keyboard`, `notes`, `fx` (sparks, flashes, lights), `transport`, `app`.
   - `on-viz/src/timeline.rs` (2447, ~1100 of it tests) → `grips`, `animator`, `decisions`, plus `tests/`.
   - `on-train/src/tune.rs::run` (~340 lines) and `rerank::train` → a state machine and helpers.
   - `on-fingering/src/solver.rs` → `events.rs` (`build_events`, `hold_sustained`), `viterbi.rs`, `assemble.rs`.
2. **Remove duplication.**
   - In `on-fingering`:
     - the four `RuleScorer::score*` loops (`rules.rs:408-464`);
     - `balliauw_position_comfort` ≡ `position_change_size`;
     - `Grip::cache_key` vs `normalised`;
     - `scales.rs:110` reimplements `is_black`;
     - `CHORD_SECONDS` is defined twice.
   - In `on-score`: `truth_hands` vs `assign_from_*` (with different thresholds).
   - In `on-viz`: a shared `math.rs` for smoothstep, the `(1-u²)³` kernel, `minimum_jerk` and `bump`; `KEY_LEAD`/`KEY_BEFORE_SOUND` and `TOGETHER` are duplicated.
   - Hashes and RNGs: FNV and SplitMix64 are duplicated across `on-train` and `on-cli`. Make one `on-train::hash`/`rng` module.
   - Load → assign hands → solve is duplicated in `annotate.rs`, `explain.rs`, `learn.rs` and six viz examples. Make one `on_fingering::pipeline::finger_document()` used everywhere. The examples currently call `finger_score_consensus` while the app calls `finger_score_with_prior`.
   - Add `Note::new` / a builder and a `test-support` module. The 15-field `Note` literal is copied about 20 times.
3. **Errors.** Library crates get typed `thiserror` errors (`ScoreError`, `ModelError`). `anyhow` stays in `on-cli` only. Stop flattening `musicxml` errors to strings.
4. **Visibility.** Narrow `pub` to `pub(crate)` for solver internals (`Event`, `Candidate`, `build_events`), span tables and `dof` indices. Replace the global `set_inner_threads` toggle with a field in `FingeringOptions`, or a rayon pool.
5. **Remove dead code.**
   - `score_excluding`, `RuleScorer::total`, `set_weights`, `strain::worst_finger`, `Torso::preference`, `staff_histogram`;
   - the glove code and `assets/gloves/*.png` (1.2 MB, unused);
   - the `ON_MOTION=v1` path, once v2 is confirmed;
   - `spikes/verovio/`, superseded by on-engrave.
6. **Name the magic numbers.** Collect each crate's tuning constants into one `consts.rs` / `Weights` struct with a one-line doc on each, including its source paper or "tuned by `opennote tune`". This includes the 13 thresholds in `known.rs` and the bit layouts in `known.rs`/`learned.rs`. Reconcile constants that disagree: wrist height 45 vs 60 mm, gap 0.02 vs 0.03 s.
7. **Documentation in code.**
   - Write `//!` crate and module docs and `///` on every public item. Turn on `#![warn(missing_docs)]` crate by crate.
   - Cite the papers at each rule, span table and cost term.
   - Restore the MIT attribution headers that `LICENSES.md:15-19` promises in `rules.rs`/`spans.rs`, and document the two departures it mentions. This is a licence-compliance item.

## Phase 3: Fingering accuracy

1. **A real-repertoire regression harness.**
   - Golden tests over `samples/*.musicxml`: snapshot the fingering with `insta` and diff it in review.
   - Add a small public corpus of editorially fingered public-domain pieces, and wire `opennote benchmark` into CI as an opt-in job that compares against the committed scorecard. The model stays private: CI uses rules-only, or a test model, plus a local script for the sealed one.
2. **Evaluation hygiene.**
   - `rerank` currently picks the best pass on the test split and reports gain on that same split (`rerank.rs:405-420`). Add a validation split.
   - `tune` prints test-bucket scores at every promotion. Hide them until the end.
   - Make results independent of core count: fix the batch and generation sizes (`rerank.rs:327`, `tune.rs:888`) and hash canonical paths (`tune.rs:289`).
   - Store the held-out fraction, corpus hash, options and commit in the model's metadata, and have `eval` read them.
   - `eval` assumes every annotation of a piece has an identical note list. Align by (onset, pitch) instead of by index.
3. **Model and solver improvements**, each measured on the benchmark before merging:
   - Detect scales and runs *before* `hold_sustained`, so legato overlaps in recorded MIDI don't hide scales.
   - Allow the same finger on adjacent keys (a thumb taking two keys, e.g. a white–black pair), and finger substitution as a real search move rather than only a penalty. `Fingering.substitute` is never set.
   - Hand assignment that can represent crossing hands: switch from a pitch split point to a per-note hand state in the Viterbi. Honour `voice`, cross-staff notation and channel-per-hand MIDI. Allow a 3-staff piano and companion parts; today a non-piano part is merged in.
   - Taught patterns: add melodic minor, chromatic in both hands, double-note thirds and sixths, and broken-chord/Alberti figures.
   - Accumulate costs in `f64`, or normalise each Viterbi row by its minimum (constants of 800/60 in `f32` over thousands of steps).
4. **Performance.** Faster solves let `tune` explore more, and that improves accuracy.
   - Hoist `event_cost(i,c)` out of the `b` loop (`solver.rs:857`). Precompute `move_cost` per (b,c) pair.
   - Remove per-call `Vec`s in `grip()`, `GripKey`, the `Learned::*` lookups and `ik::build_rows`, which also computes the Jacobian 3× per row (`ik.rs:241-252`).
   - Fix the hidden O(N²) paths:
     - `Solution::finger_of` → a map;
     - `Score::release_seconds`;
     - `keep_repeated_notes_in_one_hand`;
     - `TempoMap::seconds_at` → prefix sums plus binary search, which matters because expression inserts a tempo change every 16th.
   - Replace the nested `thread::scope` (4 sets × 2 hands) with rayon. Turn the `cost.rs` example into a `criterion` benchmark and track it in CI.
   - The default consensus path runs 5 full solves. Cache the per-rule-set votes by score hash for repeated calls (viewer reload, explain).

## Phase 4: User experience

**CLI (`on-cli`):**
- **Consistent flags:**
  - use `-o/--output` everywhere for output files, with `--state-dir` for tune;
  - separate input and output flags in `rerank`;
  - `--tuned` in, `--save-weights` out;
  - one meaning for `--measure` and `--explain`;
  - `--threads` on every heavy command;
  - add a `--color` alias.
- **Rename `bench` and `benchmark`** to `measure` and `scorecard`, with hidden aliases for the old names.
- **Split `ModelArgs`:** `train` shouldn't show `--model`, `--style` or `--prior-scale`.
- **Validate inputs** with clap `value_parser` ranges (`--crf 0..=51`, fps/width > 0, `--speed`) and remove the silent clamps.
- **New features:**
  - batch `annotate` (several inputs or a glob), with `-` for stdin/stdout;
  - `--format json|csv` for `explain`, `eval` and `measure`;
  - `--force`, and refuse to overwrite otherwise;
  - `-q/--quiet`, and `-v` that turns on debug for `opennote` crates only;
  - progress bars with `indicatif` on stderr, with results on stdout;
  - shell completions and a man page (`clap_complete`, `clap_mangen`);
  - a config file `~/.config/opennote/config.toml` for hand length, rules, assets and SoundFont;
  - `--keep-editorial` on annotate;
  - engraving options: page size, scale and PNG DPI.
- **Write-back fidelity:**
  - MusicXML `substitution="yes"` instead of the text "3-1";
  - place by staff, not by hand;
  - avoid duplicate `<fingering>` elements;
  - bundle a music text font for headless engraving instead of relying on "Times New Roman".

**Viewer (`on-viz`):**
- **Practice features:**
  - A–B loop and loop by measure (bar lines already come from `score.marks.measures`);
  - a tempo slider in the transport bar showing the current speed (the menu radios disagree with the ↑/↓ keys);
  - a practice ramp;
  - mute, hide or dim each hand;
  - step by note and by measure;
  - a count-in.
- **Score sync:** show bar and beat numbers on the timeline; later, a Verovio-rendered score strip with a cursor (on-engrave already produces SVG).
- **Controls:**
  - a help overlay (`?`);
  - Ctrl+O, fullscreen, drag-and-drop, recent files;
  - make the seek slider match `LEAD_IN`;
  - a camera orbit/zoom preset or two.
- **Settings and loading:** persist `SessionSettings`; run loading on a background task with a spinner (today it blocks the UI thread); show errors in a toast or panel instead of the menu bar.
- **Accessibility:**
  - a colour-blind-safe hand palette preset and colour pickers in the UI;
  - reduced-motion / reduced-effects mode (sparks, flashes, bloom);
  - a quality preset (Low/Medium/High) for weaker GPUs;
  - UI scale.
- **Viewer performance:**
  - one shared unit-quad mesh for notes, with size per instance (today each note has its own mesh);
  - GPU or instanced sparks instead of 24k entities;
  - compute `key_depression()` once per frame and index notes by time;
  - only `get_mut` key materials that changed (today all ~88 every frame);
  - remove per-frame `Vec` and `String` allocations;
  - move `Decisions`/`Lag` precomputation off the main thread.
- **Video export:**
  - a headless offscreen render with no window and no vsync;
  - pipelined GPU readback;
  - a bounded frame channel;
  - progress and cancel in the UI;
  - clean up the temporary WAV on failure.

**Audio:** make the cpal callback real-time safe (no allocation or boxing in the callback, `output.rs:197-222`), add device selection, stop the thread when `Player` is dropped, and put a limiter on the SoundFont path.

## Phase 5: Piano, notes and skin realism

The rendering is defined by `crates/on-viz/src/{render.rs,hands.rs,skin.rs}` and `{hand,key,note}.wgsl`. Do it in this order; each step is checked by the golden frames in 5.8.

1. **Colour pipeline first.** Today `Tonemapping::None` (`render.rs:376`) is combined with HDR bloom and emissive values of 4–9.5. That clips and shifts hues, and `hand.wgsl:52-57` hand-rolls a highlight roll-off to compensate.
   - Switch to `Tonemapping::AgX` or `TonyMcMapface` with an explicit `Exposure`.
   - Recalibrate the note, flash and spark emissives to physically plausible ratios.
   - Delete the custom roll-off in `hand.wgsl`.
   - Every later step depends on this.
2. **Environment lighting.**
   - Replace the hard-coded "studio" function in `key.wgsl` and its fake `LACQUER_EYE_MM` eye with a real environment: an `EnvironmentMapLight` built from a small, CC0 studio HDRI (the Poly Haven type), plus a reflection light probe over the keyboard.
   - For an orthographic camera, the view vector is the camera's forward direction, not `eye - world_pos`. Fix that before or instead of the swap.
3. **Piano keys.**
   - **Black keys:** use `StandardMaterial` clearcoat (`clearcoat`, `clearcoat_perceptual_roughness`) for the lacquer instead of the extension shader. That gives correct layered Fresnel against the environment.
   - **Ivory:** give the procedural `skin::white_key()` texture fine grain, a slight warm translucency and per-key variation (hash the key index to shift tint and roughness by ±2%).
   - Generate mipmaps for the procedural textures (none today) and drop the main-world copies.
   - **Key geometry:** keep the rounded keys; add a felt strip and the key slip and cheek blocks in the case so keys don't float.
   - **Shadows:** make sure hands cast shadows onto the keys with contact shadows (Bevy 0.19 supports contact shadows on directional lights). Shadows the hands drop onto keys are the strongest single realism cue in a top-down view.
4. **Falling notes (`note.wgsl`).**
   - The facet and glint pattern is keyed on `world_position.y`, so it slides over the falling bar. Key it on the bar's local `uv`/`uv_b`.
   - Notes are alpha/additive with no motion vectors, so TAA smears them. Either render notes in a pass excluded from TAA, or switch the default AA to SMAA/MSAA for this scene and keep TAA only as an option.
   - Glassy look: a thin-film or Fresnel edge highlight that reflects the environment map, a soft inner gradient by velocity, and a hit "squash" glow where the bar meets the key line. The flare already exists; reduce it once tonemapping is correct.
   - Note glow on the hands: the 10 point lights already move with notes. Make the skin respond to them (step 5.5).
5. **Skin.** This is the biggest gap. Today the hand is 31.8k vertices, with one flat material and no textures (verified in the GLB). The colour comes from `complexion()` vertex colours (`hands.rs:491-608`) with three-sine "mottle", and the meshes are `NotShadowReceiver`.
   - **Assets.** Re-export through `tools/export_hand.py` with MakeHuman's CC0 skin, baking:
     - albedo, 2k per hand;
     - a tangent-space normal map (knuckle wrinkles, palm lines, nail edges);
     - a roughness map (oilier fingertips, matte back of hand);
     - a cavity/AO map.
     The GLB already carries `TANGENT`s. Update `assets/hands/PROVENANCE.md` to match what is actually shipped; it's wrong today.
   - **Detail.** Add a tileable micro-normal (pores) with mipmaps, sampled at high UV scale and faded by distance, in `hand.wgsl`. Keep `complexion()` only as a fallback tint and region-mask generator (nail mask, knuckle redness).
   - **Subsurface.** Keep the wrap-lighting term, but loop over clustered point lights as well as directional lights (`hand.wgsl:38-46`), so coloured note light bleeds red through the skin.
     - Add a thickness term from a baked thickness map (or the existing `diffuse_transmission`/`thickness`). That gives backlit translucency at finger edges.
     - Optionally use Bevy's screen-space SSS if 0.19 exposes it on `StandardMaterial`. Otherwise keep the custom term, and calibrate the per-channel scatter radius (R > G > B) against reference photos.
   - **Specular.** Use two specular lobes, a sharp oily one and a broad one (the standard skin approach), driven by the roughness map. Fingernails get their own roughness and clearcoat from the nail mask.
   - **Receive shadows.** Remove `NotShadowReceiver` and fix shadow acne with normal bias, so fingers shadow each other.
   - **Skin-tone presets** (several Fitzpatrick types) in the View menu, by moving the albedo and scatter tint. This makes the viewer inclusive and is cheap once albedo comes from a texture.
   - **Deformation.** Check the knuckles for candy-wrapper collapse under bend. Add corrective blend shapes or dual-quaternion skinning if Bevy supports it; otherwise add helper bones in the export script.
6. **Scene.**
   - A dark felt or wood fallboard and a key-bed backdrop instead of a void.
   - Subtle depth of field (optional, Video preset only).
   - Film grain or vignette only in the Video preset.
7. **Quality presets.** Low / Medium / High / Video control:
   - shadow-map size (4096 today);
   - SSAO, TAA and bloom;
   - the micro-normal;
   - the light count.
   Export defaults to Video.
8. **Visual regression.** Add a headless golden-frame test that uses the existing `capture` path (`render.rs:1417`):
   - render 3–4 fixed timestamps of `samples/fur-elise.musicxml`;
   - compare with a perceptual tolerance (FLIP or SSIM);
   - run in CI on lavapipe/llvmpipe (software Vulkan).
   Keep a side-by-side before/after sheet in each realism PR.

## Phase 6: Docs and distribution

1. **README rewrite:**
   - what OpenNote does and how (rules + biomechanics + learned model);
   - a GIF or still of the viewer, with links to the showcase videos in Releases;
   - install from release binaries, and building from source:
     - Rust 1.85+;
     - Linux packages: `pkg-config libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev` plus a C++ toolchain for verovio;
     - `xdg-desktop-portal` for file dialogs;
     - ffmpeg for `render`;
   - a quick start on `samples/*.musicxml` (`annotate`, `explain`, `play`, `render`);
   - a crate architecture diagram;
   - hand-size guidance;
   - the licence (PolyForm Noncommercial) in plain words.
2. **Fix dangling references.**
   - Write a public `docs/TUNING.md` (usage only), or remove the reference at `main.rs:47`.
   - Rewrite help text that names private tools (`tools/watch_pianist.py`, `harvest.py`, `docs/TRAINING.md`, `learn.rs:38,51,674,705`, `video.rs:127`) so it says the pipeline is private rather than pointing at missing files.
   - Add `assets/hands/README.md` and `assets/soundfont/README.md` (where to get the YDP grand and how it's picked up), both referenced in code.
   - Fix `LICENSES.md`, which says the hand model "is not included" although it is.
   - Record which SoundFont and which sources the showcase pieces used. Confirm the copyright status of non-public-domain showcase pieces ("tomandjerry", "tarnation", "grief") before republishing them.
3. **Repository size, going forward only.**
   - Remove `showcases/*.mp4` from the tree. Publish them as assets of a `showcases-<model>` GitHub Release and link them from the README.
   - Add `.gitattributes`, and a CI check that rejects blobs over 5 MB outside `assets/`.
   - Delete `spikes/verovio/`, root `glover.svg` (unused; the README uses a user-attachments URL) and `assets/gloves/`.
   - The history stays 1.33 GiB, as decided. A shallow-clone tip goes in the README.
4. **Releases.**
   - A `release.yml` workflow that builds Windows, macOS and Linux binaries on tag, with the sealed model injected from CI secrets (`OPENNOTE_MODEL` content, `OPENNOTE_RELEASE_KEY`).
   - Publish `.onm` models with sha256 sums, which `models get` verifies (1.22).
   - Version-pinned model tags instead of a single mutable `models` release.
5. **Sealing stays private, but make it robust** (`on-cli/build.rs`, `seal.rs`, `models.rs`):
   - **Reproducible builds:** derive the per-build key deterministically, e.g. from a hash of the model bytes plus `OPENNOTE_RELEASE_KEY`, instead of `SystemTime::now()` (`build.rs:23`).
   - Emit `cargo:warning` when no model is found. Always emit `rerun-if-changed` for the model path, so adding the file later triggers a rebuild.
   - Warn when `OPENNOTE_RELEASE_KEY` / `release-key.txt` doesn't parse, instead of `.ok()`.
   - Add a version byte and a checksum (MAC) to the `ONM1` header, so corruption is detected properly instead of by "invalid UTF-8".
   - Use platform data directories (`dirs` crate) in `models::dir()`. Make `models list` ignore `.part` files.
6. **LGPL obligations (Verovio, statically linked).** Document the relinking route (object files or build instructions) in `LICENSES.md`, or link verovio dynamically for release builds.

## Order and sizing

| Phase | Depends on | Rough size |
|---|---|---|
| 0 Safety net | none | small, 1–2 PRs |
| 1 Bugs | 0 | 22 small PRs, or 5 grouped by crate |
| 2 Code health | 0; split files before deep refactors | medium |
| 3 Accuracy | 1 (bugs #1, #2, #4, #5, #8 affect scores), the 2.2 pipeline, the 3.1 harness | large, iterative |
| 4 UX | 1 (#16–21), the 2.1 render split | large |
| 5 Realism | 5.1 first; the 5.8 harness early; the asset re-export (5.5) needs Blender/MPFB2 locally | large |
| 6 Docs and distribution | 6.1–6.3 can start at once; 6.4–6.5 after 0 | medium |

Phases 3, 4 and 5 can run in parallel once 0–2 have landed.

## Verification

- **Each PR:** CI green (fmt, clippy `-D warnings`, `cargo test --workspace` on Linux and Windows, MSRV, examples compile, cargo-deny), and a new regression test for every bug fixed in Phase 1.
- **Refactor PRs (Phase 2):** `opennote benchmark` scorecard and `insta` fingering snapshots identical before and after; `criterion` numbers not worse.
- **Accuracy PRs (Phase 3):** scorecard deltas reported in the PR on the validation split. The test split is read only at release.
- **Rendering PRs (Phase 5):** the golden-frame test passes within tolerance after deliberately updating the references, plus a before/after image sheet. Manually run `opennote play samples/fur-elise.musicxml` and `opennote render samples/fur-elise.musicxml -o out.mp4`, and check the FPS overlay at each quality preset.
- **UX:** end-to-end CLI tests with `assert_cmd` on `samples/*.musicxml` (annotate → `.musicxml`/`.mxl`/PDF, explain `--format json`, `measure`); `Cli::command().debug_assert()`.
- **Distribution:** a release dry-run on a tag in a fork. The binaries start, `models list` shows the built-in model, and two builds of the same commit with the same secrets are byte-identical.
