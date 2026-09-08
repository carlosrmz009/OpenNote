"""Export a rigged hand model from MakeHuman's CC0 assets, via MPFB2 in Blender.

OpenNote does not model a hand. This script drives MPFB2 — the MakeHuman plugin for
Blender — to generate one from MakeHuman's own CC0 base mesh and rig, and writes it
out as glTF for the visualizer to load.

    blender --background --python tools/export_hand.py -- --out assets/hands/hands.glb

Blender is usually not on PATH on Windows; `tools/export_hand.ps1` finds it for you.

Requires Blender 4.2+ with the MPFB2 extension installed. MPFB2 ships as a Blender
extension, so it imports as `bl_ext.blender_org.mpfb` rather than as `mpfb`; both are
tried below.
"""

from __future__ import annotations

import argparse
import os
import sys

try:
    import bpy
except ImportError:  # pragma: no cover - only meaningful inside Blender
    print("This script has to be run inside Blender:", file=sys.stderr)
    print("  blender --background --python tools/export_hand.py -- --out <path>", file=sys.stderr)
    raise SystemExit(2)


def import_mpfb():
    """Import MPFB2's services, whether it is installed as an extension or an add-on."""
    errors = []
    for prefix in ("bl_ext.blender_org.mpfb", "bl_ext.user_default.mpfb", "mpfb"):
        try:
            human = __import__(f"{prefix}.services.humanservice", fromlist=["HumanService"])
            objects = __import__(f"{prefix}.services.objectservice", fromlist=["ObjectService"])
            return human.HumanService, objects.ObjectService
        except ImportError as error:
            errors.append(f"  {prefix}: {error}")
    print("Could not import MPFB2. Tried:", file=sys.stderr)
    print("\n".join(errors), file=sys.stderr)
    print("Install it from Blender's extensions platform and try again.", file=sys.stderr)
    raise SystemExit(3)


def parse_args() -> argparse.Namespace:
    """Read the arguments after Blender's own `--` separator."""
    argv = sys.argv
    args = argv[argv.index("--") + 1:] if "--" in argv else []
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out-dir",
        default="assets/hands",
        help="directory to write hand-left.glb and hand-right.glb into",
    )
    parser.add_argument(
        "--rig",
        default="default_no_toes",
        help="which MakeHuman rig to use; any of them names finger bones finger1-1.L and so on",
    )
    parser.add_argument(
        "--subdivide",
        type=int,
        default=2,
        help="Catmull-Clark subdivision levels; 0 leaves MakeHuman's own topology",
    )
    parser.add_argument(
        "--hand-length",
        type=float,
        default=182.0,
        help="target hand length in millimetres, wrist crease to middle fingertip",
    )
    return parser.parse_args(args)


def clear_scene() -> None:
    """Start from an empty file, whatever Blender opened with."""
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for collection in (bpy.data.meshes, bpy.data.armatures, bpy.data.objects):
        for item in list(collection):
            collection.remove(item)


# The forearm is included. The model used to stop at the wrist, on the reasoning that
# an arm running off the edge of the screen adds nothing — but a hand that stops dead at
# the wrist has to end in *something*, and whatever that something was read as a cuff
# stuck on the end of it. An arm that simply continues out of frame is both simpler and
# what a viewer expects to see.
HAND_BONE_HINTS = ("finger", "metacarpal", "wrist", "hand", "lowerarm", "forearm")


def is_hand_bone(name: str) -> bool:
    """Whether a bone belongs to the part of the body we are keeping."""
    lowered = name.lower()
    return any(hint in lowered for hint in HAND_BONE_HINTS)


