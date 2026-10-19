# SoundFonts

OpenNote plays **Studio Yamaha C7** by default: the viewer and `opennote render` pick up the
`.sf2` in this folder, and `--soundfont <file>` chooses another one. The Sound menu in the viewer
switches between them. Without any SoundFont, OpenNote plays a modelled piano that uses no samples.

Studio Yamaha C7 is licensed under the
[Free Art License 1.3](LICENCE-Free-Art-1.3.txt) (official text: <https://artlibre.org/licence/lal/en/>). The installers include it. A build from source gets it
with:

```sh
opennote soundfont install
```

That downloads it and checks it against a known checksum. It is not kept in the repository, and
`*.sf2` is gitignored, so whatever you download stays in your working copy. The release workflow
runs the same command before packaging each installer.

Any General MIDI SoundFont works; OpenNote plays its first preset. Check its licence before
publishing what you render with it. [`LICENSES.md`](../../LICENSES.md) covers what that can mean.
