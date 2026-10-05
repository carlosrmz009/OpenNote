from __future__ import annotations

import argparse
import os
import sys

try:
    import bpy
except ImportError:
    print("This script has to be run inside Blender:", file=sys.stderr)
    print("  blender --background --python tools/export_hand.py -- --out <path>", file=sys.stderr)
    raise SystemExit(2)

def import_mpfb():
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
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for collection in (bpy.data.meshes, bpy.data.armatures, bpy.data.objects):
        for item in list(collection):
            collection.remove(item)

HAND_BONE_HINTS = ("finger", "metacarpal", "wrist", "hand", "lowerarm", "forearm")

def is_hand_bone(name: str) -> bool:
    lowered = name.lower()
    return any(hint in lowered for hint in HAND_BONE_HINTS)

def keep_only_hands(basemesh, armature, side: str) -> None:
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
    if not mesh.verts:
        return 0.0
    lo = [min(v.co[axis] for v in mesh.verts) for axis in range(3)]
    hi = [max(v.co[axis] for v in mesh.verts) for axis in range(3)]
    return max(hi[axis] - lo[axis] for axis in range(3))

def boundary_loops(mesh) -> list:
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
    delta = armature.matrix_world.inverted() @ basemesh.matrix_world
    offset = delta.to_translation()
    if offset.length > 1e-6:
        print(f"  mesh sits {offset.length:.4f} units from the rig; moving it into rig space")
    basemesh.data.transform(delta)
    basemesh.matrix_world = armature.matrix_world.copy()
    basemesh.data.update()

def bone_head(armature, name: str):
    bone = armature.data.bones.get(name)
    return armature.matrix_world @ bone.head_local if bone else None

def bone_tail(armature, name: str):
    bone = armature.data.bones.get(name)
    return armature.matrix_world @ bone.tail_local if bone else None

def smooth_surface(basemesh, levels: int) -> None:
    if levels <= 0:
        return
    bpy.context.view_layer.objects.active = basemesh

    if basemesh.data.shape_keys is not None:
        bpy.ops.object.shape_key_remove(all=True, apply_mix=True)

    for modifier in list(basemesh.modifiers):
        if modifier.type == "MASK":
            bpy.ops.object.modifier_apply(modifier=modifier.name)

    modifier = basemesh.modifiers.new("Subdivide", "SUBSURF")
    modifier.levels = levels
    modifier.render_levels = levels
    bpy.ops.object.modifier_move_to_index(modifier=modifier.name, index=0)
    bpy.ops.object.modifier_apply(modifier=modifier.name)

    for polygon in basemesh.data.polygons:
        polygon.use_smooth = True
    print(f"  subdivided to {len(basemesh.data.vertices)} vertices")
    mended = mend_mesh_holes(basemesh)
    if mended:
        print(f"  mended {mended} punctures")

def add_skin_material(basemesh) -> None:
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
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.export_scene.gltf(
        filepath=path,
        export_format="GLB",
        export_skins=True,
        export_animations=False,
        export_apply=False,
        use_selection=True,
        export_yup=False,
        export_tangents=True,
    )
    print(f"wrote {path}")

def build_hand(human_service, args, side: str, out_path: str) -> None:
    label = {"L": "left", "R": "right"}[side]
    print(f"building the {label} hand")

    clear_scene()
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

    if wrist is not None and tip is not None:
        direction = (tip - wrist).normalized()
        print(f"  hand is {(tip - wrist).length:.4f} units long, fingers pointing {direction}")

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