def keep_only_hands(basemesh, armature, side: str) -> None:
    """Delete everything except one hand, in the mesh and in the skeleton.

    One file per hand, rather than one file with both. The visualizer places each hand
    independently — they are rarely in the same place — and a scene holding both would
    have to be taken apart again at load time.
    """
    import bmesh

    suffix = "." + side

    def belongs(name: str) -> bool:
        return is_hand_bone(name) and name.endswith(suffix)

    keep_groups = {
        index for index, group in enumerate(basemesh.vertex_groups) if belongs(group.name)
    }
    if not keep_groups:
        raise SystemExit(f"found no vertex groups for the {side} hand")

    mesh = bmesh.new()
    mesh.from_mesh(basemesh.data)
    deform = mesh.verts.layers.deform.verify()
    doomed = [v for v in mesh.verts if not keep_groups & set(v[deform].keys())]
    bmesh.ops.delete(mesh, geom=doomed, context="VERTS")
    mesh.to_mesh(basemesh.data)
    mesh.free()
    basemesh.data.update()

    bpy.context.view_layer.objects.active = armature
    bpy.ops.object.mode_set(mode="EDIT")
    doomed = [bone for bone in armature.data.edit_bones if not belongs(bone.name)]
    for bone in sorted(doomed, key=lambda b: -len(b.parent_recursive)):
        armature.data.edit_bones.remove(bone)
    bpy.ops.object.mode_set(mode="OBJECT")
    kept = sorted(bone.name for bone in armature.data.bones)
    print(f"  {len(basemesh.data.vertices)} vertices, {len(kept)} bones: {', '.join(kept)}")


def mend_mesh_holes(obj, largest_fraction: float = 0.15) -> int:
    """Close the punctures in an object's mesh, leaving its real openings alone."""
    import bmesh

    mesh = bmesh.new()
    mesh.from_mesh(obj.data)
    mended = mend_holes(mesh, largest_fraction)
    if mended:
        mesh.to_mesh(obj.data)
        obj.data.update()
    mesh.free()
    return mended


def mend_holes(mesh, largest_fraction: float = 0.15) -> int:
    """Close the small holes trimming the body down to a hand leaves behind.

    The trim keeps a vertex only if some hand bone deforms it. A handful of vertices
    scattered over the hand are weighted entirely to bones that are not in the hand,
    and losing those leaves small holes clean through the surface. From above, with
    nothing behind the hand but a black background, each one renders as a neat black
    dot, and a hand freckled with them looks like it has been spattered with ink. They
    are easy to mistake for moles in the skin texture and impossible to fix as one —
    which cost an afternoon of increasingly baroque texture filtering before anyone
    counted the mesh's boundary edges.

    This runs after subdivision rather than straight after the trim, because that is
    where the punctures actually appear: the trim leaves a few vertices attached by
    nothing, and it is Catmull-Clark that turns those into open rings.

    Only small holes are closed. The cut at the wrist is a hole too, by exactly the
    same test, and it has to stay open — it is where the hand ends. So each boundary
    loop is measured against the size of the whole mesh, and anything that is a real
    opening rather than a puncture is left alone.
    """
    import bmesh

    span = max(basemesh_extent(mesh), 1e-6)
    holes, mended = [], 0
    for loop in boundary_loops(mesh):
        points = [v.co for edge in loop for v in edge.verts]
        reach = max(
            max(p[axis] for p in points) - min(p[axis] for p in points) for axis in range(3)
        )
        if reach < span * largest_fraction:
            holes.extend(loop)
            mended += 1
    if holes:
        bmesh.ops.holes_fill(mesh, edges=holes, sides=0)
    return mended


def basemesh_extent(mesh) -> float:
    """The longest side of a bmesh's bounding box."""
    if not mesh.verts:
        return 0.0
    lo = [min(v.co[axis] for v in mesh.verts) for axis in range(3)]
    hi = [max(v.co[axis] for v in mesh.verts) for axis in range(3)]
    return max(hi[axis] - lo[axis] for axis in range(3))


def boundary_loops(mesh) -> list:
    """Group a bmesh's boundary edges into connected loops."""
    boundary = [edge for edge in mesh.edges if edge.is_boundary]
    among = set(boundary)
    loops = []
    seen = set()
    for start in boundary:
        if start in seen:
            continue
        loop, pending = [], [start]
        while pending:
            edge = pending.pop()
            if edge in seen:
                continue
            seen.add(edge)
            loop.append(edge)
            for vertex in edge.verts:
                pending.extend(e for e in vertex.link_edges if e in among and e not in seen)
        loops.append(loop)
    return loops


