# Making a release

A release is made by changing the version.

1. Set `version` in the workspace `Cargo.toml` (`[workspace.package]`), commit, and push to `main`.
2. The **Release** workflow notices a version it has not released yet. It builds the Windows
   installer, a disk image for each kind of Mac and a Linux AppImage, then creates the tag
   `vX.Y.Z` and the GitHub Release with the installers and `SHA256SUMS.txt` attached, and marks
   it **Latest**. This takes about 45 minutes.
2a. To say what is new, write `docs/releases/X.Y.Z.md` first: it becomes the top of the release page, and
   GitHub adds its changelog below.
3. Pushing other changes to `Cargo.toml` (dependencies, say) does nothing: that version is
   already released.

The installers have no version in their names, so `releases/latest/download/OpenNote-windows-setup.exe`
(and the other three, see the README) always points at the newest. Older releases stay under
**Releases**.

You can also create a release by hand with a tag like `v1.2.3`; the tag must match `Cargo.toml`, or
the workflow stops, because the app compares its own version with the release's. To try the
packaging without publishing anything, run the **Release** workflow from the Actions tab; the
installers are attached to that run instead.

## The trained model

The trained model is private, so it is not in the repository. Without it the installers still
work and finger with the rules and the hand model alone. To build it in, add repository secrets:

- `OPENNOTE_MODEL`: the model's JSON, if it is under the 48 KB a secret can hold; or
- `OPENNOTE_MODEL_URL`, and `OPENNOTE_MODEL_TOKEN` if the place needs one: where the workflow
  downloads it from.
- `OPENNOTE_RELEASE_KEY`: the key that lets the build open published `.onm` models.

## Not signed

The installers are not signed by a registered developer, so Windows SmartScreen and macOS
Gatekeeper ask for confirmation the first time (the README says how to answer). Removing that
needs a code-signing certificate for Windows and an Apple Developer ID for macOS, added to the
workflow as secrets.

## Updates inside the app

The viewer looks at this repository's releases when it opens, and offers an update when:

- a release tagged `vX.Y.Z` is newer than the running version and has the installer for the
  user's system (its name ends in `-windows-setup.exe`, `-macos-arm64.dmg`, `-macos-intel.dmg` or
  `-linux-x86_64.AppImage`) plus `SHA256SUMS.txt`. The app refuses a download whose checksum does
  not match; or
- the `models` release holds a model of the same family and a higher number than the one in
  use (`helios-beta1.9.onm` over `helios-beta1.8`). Publish those with `opennote models seal`.

Builds without the release key cannot open published models, so they are never offered one.

## The app icon

`packaging/icon.png` and `icon.ico` come from `packaging/glover-new.png` by
`python3 packaging/make-icons.py`, which crops the tile and makes the backdrop transparent.
