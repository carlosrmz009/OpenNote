# The hands

OpenNote does not include a hand model and does not build one. The 3D hands are a
**premade, openly licensed, rigged model that you supply**, dropped in here as
`hands.glb`. This file explains where to get one and how to prepare it.

The app runs without it — you get the keyboard and the falling notes, and a message
saying the hands are missing.

## What the model has to have

* a skeleton with a bone per finger segment: three per finger, three for the thumb
* both hands, or one hand that can be mirrored
* glTF binary (`.glb`) or glTF (`.gltf`)

It does **not** have to use any particular bone naming, rest pose, axis convention or
scale. `crates/on-viz/src/rig.rs` identifies bones by name across the four common
conventions and drives them by direction, so a rig from any of the sources below
works without editing code. If your rig uses names none of those cover, add them to
`rig::classify` — the tests there show the pattern.

| Convention | Example bone name |
|---|---|
| Unity / VRM humanoid | `LeftIndexProximal` |
| Mixamo | `mixamorig:LeftHandIndex1` |
| Blender Rigify | `f_index.01.L` |
| MakeHuman | `finger2-1.L` |

## Recommended: MakeHuman via MPFB2 (CC0)

MakeHuman's meshes, rigs and skins are released under **CC0** — no attribution, no
restrictions, commercial use fine — and the model is parametric, so you can match the
hand to a real pianist's proportions rather than accepting a generic one.

1. Install [Blender](https://www.blender.org/) 4.2 or newer.
2. Install the [MPFB2](https://static.makehumancommunity.org/mpfb.html) add-on
   (MakeHuman Plugin For Blender). It is on the Blender extensions platform.
3. Run the export script:

   ```bash
   blender --background --python tools/export_hand.py -- --out assets/hands/hands.glb
   ```

   The script is documented at the top of `tools/export_hand.py` and does the work in
   the order MPFB2 expects: create the human, add the skeleton, isolate the hands and
   forearms, and export.

4. Check what you got:

   ```bash
   cargo run -p on-cli -- play samples/fur-elise.musicxml
   ```

If MPFB2's interface has moved since the script was written, do the same steps by
hand in Blender and export the result to `assets/hands/hands.glb`; nothing about the
app depends on how the file was produced.

## Alternatives

| Source | Licence | Notes |
|---|---|---|
| [MakeHuman / MPFB2](https://static.makehumancommunity.org/) | CC0 assets, GPLv3 tool | Recommended. Parametric, realistic, no attribution needed. |
| [Ultraleap Unity Plugin](https://github.com/ultraleap/UnityPlugin) | Apache-2.0, assets included | `Packages/Tracking/Hands/Runtime/Models`. Guaranteed-correct four-bone-per-finger rig built for real-time articulation, but stylised rather than photoreal. Files are Git LFS; needs FBX to glTF conversion. |
| CC0 VRM avatars | CC0 | VRM *is* glTF, and VRM 1.0 mandates all fifteen finger bones per hand, so these load directly. |
| [LibHand](https://www.libhand.org/) | CC-BY 3.0 | 70k-triangle realistic textured hand. Needs attribution in the app, and the file is Blender 2.6-era. |

Avoid MANO, SMPL-X and NIMBLE: they are excellent models but licensed for
non-commercial academic research only.

## Recording what you used

If you add a model, note it in `PROVENANCE.md` next to this file — what it is, where
it came from, and under what licence. Anyone redistributing the app needs that, and
CC-BY models need it visible in the app itself.
