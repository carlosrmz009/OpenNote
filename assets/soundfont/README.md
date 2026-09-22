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

Both of these come from [FreePats](https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html),
which is the reputable place for openly licensed instrument samples. Nothing here is
bundled and nothing is downloaded for you — pick one and fetch it yourself.

Sizes below are the download and then the `.sf2` it unpacks to — the second is the
one that matters, because the whole file is read into memory when a piece is played.

| | Download | Unpacked | Licence | Notes |
|---|---|---|---|---|
| **YDP Grand Piano** | 36 MiB | 113 MiB | CC-BY 3.0 | A Yamaha Disklavier Pro. A real grand at a size you will not notice. |
| **Upright Piano KW** | 27 MiB | ~30 MiB | **CC0** | An upright, not a grand — but public domain, so it asks nothing of you at all. |

```bash
# YDP Grand Piano — a real grand, and small enough not to notice
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

The YDP grand is **CC-BY 3.0**, which asks you to credit the author and say if you
changed anything. That obligation is yours, not this repository's, and it attaches to
anything you publish with that piano on it — a rendered video, most obviously.

- **YDP Grand Piano** — Roberto (roberto@zenvoid.org), from Zenph Studios / OLPC samples
- **Upright Piano KW** — CC0, so nothing is required

Whatever you supply yourself carries its own terms, and they are yours to honour.
The concert grand this project's own renders use is not from FreePats and is not
covered by anything above: it is under the **Free Art License 1.3**, which is a
copyleft licence.

That matters more than CC-BY does, and it is worth reading before you build anything
on it. The Free Art License permits commercial use outright — distribution "with or
without any charge" — and asks in return that you name the original authors, say where
the original can be found, attach the licence or point to it, and, for a *subsequent
work*, say that you modified it and release it under the same or a compatible licence.

The question that needs a real answer, and this file is not the place it gets one, is
whether audio rendered from a sampled instrument counts as a subsequent work. A
rendering reproduces the samples rather than merely being made with a tool, which is
not the same situation as a program's output, and the copyleft term is what makes the
answer matter. If you are shipping rendered audio in something proprietary, ask a
lawyer rather than this README.

There is a way round it that costs nothing: the piano in `on-audio` is synthesised
from a model of a struck string and uses no samples at all, so it carries no sample
licence of any kind. It is the default, and it is what plays when no `.sf2` is
present.

`assets/soundfont/*.sf2` is gitignored. Downloaded samples are yours and stay in your
working copy; redistributing them is a licence question this repository does not answer
for you.
