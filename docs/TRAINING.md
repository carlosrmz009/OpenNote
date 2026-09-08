# Training the fingering model

OpenNote works out fingerings from keyboard geometry, a model of the hand, and the
published ergonomic rules. That gets you a long way and needs no data at all.

What it cannot know is what pianists *actually do*. Fingering carries habits and
teaching traditions that comfort alone does not explain, and the only way to pick those
up is from examples. This is how you give it some.

You do not need to know anything about machine learning to follow this. There is
nothing to configure, nothing that can fail to converge, and every step tells you
whether it helped.

---

## The shape of it

```
opennote corpus add <path>     collect expert fingerings
opennote train                 learn from them, and say whether it helped
opennote play <score> --model models/prior.json
```

Three commands. The middle one always prints a before and an after, measured on pieces
the model was not allowed to see, so you can tell at a glance whether the data you just
added made things better or worse.

---

## 1. Find some fingered music

The model learns from music that somebody has already fingered. Three sources work:

**A dataset.** The standard one is **PIG** (the Piano Fingering Dataset, from Nakamura,
Saito & Yoshii): 150 pieces, 309 fingerings by professional pianists. It is free, but
you have to register for it, and its licence is for nonprofit academic use only — so it
is not included here and never will be. Download your own copy from the authors and
point `corpus add` at the folder.

**Your own editions.** Any MusicXML file that already carries fingering marks will do —
an urtext edition you have marked up, or anything exported from a notation program with
fingerings on the notes. These are often better than a dataset, because they are the
music *you* play, fingered the way you want it.