def unify_spaces(basemesh, armature) -> None:
    """Rewrite the mesh's vertices into the armature's space.

    MPFB2 builds the base mesh and the rig it generates for that mesh at different
    object origins. Blender does not mind — it composes each object's own matrix when
    it draws — but glTF skinning does. A skinned mesh is bound to its skeleton by
    inverse bind matrices measured in the armature's space, and glTF has no place to
    record that the mesh itself lives somewhere else. Vertices then get swung around
    joints they sit the better part of a metre away from, and the hand exports as a
    smear rather than a hand.

    The fix is applied to the vertices rather than to the objects. Transforming an
    armature that already has a skinned child is a well-known way to desynchronise the
    two silently; moving the points once, into the space the bones are measured in,
    cannot.
    """
    delta = armature.matrix_world.inverted() @ basemesh.matrix_world
    offset = delta.to_translation()
    if offset.length > 1e-6:
        print(f"  mesh sits {offset.length:.4f} units from the rig; moving it into rig space")
    basemesh.data.transform(delta)
    basemesh.matrix_world = armature.matrix_world.copy()
    basemesh.data.update()


def bone_head(armature, name: str):
    """World-space head of a bone."""
    bone = armature.data.bones.get(name)
    return armature.matrix_world @ bone.head_local if bone else None


def bone_tail(armature, name: str):
    """World-space tail of a bone."""
    bone = armature.data.bones.get(name)
    return armature.matrix_world @ bone.tail_local if bone else None


def smooth_surface(basemesh, levels: int) -> None:
    """Subdivide the hand, and shade it smoothly.

    MakeHuman's base mesh is built for a whole figure at conversational distance. A
    hand from it has perhaps two thousand vertices, which is plenty for the shading
    but not for the silhouette: seen from close overhead, a finger is visibly a
    six-sided prism and a knuckle is a crease.

    Catmull-Clark subdivision is applied here rather than left to the renderer,
    because the vertex weights have to be interpolated with the surface or the
    subdivided mesh would not deform. Two levels turn a finger into something round,
    at a vertex count that is still nothing for a modern GPU — there are only two
    hands on screen.
    """
    if levels <= 0:
        return
    bpy.context.view_layer.objects.active = basemesh

    # MPFB2 drives the body's proportions with shape keys, and Blender will not apply
    # a modifier to a mesh that has any. The proportions are already the ones we want,
    # so the current mix is baked into the mesh and the keys removed.
    if basemesh.data.shape_keys is not None:
        bpy.ops.object.shape_key_remove(all=True, apply_mix=True)

    # MakeHuman's base mesh carries helper geometry — a second skin, loose around the
    # body, that garments are fitted to. `mask_helpers` hides it behind a mask modifier
    # rather than deleting it, so the vertices are still in the mesh and still get
    # exported. What arrives in the file is the hand inside a sleeve: a separate shell
    # over the forearm that stops at the wrist, and the edge where it stops is a hard
    # collar round the arm.
    #
    # Applying the mask removes them for real. It has to happen here rather than at
    # creation, because Blender will not apply a modifier to a mesh with shape keys and
    # MPFB2 drives the proportions with those; they are baked in just above.
    for modifier in list(basemesh.modifiers):
        if modifier.type == "MASK":
            bpy.ops.object.modifier_apply(modifier=modifier.name)

    modifier = basemesh.modifiers.new("Subdivide", "SUBSURF")
    modifier.levels = levels
    modifier.render_levels = levels
    # The armature modifier has to stay behind it: subdividing a posed mesh would
    # bake the pose into the surface.
    bpy.ops.object.modifier_move_to_index(modifier=modifier.name, index=0)
    bpy.ops.object.modifier_apply(modifier=modifier.name)

    for polygon in basemesh.data.polygons:
        polygon.use_smooth = True
    print(f"  subdivided to {len(basemesh.data.vertices)} vertices")
    mended = mend_mesh_holes(basemesh)
    if mended:
        print(f"  mended {mended} punctures")


def add_skin_material(basemesh) -> None:
    """Give the hand a plain material.

    Only a placeholder. The visualizer dresses the hands itself — in gloves it generates
    from a weave, which is both a kinder thing to magnify than skin and free of the
    problems that come with baking a whole body's texture layout onto one hand. Nothing
    in this file needs to produce a texture any more.

    Kept because a glTF with no material at all is awkward to open in anything else, and
    this file is worth being able to inspect.

    MPFB2 leaves the base mesh without one unless a skin asset is applied, and a
    glTF primitive with no material is not something every renderer will draw. A plain
    physically based material with a skin tone is enough: the visualizer lights it,
    and anyone who wants a photographic skin can apply a MakeHuman one before
    exporting.
    """
    material = bpy.data.materials.new(name="Skin")
    material.use_nodes = True
    principled = material.node_tree.nodes.get("Principled BSDF")
    if principled is not None:
        principled.inputs["Base Color"].default_value = (0.82, 0.63, 0.53, 1.0)
        principled.inputs["Roughness"].default_value = 0.62
        if "Specular IOR Level" in principled.inputs:
            principled.inputs["Specular IOR Level"].default_value = 0.35
    basemesh.data.materials.clear()
    basemesh.data.materials.append(material)


