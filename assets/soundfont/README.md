# The piano

OpenNote synthesises a piano from a model of a struck string, so it makes sound the
moment it is built with nothing to download. That model is honest but it is not a
recording of a particular instrument, and it does not sound like a concert grand,
because a real one has a soundboard, sympathetic strings, key noise and a spectrum no
handful of partials reproduces.

Drop a **SoundFont** in this directory and it plays that instead. Any `.sf2` here is
picked up automatically; `--soundfont <path>` overrides.

```bash
opennote play score.mid                      # uses whatever .sf2 is here
opennote play score.mid --soundfont other.sf2
```

It is used for the live window and for `opennote render` alike, so the exported
video has the same piano on it as the one you heard.

## Where to get one

All three of these come from [FreePats](https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html),
which is the reputable place for openly licensed instrument samples. Nothing here is
bundled and nothing is downloaded for you — pick one and fetch it yourself.

Sizes below are the download and then the `.sf2` it unpacks to — the second is the
one that matters, because the whole file is read into memory when a piece is played.

| | Download | Unpacked | Licence | Notes |
|---|---|---|---|---|
| **Salamander Grand Piano** | 296 MiB | **1.27 GiB** | CC-BY 3.0 | A Yamaha C5 in sixteen velocity layers, by Alexander Holm. The best of the three by a distance, and it wants the RAM to prove it. |
| **YDP Grand Piano** | 36 MiB | 113 MiB | CC-BY 3.0 | A Yamaha Disklavier Pro. A real grand at a size you will not notice. |
| **Upright Piano KW** | 27 MiB | ~30 MiB | **CC0** | An upright, not a grand — but public domain, so it asks nothing of you at all. |

```bash
# Salamander Grand Piano — the best of them
curl -LO 'https://freepats.zenvoid.org/Piano/SalamanderGrandPiano/SalamanderGrandPiano-SF2-V3+20200602.tar.xz'
tar xJf 'SalamanderGrandPiano-SF2-V3+20200602.tar.xz' --strip-components=1 -C assets/soundfont

# YDP Grand Piano — a tenth of the size, still a real grand
curl -LO https://freepats.zenvoid.org/Piano/YDP-GrandPiano/YDP-GrandPiano-SF2-20160804.tar.bz2
tar xjf YDP-GrandPiano-SF2-20160804.tar.bz2 --strip-components=1 -C assets/soundfont

# Upright Piano KW — public domain
curl -LO https://freepats.zenvoid.org/Piano/UprightPianoKW/UprightPianoKW-SF2-20220221.7z
7z x UprightPianoKW-SF2-20220221.7z -oassets/soundfont
```

Move the `.sf2` itself to `assets/soundfont/` if the archive nests it in a folder.
Only one is used — the first `.sf2` in alphabetical order — so keep the one you want.

Any other SoundFont works too — `FluidR3_GM` and `GeneralUser GS` are the usual
general-MIDI ones, and their pianos are fine without being remarkable. The first
preset in the file is the one played, so a piano-only SoundFont needs no configuring.

## Attribution

Both grands are **CC-BY 3.0**, which asks you to credit the author and say if you
changed anything. That obligation is yours, not this repository's, and it attaches to
anything you publish with that piano on it — a rendered video, most obviously.

- **YDP Grand Piano** — Roberto (roberto@zenvoid.org), from Zenph Studios / OLPC samples
- **Salamander Grand Piano** — Alexander Holm
- **Upright Piano KW** — CC0, so nothing is required

`assets/soundfont/*.sf2` is gitignored. Downloaded samples are yours and stay in your
working copy; redistributing them is a licence question this repository does not answer
for you.
