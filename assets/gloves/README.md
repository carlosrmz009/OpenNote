# Glove fabric

The weave the gloves are made of.

| File | What it is |
|---|---|
| `weave-colour.png` | how much light the threads catch, as a single channel |
| `weave-normal.png` | the relief of the weave, as an OpenGL-convention tangent-space normal map |

Both are downscaled and converted from **Fabric018** on
[ambientCG](https://ambientcg.com/view?id=Fabric018), which publishes its materials
under [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/) — public domain, no
attribution required. The credit here is courtesy, not obligation.

It is a knit, and it had to be. A hand covers perhaps two hundred pixels of the finished
picture; a fine linen has a hundred threads across a tile, so a thread lands well under
a pixel and averages away to a flat colour however carefully it is sampled. The
arithmetic only closes with a cloth whose structure is a few dozen stitches across
rather than a few hundred. The first attempt used a linen and produced a glove with no
visible texture at all.

The colour map is kept as a single channel on purpose. The gloves are a flat cartoon
yellow and the cloth only says how much light each stitch catches; keeping its own colour
would drag the yellow towards the green of the wool it was photographed from.

Neither file is a texture the renderer uses directly. Both are sampled while the glove
is painted onto the hand — see `crates/on-viz/src/glove.rs` — because the hand's own
texture layout maps the palm and the forearm at quite different densities, and a texture
tiled over the model at one scale suits neither.
