# SoundFonts

Without a SoundFont, OpenNote plays a modelled piano that uses no samples. To hear a
sampled piano instead, put a SoundFont (`.sf2`) in this folder: the viewer and
`opennote render` pick up any `.sf2` here, and `--soundfont <file>` chooses one
explicitly. The Sound menu in the viewer switches between them. `*.sf2` is gitignored,
so whatever you download stays in your working copy.

## The YDP Grand Piano

The Yamaha Disklavier Pro grand from the [FreePats](https://freepats.zenvoid.org/)
project, built from Zenph Studios' samples for OLPC, works well. It is licensed
**CC-BY 3.0**: anything you publish with it, such as a rendered video, must credit it.
The full licence and credits are in [`LICENCE-YDP-GrandPiano.txt`](LICENCE-YDP-GrandPiano.txt).

The installers include it. In a build from source, run `opennote soundfont install`. The file
is 118 MB, which is more than GitHub allows in a repository, so it is never committed; the
release workflow downloads it from FreePats, checks its checksum, and packs it into each installer.

## Others

Any General MIDI SoundFont works; OpenNote plays its first preset. Check its licence
before publishing what you render with it. [`LICENSES.md`](../../LICENSES.md) covers what
that can mean.
