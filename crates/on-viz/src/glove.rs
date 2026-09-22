//! The cartoon gloves the hands wear.
//!
//! The glove is its own surface: a copy of the hand's mesh, pushed out along its
//! normals and cut back to the part a glove covers, carrying the hand's own joints and
//! weights so the same skeleton drives both. The hand underneath is left bare, and what
//! shows of it is the arm and a little of the wrist.
//!
//! It used to be painted into the hand's own texture instead, and a painted glove can
//! only ever be a colour. The cuff was a line where the colour changed rather than an
//! edge; the fingers of the glove were exactly as thick as the fingers of the hand; and
//! the cloth had to be baked at whatever scale the model's texture layout gave it,
//! which — MakeHuman laying a whole body over one sheet — is a different scale on the
//! hand than on the arm. A weave coarse enough to survive that came out as bumps, and
//! bumps on a hand look like poultry.
//!
//! As a surface of its own it can be tiled instead, as finely as it likes.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::default;
use bevy::image::Image;
use bevy::math::Vec3;
use bevy::mesh::{Indices, Mesh, VertexAttributeValues};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// How far the glove stands off the hand, in the model's own units.
///
/// The glove is a second surface rather than a colour on the first one, which is what
/// gives it an edge you can see against the skin at the cuff and a silhouette a little
/// fuller than the hand inside it. Real leather is under a millimetre; this is thicker
/// than that, because a glove that stands off by exactly its own thickness reads as
/// paint again.
const GLOVE_STANDOFF: f32 = 0.0032;

/// How far up the hand from the wrist the glove's cuff sits.
///
/// Zero is the wrist and one is the middle knuckle. Just above the wrist: a glove ends
/// where the hand does. Earlier attempts put it well down the forearm, which read as a
/// gauntlet, and pulling it up onto the palm left the hand looking bandaged.
const GLOVE_CUFF_AT: f32 = 0.06;

/// How many times the cloth repeats across the glove's texture coordinates.
///
/// High, because the weave has to be finer than the eye can resolve at this size or it
/// stops being cloth and becomes bumps.
const GLOVE_WEAVE_TILES: f32 = 26.0;

/// A hand's own frame, taken from where its bones sit in the model.
#[derive(Debug, Clone, Copy)]
pub struct HandFrame {
    /// The wrist, which everything is measured from.
    pub wrist: Vec3,
    /// Along the hand, towards the knuckles.
    pub along: Vec3,
    /// Across the knuckles, towards the little finger.
    pub across: Vec3,
    /// Out through the back of the hand.
    pub dorsal: Vec3,
    /// Wrist to middle knuckle, which is the unit everything else is in.
    pub palm: f32,
}

impl HandFrame {
    /// Build a frame from the wrist, the middle knuckle, the two outer knuckles and a
    /// point on the palm side — the thumb's tip, which is the easiest thing to be sure
    /// about the sign of.
    pub fn new(wrist: Vec3, middle: Vec3, index: Vec3, little: Vec3, thumb_tip: Vec3) -> Self {
        let along = (middle - wrist).normalize_or_zero();
        let across = (little - index).normalize_or_zero();
        let mut dorsal = along.cross(across).normalize_or_zero();
        // The thumb curls over the palm, so whichever way it lies is the front.
        if (thumb_tip - wrist).dot(dorsal) > 0.0 {
            dorsal = -dorsal;
        }
        Self {
            wrist,
            along,
            across,
            dorsal,
            palm: (middle - wrist).length().max(1e-6),
        }
    }

    /// Where a point sits in this frame, in units of the palm's length.
    fn place(&self, point: Vec3) -> (f32, f32, f32) {
        let offset = (point - self.wrist) / self.palm;
        (
            offset.dot(self.along),
            offset.dot(self.across),
            offset.dot(self.dorsal),
        )
    }
}

/// The cloth the gloves are cut from, as a tiling texture.
///
/// The full-resolution photograph rather than the box-averaged one the painted glove
/// used: this is repeated across the glove's own texture coordinates instead of being
/// laid into a texture at whatever scale the hand's layout gives, so it can be as fine
/// as it likes and there is no resampling to alias.
///
/// Read as linear rather than sRGB, because a normal map is a direction and not a
/// colour — decoding it through a gamma curve tilts every normal towards flat.
pub fn cloth(assets: &std::path::Path) -> Option<Image> {
    let normal = image::open(assets.join("gloves").join("weave-normal.png"))
        .ok()?
        .to_rgba8();
    let (width, height) = normal.dimensions();
    let mut image = Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        normal.into_raw(),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::Repeat,
        address_mode_v: bevy::image::ImageAddressMode::Repeat,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    });
    Some(image)
}

