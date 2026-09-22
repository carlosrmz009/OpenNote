# Training data

Empty on purpose, and gitignored apart from this file.

`opennote corpus add <path>` reads expert fingerings into `corpus/normalised/` as
JSON, one fingering per line, and `opennote train` learns from whatever is there.
See `docs/TRAINING.md` for the walkthrough.

Two sources are understood: PIG `_fingering.txt` files, and MusicXML that already
carries `<fingering>` marks on its notes.

The standard dataset is PIG (Nakamura, Saito & Yoshii), 150 pieces with 309 expert
fingerings. It is free but registration-gated and licensed for nonprofit academic use
only, so it is never committed here and nothing in this repository downloads it.
Fetch your own copy and point `corpus add` at it.