def export(path: str) -> None:
    """Write the scene out as a glTF binary with its skeleton."""
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.export_scene.gltf(
        filepath=path,
        export_format="GLB",
        export_skins=True,
        export_animations=False,
        export_apply=False,
        use_selection=True,
        export_yup=False,
        # The normal map is useless without them: a tangent-space normal is expressed
        # in a frame the mesh has to carry, and a renderer handed the map but not the
        # frame has no way to know which way "sideways" points on a given triangle.
        export_tangents=True,
    )
    print(f"wrote {path}")


def build_hand(human_service, args, side: str, out_path: str) -> None:
    """Generate, trim, orient, scale and export one hand."""
    label = {"L": "left", "R": "right"}[side]
    print(f"building the {label} hand")

    clear_scene()
    # feet_on_ground is deliberately off. It bakes a Z offset into the mesh's
    # vertices, but the rig that MPFB2 fits afterwards is built in MakeHuman's own
    # hip-centred space — so the mesh ends up standing about 0.83 units above its own
    # skeleton. Blender still draws it correctly, because the armature modifier is
    # evaluated per object, but glTF skinning has nowhere to record the discrepancy
    # and the hand exports as a smear. Leaving the human where MakeHuman put it keeps
    # mesh and bones in one space, which is all this export needs.
    basemesh = human_service.create_human(
        mask_helpers=True, detailed_helpers=False, feet_on_ground=False
    )
    armature = human_service.add_builtin_rig(basemesh, args.rig, import_weights=True)
    if armature is None:
        raise SystemExit(f"MPFB2 could not add the {args.rig} rig")

    keep_only_hands(basemesh, armature, side)
    unify_spaces(basemesh, armature)
    smooth_surface(basemesh, args.subdivide)
    add_skin_material(basemesh)
    bpy.context.view_layer.update()

    wrist = bone_head(armature, f"wrist.{side}")
    tip = bone_tail(armature, f"finger3-3.{side}")

    # Deliberately no scaling, rotating or moving here. Transforming an armature that
    # has a skinned mesh bound to it leaves the two in different spaces unless every
    # step is done in exactly the right order, and the failure is silent: the file
    # looks fine and the hand renders nowhere near its own skeleton.
    #
    # The renderer measures what it is given — which way the hand faces, where its
    # wrist is, how long it is — and fits it to the pianist. So the file is exported
    # exactly as MakeHuman built it, and stays self-consistent.
    if wrist is not None and tip is not None:
        direction = (tip - wrist).normalized()
        print(f"  hand is {(tip - wrist).length:.4f} units long, fingers pointing {direction}")

    # The check that catches the failure above before it reaches the renderer: the
    # mesh has to enclose its own skeleton.
    corners = [basemesh.matrix_world @ v.co for v in basemesh.data.vertices]
    low = [min(c[i] for c in corners) for i in range(3)]
    high = [max(c[i] for c in corners) for i in range(3)]
    print(f"  mesh spans {['%.3f' % v for v in low]} .. {['%.3f' % v for v in high]}")
    if wrist is not None:
        inside = all(low[i] - 0.05 <= wrist[i] <= high[i] + 0.05 for i in range(3))
        print(f"  wrist at {['%.3f' % v for v in wrist]} — {'inside' if inside else 'OUTSIDE'} the mesh")
        if not inside:
            raise SystemExit("the mesh and its skeleton are in different places")

    export(out_path)


def main() -> None:
    args = parse_args()
    human_service, _ = import_mpfb()

    os.makedirs(args.out_dir, exist_ok=True)
    for side, name in (("L", "hand-left.glb"), ("R", "hand-right.glb")):
        build_hand(human_service, args, side, os.path.join(args.out_dir, name))


if __name__ == "__main__":
    main()