/// Build the glove as its own surface, standing off the hand that wears it.
///
/// A copy of the hand's own mesh, pushed out along its normals and cut back to the part
/// a glove covers. It keeps the hand's joints and weights, so it is driven by the same
/// skeleton and bends with the fingers without anything having to keep the two in step.
///
/// Painting the glove into the hand's texture — which is what this replaces — could
/// only ever be a colour. The cuff was a line where the colour changed rather than an
/// edge, the cloth had to be baked at whatever scale the hand's texture layout happened
/// to give it, and the fingers of the glove were exactly as thick as the fingers of the
/// hand.
pub fn shell(mesh: &Mesh, on_the_arm: &[bool], frame: &HandFrame) -> Option<Mesh> {
    let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION)? {
        VertexAttributeValues::Float32x3(values) => values.clone(),
        _ => return None,
    };
    let normals = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL)? {
        VertexAttributeValues::Float32x3(values) => values.clone(),
        _ => return None,
    };
    let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0)? {
        VertexAttributeValues::Float32x2(values) => values.clone(),
        _ => return None,
    };
    let weights = match mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)? {
        VertexAttributeValues::Float32x4(values) => values.clone(),
        _ => return None,
    };
    let joints: Vec<[u16; 4]> = match mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)? {
        VertexAttributeValues::Uint16x4(values) => values.clone(),
        VertexAttributeValues::Uint8x4(values) => {
            values.iter().map(|v| v.map(u16::from)).collect()
        }
        _ => return None,
    };
    let indices: Vec<u32> = match mesh.indices()? {
        Indices::U32(values) => values.clone(),
        Indices::U16(values) => values.iter().map(|v| *v as u32).collect(),
    };

    // What the glove covers: the hand, from the cuff to the fingertips. Both tests are
    // needed. The bones say which vertices the arm carries, and the frame says how far
    // up the hand a vertex sits — the wrist is where one becomes the other, and neither
    // question answers the other one.
    let covered: Vec<bool> = (0..positions.len())
        .map(|i| {
            let carried_by_arm = arm_weighted(&joints[i], &weights[i], on_the_arm);
            let (along, _, _) = frame.place(Vec3::from(positions[i]));
            !carried_by_arm && along >= GLOVE_CUFF_AT
        })
        .collect();

    // Only whole triangles, so the cuff is a clean edge rather than a fringe.
    let mut keep = Vec::new();
    for triangle in indices.chunks_exact(3) {
        if triangle.iter().all(|i| covered[*i as usize]) {
            keep.extend_from_slice(triangle);
        }
    }
    if keep.is_empty() {
        return None;
    }

    // Compact to the vertices the kept triangles actually use.
    let mut moved = vec![u32::MAX; positions.len()];
    let (mut out_positions, mut out_normals, mut out_uvs) = (Vec::new(), Vec::new(), Vec::new());
    let (mut out_joints, mut out_weights) = (Vec::new(), Vec::new());
    for index in &mut keep {
        let old = *index as usize;
        if moved[old] == u32::MAX {
            moved[old] = out_positions.len() as u32;
            let normal = Vec3::from(normals[old]).normalize_or_zero();
            out_positions.push((Vec3::from(positions[old]) + normal * GLOVE_STANDOFF).to_array());
            out_normals.push(normals[old]);
            // The cloth is tiled rather than laid out, so the hand's own texture layout
            // — which maps the palm and the fingers at quite different densities — stops
            // deciding how coarse the weave is.
            out_uvs.push([uvs[old][0] * GLOVE_WEAVE_TILES, uvs[old][1] * GLOVE_WEAVE_TILES]);
            out_joints.push(joints[old]);
            out_weights.push(weights[old]);
        }
        *index = moved[old];
    }

    let mut glove = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    glove.insert_attribute(Mesh::ATTRIBUTE_POSITION, out_positions);
    glove.insert_attribute(Mesh::ATTRIBUTE_NORMAL, out_normals);
    glove.insert_attribute(Mesh::ATTRIBUTE_UV_0, out_uvs);
    glove.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(out_joints));
    glove.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, out_weights);
    glove.insert_indices(Indices::U32(keep));
    let _ = glove.generate_tangents();
    Some(glove)
}

