# Making a release

1. Set `version` in the workspace `Cargo.toml`, commit, and push to `main` once CI is green.
2. Tag it and push the tag: `git tag v0.2.0 && git push origin v0.2.0`.
3. The **Release** workflow builds the Windows installer, a disk image for each kind of Mac, and
   a Linux AppImage, and attaches them, with their checksums, to a new GitHub Release.

To try the packaging without publishing anything, run the **Release** workflow by hand from the
Actions tab; the installers are attached to that run instead.

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