**Video of somebody playing.** There is a great deal of piano on the internet filmed
from directly overhead, and every one of those videos is a pianist showing you their
fingering. See [Learning from video](#learning-from-video) below — it is more work than
the other two, and it is also the only source that scales.

## 2. Add it to the corpus

```bash
opennote corpus add ~/Downloads/PianoFingeringDataset --name pig
```

It walks the folder, reads what it recognises, and says what it found:

```
Added 309 fingerings of 150 pieces, 195420 notes altogether.
Written to corpus/normalised/pig.jsonl.
```

Files it does not recognise are skipped rather than treated as errors, so pointing it at
a whole download folder is fine. Anything that *looked* like data but could not be read
is listed, so a malformed file is not silently lost.

Add as much as you like, whenever you like:

```bash
opennote corpus add ~/scores/chopin-nocturne.musicxml --name my-editions
opennote corpus list
```

The corpus is plain JSON, one line per fingering, in `corpus/normalised/`. You can open
it, diff it, and delete a file to remove what it contained. `corpus/` is not committed
to git — your data stays yours, and PIG stays where its licence wants it.

## 3. Train

```bash
opennote train
```

```
Training on 309 fingerings…
Learned from 120 pieces, held 30 back.
  rules only:  general 57.1%   highest 61.8%   soft 64.9%   (39218 notes, 30 pieces)
  with model:  general 63.4%   highest 68.0%   soft 71.2%   (39218 notes, 30 pieces)
The model helped: agreement went up 6.3 points. Keep it.

Model written to models/prior.json.
```

Two things to understand about those numbers.

**They are measured on music the model never saw.** A fifth of the pieces are held back
before training and only used for this comparison. A model always looks good on what it
was trained on; that number would tell you nothing.

**100% is not the target.** Two pianists fingering the same piece agree about **71%** of
the time. Fingering is a matter of judgement, and above about 70% you are measuring
whose judgement, not whether the answer is good.

For reference, the published results on PIG: a second-order statistical model reaches
64.3%, a constraint-based search 56.7%, and human against human 71.4%.

### The three rates

- **general** — agreement with each pianist's fingering separately, averaged. The
  strictest, and the one to watch.
- **highest** — agreement with whichever pianist it matched best, per piece. Answers
  "did it find *an* answer a pianist would recognise?"
- **soft** — a note counts if *any* pianist used that finger. The most generous.

General going up while soft stays flat means the model is settling on one tradition.
Soft going up means it is finding fingerings nobody used — which is worth looking at.

### Seeing what it learned

```bash
opennote train --explain
```

```
Right hand: 96420 notes seen, 4180 contexts
  after finger 1, going up 1 semitone, white to black, uses 2 94% (1204 notes)
  after fingers 3-4, going up 2 semitones, white to white, uses 5 91% (860 notes)
  ...
```

The model is a table of counts, so it can say what it believes in plain language. If a
line looks wrong, the data that produced it is wrong.

### Fitting the weights too

```bash
opennote train --tune-weights
```

Also fits three numbers: how far the trained model, the published rules and the standard
chord shapes are trusted against each other. It tries values and keeps whatever agreed
best with the training pieces. Slower, and worth doing once you have more than a handful
of pieces. It will never turn the rules off entirely — those are the only thing stopping
a fingering no hand could play.

## 4. Use it

```bash
opennote play score.musicxml --model models/prior.json
opennote annotate score.musicxml --model models/prior.json --out fingered.musicxml
```

`--prior-scale` says how far to trust it, from 0 (ignore) to about 3 (trust heavily).
The default of 1.0 blends it with the rules; the rules still veto anything unplayable.

## 5. Check again later

```bash
opennote eval
```

Prints the same before-and-after on the held-out pieces, without retraining. Run it
after adding data to see whether the model you have is still the one you want.

---

## Learning from video

An overhead piano video plus a MIDI file of the same performance is a fingering. The
video says where the fingers were; the MIDI says which keys went down and when. Joining
the two gives you an annotation nobody had to write out by hand — and unlike a dataset,
there is no shortage of them.

This is PianoMime's method (Qian, Urain, Zakka & Peters, 2024), which they built to
teach a robot hand to play. The part in the middle — working out which finger was on
which key — is exactly what a fingering model wants.

### What you need

- A video shot from **directly overhead**, showing the whole keyboard, with the hands
  clearly visible. The Synthesia-style tutorial channels are ideal; a video shot from
  the side is not.
- A **MIDI file of that same performance**. Many channels publish one. A MIDI of a
  *different* performance of the same piece will not line up.
- Python, with `opencv-python` and `mediapipe`, and MediaPipe's hand landmarker:

```bash
pip install opencv-python mediapipe
curl -O https://storage.googleapis.com/mediapipe-models/hand_landmarker/hand_landmarker/float16/1/hand_landmarker.task
```

### Say where the keyboard is

Once per camera position — which for a given channel means once, ever.

```bash
python tools/watch_pianist.py calibrate video.mp4 --out corners.json
opennote corpus calibrate corners.json --out camera.json
```

The first command shows you a frame and asks you to click four corners of the keyboard;
the second turns those into the map from pixels to millimetres of piano. Only the two
end keys have to be identified — where every other key is follows from the instrument's
geometry, which the program already knows to the millimetre.

### Watch the hands

```bash
python tools/watch_pianist.py watch video.mp4 --out video.landmarks.json
```

This is the slow step: it runs the hand tracker over every sampled frame and writes down
where the twenty landmarks were. Nothing is interpreted here — that happens next, in
Rust, where it is tested.

### Join it to the notes

```bash
opennote corpus watch video.landmarks.json     --score video.mid --calibration camera.json --annotator rousseau
```

```
Read 2841 of 3120 notes (91%): 1204 left hand, 1637 right.
Written to corpus/normalised/video.jsonl.
```

Then `opennote train` as before.

Give each pianist their own `--annotator`. Two videos of the same piece by two people
are two fingerings of it, which is what the highest and soft match rates are for.

### If the numbers look wrong

**Very few notes read.** Usually the video and the MIDI do not start together. Find a
moment you can identify in both and pass the difference as `--offset` — positive if the
video starts later than the score.

**Nonsense fingerings.** Usually the calibration. Re-click the corners, and check you
named the right end keys: an octave out puts every finger seven keys off.

**Hands found in only a few frames.** The tracker needs the hands reasonably large in
frame. A 4K video downscaled to fit a phone screen often loses them.

### What it does with the data

For every chord in the score it finds the video frame closest in time, projects each
fingertip onto the keyboard, and matches keys to fingers nearest-first. Two rules keep it
honest: a finger plays at most one key at a time, and a fingertip that is not over the
keyboard at all is playing nothing. Fingers of one hand are then put back into pitch
order, because a pianist's are, and tracking noise sometimes says otherwise.

It is not perfect and does not need to be. It is a statistical model: what matters is
that the errors are unbiased and the volume is large, and video gives you volume no hand
annotation ever will.

---

## When it goes wrong

**"the corpus is empty"** — nothing has been added yet, or `corpus add` found no
fingerings. MusicXML files only count if their notes actually carry `<fingering>` marks;
a plain score has nothing to learn from.

**The model made things worse.** Almost always too little data. A handful of pieces
teaches the model a handful of habits and nothing about the exceptions. Add more before
relying on it — the model is still written, so nothing is lost.

**The numbers do not move.** Also normal below about twenty pieces. The rules already do
well on ordinary passages; the model earns its keep on the awkward ones, and it needs to
have seen some.

**Nothing changed after adding data.** Check `opennote corpus list`. If the piece count
did not go up, the files were not recognised.

---

## What the model actually is

A table of counts: how often each finger followed each other finger, over each interval,
between each pair of key colours. Nothing more.

That is deliberate. It trains in a second, the file can be read by eye, ten more pieces
change it by about what ten more pieces should, and there is no way for training to
silently go wrong. A neural network would fit a corpus this size beautifully and tell you
nothing you could check.

Contexts nobody has played are not forbidden, only made unlikely: estimates back off from
specific contexts to general ones, weighted by how much was actually seen (Witten-Bell
smoothing). The rules, not the corpus, are what rule a fingering out.

See `crates/on-fingering/src/prior.rs` if you want the details.