/// Whether the arm moves a vertex more than the hand does.
fn arm_weighted(joints: &[u16; 4], weights: &[f32; 4], on_the_arm: &[bool]) -> bool {
    let (mut on_arm, mut on_hand) = (0.0, 0.0);
    for (j, w) in joints.iter().zip(weights) {
        match on_the_arm.get(*j as usize) {
            Some(true) => on_arm += w,
            _ => on_hand += w,
        }
    }
    on_arm > on_hand
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame with the hand lying flat, fingers along +Y and the back facing +Z.
    fn flat() -> HandFrame {
        HandFrame::new(
            Vec3::ZERO,
            Vec3::new(0.0, 100.0, 0.0),
            Vec3::new(-30.0, 100.0, 0.0),
            Vec3::new(30.0, 100.0, 0.0),
            // The thumb curls over the palm, which is -Z.
            Vec3::new(-40.0, 40.0, -20.0),
        )
    }

    #[test]
    fn the_frame_finds_the_back_of_the_hand() {
        let frame = flat();
        assert!(frame.dorsal.z > 0.9, "dorsal should be +Z: {:?}", frame.dorsal);
        assert!((frame.palm - 100.0).abs() < 1e-3);
    }

    /// A strip of triangles running up the hand, from below the wrist to the fingers,
    /// with the lower half carried by the arm.
    fn strip() -> (Mesh, Vec<bool>) {
        let rungs: Vec<f32> = vec![-0.4, -0.1, 0.2, 0.5, 0.9];
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut joints: Vec<[u16; 4]> = Vec::new();
        let mut weights: Vec<[f32; 4]> = Vec::new();
        for (rung, along) in rungs.iter().enumerate() {
            for side in [-20.0f32, 20.0] {
                positions.push([side, along * 100.0, 0.0]);
                normals.push([0.0, 0.0, 1.0]);
                uvs.push([(side + 20.0) / 40.0, rung as f32 / 4.0]);
                // Joint 0 is the forearm, joint 1 the hand. Below the wrist the arm
                // carries it; above, the hand does.
                joints.push([0, 1, 0, 0]);
                weights.push(if *along < 0.0 { [1.0, 0.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0, 0.0] });
            }
        }
        let mut indices = Vec::new();
        for rung in 0..rungs.len() as u32 - 1 {
            let (a, b) = (rung * 2, rung * 2 + 2);
            indices.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
        }
        let mut mesh = Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(joints));
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
        mesh.insert_indices(Indices::U32(indices));
        (mesh, vec![true, false])
    }

    fn positions_of(mesh: &Mesh) -> Vec<[f32; 3]> {
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
            VertexAttributeValues::Float32x3(values) => values.clone(),
            _ => panic!("no positions"),
        }
    }

    #[test]
    fn the_glove_covers_the_hand_and_stops_at_the_wrist() {
        let (hand, on_the_arm) = strip();
        let glove = shell(&hand, &on_the_arm, &flat()).expect("a glove");
        let along: Vec<f32> = positions_of(&glove)
            .iter()
            .map(|p| flat().place(Vec3::from(*p)).0)
            .collect();
        assert!(!along.is_empty(), "the glove should cover something");
        assert!(
            along.iter().all(|a| *a >= GLOVE_CUFF_AT - 0.01),
            "nothing below the cuff: {along:?}"
        );
        assert!(
            along.iter().any(|a| *a > 0.8),
            "and it should reach the fingers: {along:?}"
        );
    }

    #[test]
    fn the_glove_stands_off_the_hand_it_is_worn_on() {
        // Not a colour on the hand's own surface: a surface of its own, a little outside
        // it. That is what gives the cuff an edge and the fingers a little thickness.
        let (hand, on_the_arm) = strip();
        let glove = shell(&hand, &on_the_arm, &flat()).expect("a glove");
        // The strip's normals all point at +Z, so every vertex should have moved that
        // way by the standoff and not at all in any other direction.
        for point in positions_of(&glove) {
            assert!(
                (point[2] - GLOVE_STANDOFF).abs() < 1e-6,
                "vertex at {point:?} did not stand off"
            );
        }
    }

    #[test]
    fn the_glove_is_driven_by_the_hands_own_skeleton() {
        // It carries the hand's joints and weights, so nothing has to keep the two in
        // step as the fingers bend — they are posed by the same bones.
        let (hand, on_the_arm) = strip();
        let glove = shell(&hand, &on_the_arm, &flat()).expect("a glove");
        assert!(glove.attribute(Mesh::ATTRIBUTE_JOINT_INDEX).is_some());
        assert!(glove.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT).is_some());
        // And tangents, or the cloth's normal map has no frame to be read in.
        assert!(glove.attribute(Mesh::ATTRIBUTE_TANGENT).is_some());
    }

    #[test]
    fn the_cloth_is_tiled_rather_than_laid_out() {
        // The hand's own texture layout maps the palm and the fingers at quite different
        // densities, so a weave laid into it is coarse in one place and fine in another.
        // Tiled, the scale is the glove's own business.
        let (hand, on_the_arm) = strip();
        let glove = shell(&hand, &on_the_arm, &flat()).expect("a glove");
        let uvs = match glove.attribute(Mesh::ATTRIBUTE_UV_0).unwrap() {
            VertexAttributeValues::Float32x2(values) => values.clone(),
            _ => panic!("no uvs"),
        };
        let widest = uvs.iter().map(|uv| uv[0]).fold(0.0f32, f32::max);
        assert!(widest > 1.0, "the cloth should repeat, not stretch: {widest}");
    }
}
