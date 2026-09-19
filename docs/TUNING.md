# Tuning the engine overnight

The engine is rules and a model of a hand, and how much each rule and each kind of
strain counts against the others is a set of numbers somebody chose. `opennote tune`
searches for better ones. It tries a few settings at a time on every core, keeps
whatever agrees more often with music somebody else fingered or divided between the
hands, and writes each improvement to a file. It runs until you stop it or until its
time is up, and it can be stopped and started again at any point.

It needs nothing but the CPU. No GPU, no network once the data is on disk, and no
Claude.

## What it learns from

What it is graded against decides what it learns. More examples beats more hours: on
a dozen scores it memorises them, and the one figure it cannot see gets worse while
the others improve.

* **Hands** (`--target hands`) learns from scores whose source says which hand plays
  what: two-staff MusicXML, piano MIDI with a track per hand, or
  [PDMX](https://github.com/pnlong/PDMX). PDMX is a quarter of a million public
  domain scores, of which 21,538 are two-staff piano. `tools/pdmx_piano.py` pulls
  those out of the download; point `--scores` at the `PDMX/data` folder it leaves.
* **Fingers** (`--target fingers`) learns from the corpus, which is fingerings
  somebody wrote. `opennote corpus add` reads MusicXML with printed fingerings.
  **PDMX cannot help here.** Its JSON drops fingerings, and its original MusicXML
  barely has any: of 21,083 public domain piano scores, 35 carry a printed fingering
  at all, 1,432 marks between them. Commercially usable fingering data is the thing
  this project does not have, and no amount of searching makes up for it.

Anything tuned on PIG must never ship. PIG is licensed for academic use only, like
PianoVAM. Measure against it by all means (see [TRAINING.md](TRAINING.md)), but don't
put it in the corpus you tune from.

PDMX is CC BY 4.0, and the scores the script keeps are public domain or CC0. If
weights tuned on it become the engine's defaults, credit it in `LICENSES.md`.

### What to aim at

The dataset, and the held-out and test thirds of it in particular. Those are music the
search was not allowed to learn from, so they are the closest thing there is to a
measure of how the engine will do on a score nobody has seen — which is the point of
the exercise.

Do not tune against a handful of your own files. A dozen pieces get memorised: the
learning figure climbs while the test figure falls. And scores grabbed to test with are
often arrangements no human would play, so "worse" on them is not evidence of anything.

Two options exist for when you do want to hold a line, and both cost dataset accuracy,
so leave them alone unless you mean it:

* `--guard <pieces>` refuses any setting that does worse than the defaults on any one
  of those pieces, within a quarter of a point.
* `--min-shared 0.1` learns only from music where at least a tenth of the notes fall
  inside the other hand's usual range — the hard third of PDMX, where the hands share
  the keyboard.

## Running it

```bash
cargo build --release -p on-cli
target/release/opennote tune --target hands --scores path/to/PDMX/data
target/release/opennote tune --target fingers
```

Useful options:

* `--hours 8` stops after eight hours. Without it, it runs until you stop it.
* `--limit 5000` sets how many scores the hand search keeps. Every one is fingered
  again for every setting tried, so this is what a generation costs: more generalises
  better and moves slower. The ones kept are spread across the whole dataset rather
  than taken alphabetically.
* `--measure models/weights.json` doesn't search. It scores that file against the
  defaults on whatever `--scores` you give it, which is how to check a setting on
  music it was never tuned on, one piece at a time if you like.
* `--threads 4` sets how many settings it tries at once. It uses one per core by
  default; use fewer if you want the machine for something else.

To leave it running on Windows without it getting in the way, start it at low
priority:

```bat
start "opennote tune" /low /min target\release\opennote.exe tune --target hands --scores D:\PDMX\PDMX\data
```

To start it every time you log on, create a Task Scheduler task with the trigger "At
log on" and that same command as the action. Stopping it is safe at any moment,
whether by closing the window, Ctrl+C or turning the machine off: everything it needs
to carry on is written after every generation, and the next run picks up from there.

## How long it has run

```bash
target/release/opennote tune --report
```

```text
hands: 41.75 hours, 1802184 settings tried over 150182 generations.
  agreement with the people who wrote the music down, on the third it never learned
  from: 91.43%, against 87.65% for the engine's defaults at the time.
Altogether: 41.75 hours, 1802184 settings tried.
```

The hours are time actually spent searching, added up over every run there has ever
been, and they carry on counting as long as `out/tune/` is kept. Time with the search
stopped is not in them, so the number is one you can stand behind.

Each promotion also writes the same figures into `models/weights.json` beside the
weights, under names beginning with an underscore, so a setting carries its own
provenance:

```json
  "_hands.searched_hours": 41.75,
  "_hands.settings_tried": 1802184.0,
  "_hands.examples": 5000.0
```

Those are ignored when the weights are applied.

## Reading what it did

Everything goes under `out/tune/<target>/`:

* **`log.csv`** has one row per generation. `best_held_out` is the column to watch:
  it only rises when a setting also did better on scores the search was not allowed
  to learn from. `best_test` is scores it has never been graded on at all, and is the
  honest figure.
* **`state.json`** is where it is. Delete it to start from the engine's defaults
  again.

Each improvement is written to `models/weights.json`, and both targets share that
file. To try it:

```bash
target/release/opennote annotate score.musicxml --tuned models/weights.json -o out.musicxml
```

## Making it the default

A promoted setting is a candidate, not a result. Before its numbers become the
engine's defaults, it has to:

1. beat the defaults on `best_test` by more than noise — that is the whole case for
   it, and it is about the dataset, not about anybody's favourite piece;
2. leave `cargo test --workspace` passing, including the method-book fingerings in
   `golden_fingerings.rs`;
3. leave `anatomy` reporting zero unreachable fingers, out-of-range joints and silent
   strikes, and `jitter` reporting no steps (see
   [PORTING.md](PORTING.md#checking-a-port)). These are physical honesty, not taste: a
   setting that agrees with a corpus by asking for fingerings a hand cannot make has
   not improved anything.

`handtruth` on your own scores is worth a look afterwards, but it is a report, not a
gate. They are a handful of arrangements, and the dataset is fifteen thousand.

Then the values are copied into the `Default` implementations: `HandAssignment` in
`on-score`, and `FingeringOptions`, `RuleWeights`, `BiomechWeights` and
`StrainWeights` in `on-fingering` and `on-hand`. Handy picks them up through the
normal engine drop, with no file to ship.
