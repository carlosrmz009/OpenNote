# Tuning the engine overnight

The engine is rules and a model of a hand, and how much each rule and each kind of
strain counts against the others is a set of numbers somebody chose. `opennote tune`
searches for better ones. It tries a few settings at a time on every core, keeps
whatever does better on music the search was not allowed to learn from, and writes each
improvement to a file. It runs until you stop it or until its time is up, and it can be
stopped and started again at any point.

It needs nothing but the CPU. No GPU, no network once the data is on disk, and no
Claude.

## What it learns from

What it is graded against decides what it learns. More examples beats more hours: on a
dozen scores it memorises them, and the one figure it cannot see gets worse while the
others improve. There are four things to search for, and they differ only in what the
answer is compared against.

* **Hands** (`--target hands`) — which hand plays each note, against scores whose
  source says: two-staff MusicXML, piano MIDI with a track per hand, or
  [PDMX](https://github.com/pnlong/PDMX). This is the target with real ground truth in
  quantity, and a score can only provide it by having two staves. PDMX has 21,538 of
  those out of 254,077; 14,936 survive de-duplication and a 100-note minimum, and that
  is the honest ceiling for this target. `tools/pdmx_piano.py` pulls the piano scores
  out of the download; point `--scores` at the `PDMX/data` folder it leaves.
* **Play** (`--target play`) — which finger plays each note, against what a hand can
  actually do. It fingers the scores and then measures the result: how often it asks a
  hand to be somewhere it cannot get to in the time the music allows, and how often it
  asks for a stretch beyond a comfortable one. Both are measured afterwards, against the
  hand model and the span tables.

  The two faults are counted against the number of hand shapes the music asked for,
  not against the number of notes. That matters more than it sounds: a fault can only
  happen where the hand takes a shape or moves between two, and most notes are neither,
  so counting against notes diluted the measure about five times over and left the
  engine's defaults reading 99.55% — less than half a point for a search to find.

  This one does not need a hand split in the source — it needs *a* split, and takes the
  engine's own where the score does not say, exactly as Handy does for a MIDI file
  somebody drops in. So it uses the single-staff piano scores too: **51,774** rather
  than 14,936, and the ones it gains are the harder, more realistic case.

  This is not the same thing as the engine's own cost function, which is a guess at the
  answer made note by note with a short view ahead. Tuning the guess against the
  outcome is the point, and there is something to learn precisely because the two
  differ. It is also not a substitute for a pianist's judgement: it says a fingering can
  be played, not that a teacher would write it. That is what the scale gate below is
  for.
* **Taught** (`--target taught`) — which finger plays each note, against the fingerings
  that are taught. The material is generated rather than collected: scales, major and
  chromatic, in a random key, hand, register, span, direction and speed. Their
  fingerings have a settled answer — that is what makes them exercises — so the labels
  survive being generated, which labels cut out of real music do not. A fingering in a
  piece is right *for the passage around it*, and shuffling passages keeps the label
  while throwing away what made it right.

  It needs no dataset at all. Nothing to download, nothing to license, and the held-out
  and test thirds are exercises the search genuinely never sees, since which third a
  seed falls in is fixed before any of them are built.

  **It is graded with the engine's own scale rule switched off.** The engine can be
  handed the taught fingering by a rule, and grading it with that rule on measures the
  rule rather than the engine — every setting scores alike. With it off the question is
  whether the model of the hand, left to itself, arrives where a century of teaching
  arrived. The defaults manage 71%, the largest gap between where this engine is and
  where it could be that anything here has found.

  What it cannot do is teach taste. A teacher departs from the textbook constantly for
  musical reasons, and an exercise is not repertoire, so this belongs alongside the
  other targets and never instead of them.
* **Fingers** (`--target fingers`) — which finger plays each note, against fingerings a
  pianist actually wrote, from the corpus. `opennote corpus add` reads MusicXML with
  printed fingerings. **There is almost no such data.** PDMX's JSON drops fingerings,
  and its original MusicXML barely has any: of 21,083 public-domain piano scores, 35
  carry a printed fingering at all, 1,432 marks between them. Commercially usable
  fingering data is the thing this project does not have, and no amount of searching
  makes up for it. `--target play` and `--target taught` both exist because of this.

**The two fingering targets pull on the same knobs from opposite ends.** Agreeing with a
taught fingering buys thumb crossings that a measure of comfort alone would avoid: nine
minutes of searching `taught` took it from 71.9% to 82.3% and moved `play` from 94.8%
down to 92.9%. Run in turn with nothing to stop them they do not converge — they take
turns undoing each other, and a week of that ends where it started.

So each is told what the other says and may not spend it. A promotion has to improve the
target being searched *and* leave the other within a quarter point of where the engine's
defaults had it. `play` always gets this, since exercises cost nothing to make; `taught`
gets it when you pass `--scores`, and says so plainly when you do not. A setting refused
this way is marked `crossed` in the log, and the ten-minute summary counts them.

Adding a family of exercises — arpeggios, broken chords, octaves, double thirds — is a
matter of adding its table to `on-fingering`'s `scales.rs`, **from a source**, after
which the generator randomises over it like the rest. It is deliberately not done by
guesswork: a reference table invented to fill the gap would teach the engine whatever
was invented, and nothing in the figures afterwards would say so.

Anything tuned on PIG must never ship. PIG is licensed for academic use only, like
PianoVAM. Measure against it by all means (see [TRAINING.md](TRAINING.md)), but don't
put it in the corpus you tune from.

PDMX itself is no such problem. It is CC BY 4.0, and every one of its scores is public
domain or CC0 in its own right — there is no non-commercial or no-derivatives material
in it to filter out, and `tools/pdmx_piano.py` filters only for quality. The attribution
it asks for is in `LICENSES.md`. One flag in its metadata is deliberately not enforced:
for about 12% of the dataset, what MuseScore's page says about the copyright and what
the file itself says disagree, and honouring that flag would leave 2,241 two-staff
scores instead of 14,936. Since both claims are public domain, and since nothing is
redistributed from these scores — what a search takes out of one is a number — it is
recorded rather than applied.

### What to aim at

The dataset, and the held-out and test thirds of it in particular. Those are music the
search was not allowed to learn from, so they are the closest thing there is to a
measure of how the engine will do on a score nobody has seen — which is the point of the
exercise.

Do not tune against a handful of your own files. A dozen pieces get memorised: the
learning figure climbs while the test figure falls. And scores grabbed to test with are
often arrangements no human would play, so "worse" on them is not evidence of anything.

Two options exist for when you do want to hold a line, and both cost dataset accuracy,
so leave them alone unless you mean it:

* `--guard <pieces>` refuses any setting that does worse than the defaults on any one of
  those pieces, within a quarter of a point.
* `--min-shared 0.1` learns only from music where at least a tenth of the notes fall
  inside the other hand's usual range — the hard third of PDMX, where the hands share
  the keyboard.

## Running it

```bash
cargo build --release -p on-cli
target/release/opennote tune --target hands --scores path/to/PDMX/data
target/release/opennote tune --target play   --scores path/to/PDMX/data --notes 120 --batch 200
target/release/opennote tune --target taught --scores path/to/PDMX/data --notes 120 --batch 300
```

The playing target is given smaller numbers because a score costs about three hundred
times as much to finger as it does to divide between the hands. The defaults suit the
hand target; leave them on the playing one and a generation takes minutes rather than
seconds.

To keep every part of it improving and not just one, alternate the targets round and
round. That is all `tools/tune_forever.sh` (and `tools\tune_forever.bat`) does:

```bash
./tools/tune_forever.sh ~/PDMX/PDMX/data
```

### When a target is finished

A search is finished when it stops finding anything, and it needs to be told so, for two
reasons.

The first is that **it will always find something if the bar is low enough.** Trying
eight hundred thousand settings against one held-out third and keeping whatever edges it
is not searching, it is fitting the held-out third one candidate at a time. The hand
target did exactly this: 2.9 hours, 814,250 settings, and a promoted gain of 0.004 points
on the third it could see while the third it could not went *down* 0.013. The log said
"better" each time. So a promotion now has to clear `MIN_GAIN`, five hundredths of a
point — far below any real gain this has ever found, which have been one to ten points,
and far above the noise.

The second is that a setting no better than the best still wins a straight comparison on
a fresh draw about half the time, so the confirmation draw has to be won by that same
margin. Without it something is confirmed nearly every generation, the step size grows to
its ceiling and stays there, and the search never settles down to refine anything — the
hand target sat at step 0.95 with zero restarts for sixty thousand generations.

With both in place, a target that has run out says so and returns, and `tune_forever`
gives the machine to the next one:

```text
Nothing better in 5000 generations. This target has run out of things to try at
86.03% on the third it never learned from, against 86.03% for the engine's defaults.
```

`--give-up 5000` sets the patience, and 0 runs on regardless. It is counted in
generations rather than hours so that it scales with what a target costs: the hand
target runs about twenty thousand generations an hour and gives up after a quarter of
one; the taught target runs fifteen hundred and gets most of a working day.

### Making it fast

Measured on `--target taught`, which needs no dataset and so benchmarks the same way
every time, as settings tried per minute on twelve cores:

| | per minute |
|---|---|
| before any of this | 204 |
| without the solver's own threads, and with mimalloc | 289 |
| and built with `-C target-cpu=native` | 298 |

Those two were measured together rather than one at a time, so how the 85 divides
between them is not known — only that together they are worth it.

**The big one is not spawning threads inside threads.** Fingering one score is four
searches over the published rule sets, each of which is two more, one per hand — about
ten threads spawned and joined per score. For Handy, fingering one score, that is most
of a machine put to work. For the tuner, which is already running one setting per core,
it is a hundred and twenty threads contending for twelve and a pair of thread spawns to
pay for every score. `opennote tune` turns them off; nothing else does.

`-C target-cpu=native` is left off on purpose: three percent is not worth a binary that
crashes on any other machine and a full rebuild the next time somebody builds without
it. If you want it on a box that only ever tunes:

```bat
set RUSTFLAGS=-C target-cpu=native
cargo build --release -p on-cli
```

**There is no GPU in this and there is not going to be.** The work is a dynamic program
over finger assignments — branchy, data-dependent, pointer-chasing, with the answer at
each note depending on the one before. That is what a CPU is for. The one part that
looked like GPU work, the inverse-kinematics solve for each chord shape, is now answered
from a shared cache instead of being computed at all, which removed the arithmetic a GPU
would have been given. A kernel launch costs more than a cached shape lookup by three
orders of magnitude. It would also mean a second copy of the engine's core in a compute
shader, having to agree exactly with the one Handy ships, and no way to tell from the
output which of the two was wrong.

If this were a neural network the answer would be the opposite. It is not one, by
design — see `docs/BENCHMARK.draft.md` and the rule-based solver it describes.

### Stopping it

Stopping is safe at any moment — closing the window, Ctrl+C, a power cut. Everything a
search needs to carry on is written after every generation, so the most that is ever
lost is the generation in progress, and the hours already counted stay counted.

Killing `opennote` on its own is not enough, because `tune_forever` is a loop and will
start the next target. Either close the window, or:

```bat
tools	une_stop.bat
```

which closes the window first and then the search it left running, and says so if
anything survived. Afterwards `opennote tune --report` still reads everything it got to,
and starting it again carries on from there.

Useful options:

* `--hours 8` stops after eight hours. Without it, it runs until you stop it.
* `--limit 0`, the default, keeps every score there is. Fifteen thousand whole scores
  need about a gigabyte of memory; lower it if the machine has less. For `--target
  play`, `--notes` caps what a score costs to keep, so fifty thousand of them fit in
  much the same space.
* `--batch 600` is how many of them a generation is judged on, drawn afresh every
  generation. **This is what a generation costs, and `--limit` is not.** Every setting
  in one generation is judged on the same draw, so the comparison is fair; the draw
  changes next generation, so a setting that suits one handful of pieces does not stay
  ahead. Nothing is ever promoted on a draw — that is decided on whole thirds.
* `--notes 200`, for `--target play`, cuts each score to that many notes from the
  middle. Fingering a score costs about three hundred times what deciding its hands
  does, so this is the difference between a generation taking a minute and taking five.
  0 keeps whole pieces.
* `--threads 10` sets how many settings it tries at once. It uses one per core by
  default; use fewer if you want the machine for something else.
* `--measure models/weights.json` doesn't search. It scores that file against the
  defaults on whatever `--scores` you give it, which is how to check a setting on music
  it was never tuned on, one piece at a time if you like.

To leave it running on Windows without it getting in the way, start it at low priority:

```bat
start "opennote tune" /low /min tools\tune_forever.bat D:\PDMX\PDMX\data
```

To start it every time you log on, create a Task Scheduler task with the trigger "At log
on" and that same command as the action.

### On a Raspberry Pi

It builds and runs there — pure Rust, no GPU — but a Pi is about ten times slower per
core and has four of them rather than twelve. Pass `--batch 150 --notes 80` to keep a
generation to about a minute, and `--limit 6000` if the board has less than 4 GB.

Do not expect a week on a Pi to beat a night on a desktop, and do not expect week two to
beat week one. See [what more hours buy](#what-more-hours-buy).

## How long it has run

```bash
target/release/opennote tune --report
```

```text
hands: 41.75 hours, 1802184 settings tried.
  agreement with the people who wrote the music down, on the third it never learned
  from: 91.43%, against 86.95% for the engine's defaults at the time.
play: 12.10 hours, 401220 settings tried.
  music a hand can play comfortably, on the third it never learned from: 95.82%,
  against 94.50% for the engine's defaults at the time.
Altogether: 53.85 hours, 2203404 settings tried.
```

The hours are time actually spent searching, added up over every run there has ever
been. Time with the search stopped is not in them, so the number is one you can stand
behind. They live in `out/tune/<target>/hours.json`, apart from the search's own state,
because that state has to be thrown away whenever the engine changes under it and the
hours do not — they were spent either way. Nothing but deleting `out/tune/` resets them.

Each promotion also writes the same figures into `models/weights.json` beside the
weights, under names beginning with an underscore, so a setting carries its own
provenance:

```json
  "_hands.searched_hours": 41.75,
  "_hands.settings_tried": 1802184.0,
  "_hands.examples": 14885.0
```

Those are ignored when the weights are applied.

## What more hours buy

Less than you would hope, and it is worth knowing which lever actually moves.

The first long search on the hand target reached 89.13% in its first twelve seconds and
89.19% in ninety minutes: the remaining eighty-nine minutes bought six hundredths of a
point, and the last fifty-five of them bought nothing at all across fifty-four restarts.
Six numbers against a corpus is a small space and it gets exhausted quickly.

What does raise the ceiling, in order:

1. **More data.** This is the real limit, and it is why `--limit` now defaults to every
   score there is rather than a sample of five thousand.
2. **A measure with room in it.** Check the target can be got wrong before spending a
   week on it. Two of the four here were rescaled after their first run reported a
   number in the high nineties, which is not a healthy engine but a measure that cannot
   see. `--target taught` reads 71% and moved ten points in nine minutes; that is what
   room looks like.
3. **More numbers to tune.** The hand search has six; the fingering side has
   twenty-five, and had nothing to grade them with until `--target play`. Several engine
   constants are still hard-coded and could be opened up — and one set is deliberately
   left out. `BiomechWeights::strain`, the seven numbers saying what makes a hand
   posture uncomfortable, is carried by the engine but never read: the inverse-kinematics
   solve fixes them at their defaults. Plumbing them through would let the solver prefer
   a comfortable posture rather than only judging the one it finds, but postures would
   then stop being a function of the hand alone and the shared cache would have to be
   keyed on them, which for a search that changes them every generation means solving
   every chord shape again every generation.
3. **Changing the engine.** A search can only turn dials that exist. It cannot invent a
   better way to decide where the hands divide.

Which makes the case for leaving it running indefinitely a narrow one: it is worth doing
when the data keeps growing — corrections from people using Handy would do that — or
after the engine changes, and not otherwise.

## Reading what it did

Everything goes under `out/tune/<target>/`:

* **`log.csv`** has one row per generation. `best_held_out` is the column to watch: it
  only rises when a setting also did better on scores the search was not allowed to
  learn from. `best_test` is scores it has never been graded on at all, and is the
  honest figure. Both are measured over whole thirds. `current_train` and `best_train`
  are measured on that generation's draw, so they jump about — that is the sampling, not
  the search, and it is why neither is worth reading as a result.
* **`state.json`** is where it is. Delete it to start from the engine's defaults again.
  It also holds what the defaults and the best setting scored on whole thirds, which is
  seven passes over the dataset to work out — the best part of an hour for the playing
  target — so a search that resumes on the same number of examples trusts what is in
  there rather than measuring again. **Change the engine and those figures are stale:
  delete `state.json` after a change to the code the search is grading.**

Each improvement is written to `models/weights.json`, and every target shares that file.
To try it:

```bash
target/release/opennote annotate score.musicxml --tuned models/weights.json -o out.musicxml
```

## Making it the default

A promoted setting is a candidate, not a result. Before its numbers become the engine's
defaults, it has to:

1. beat the defaults on `best_test` by more than noise — that is the whole case for it,
   and it is about the dataset, not about anybody's favourite piece;
2. leave `cargo test --workspace` passing, including the method-book fingerings in
   `golden_fingerings.rs`;
3. leave `anatomy` reporting zero unreachable fingers, out-of-range joints and silent
   strikes, and `jitter` reporting no steps (see
   [PORTING.md](PORTING.md#checking-a-port)). These are physical honesty, not taste: a
   setting that agrees with a corpus by asking for fingerings a hand cannot make has not
   improved anything.

`handtruth` on your own scores is worth a look afterwards, but it is a report, not a
gate. They are a handful of arrangements, and the dataset is fifteen thousand.

Then the values are copied into the `Default` implementations: `HandAssignment` in
`on-score`, and `FingeringOptions`, `RuleWeights`, `BiomechWeights` and `StrainWeights`
in `on-fingering` and `on-hand`. Handy picks them up through the normal engine drop, with
no file to ship.

A setting the search likes and a test refuses is worth a second look before it is thrown
away: if the behaviour the test protects is one the dataset cannot see — a hand letting
go of notes it is still holding, say — the right answer is usually to make that case a
gate in `tune.rs`, so the search stops proposing it, rather than to keep rejecting it by
hand. `holds_what_it_is_holding` is there because of exactly that.
