# Where the hand model came from

Two models are committed here, and this is what they are, so that anyone
redistributing the app knows what they are redistributing.

| Field | |
|---|---|
| File | `hand-left.glb`, `hand-right.glb` |
| Source | Generated locally by `tools/export_hand.py`, which drives [MPFB2](https://static.makehumancommunity.org/mpfb.html) inside Blender against MakeHuman's own base mesh, rig and skin |
| Author | The MakeHuman project |
| Licence | **CC0** — MakeHuman releases its meshes, rigs and skins into the public domain, so no attribution is required and commercial use is fine |
| Retrieved | Generated September 2026, from MPFB2 as installed from the Blender extensions platform |
| Changes made | Body reduced to one arm and hand; MakeHuman's helper geometry (the second skin it carries for fitting garments) removed by applying the mask modifier rather than exporting it hidden; skin and relief baked to a single 4096px texture; exported as glTF binary |

No third-party hand dataset is involved. `LICENSES.md` at the repository root
notes that MANO, SMPL-X and NIMBLE carry research-only terms and must not be used
outside non-commercial academic research — none of them is used here, and
`tools/export_hand.py` is the whole of how these two files were made, so the
result carries MakeHuman's terms and nothing else.

## If you replace them

Record the same fields again. A CC-BY model — most of the alternatives in
`README.md` are CC-BY — puts the attribution obligation on whoever publishes
something made with it, which is you rather than this repository.
