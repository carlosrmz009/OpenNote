use bevy::asset::RenderAssetUsages;
use bevy::prelude::default;
use bevy::image::Image;
use bevy::math::Vec3;
use bevy::mesh::{Indices, Mesh, VertexAttributeValues};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

const GLOVE_STANDOFF: f32 = 0.0032;

const GLOVE_CUFF_AT: f32 = 0.06;

const GLOVE_WEAVE_TILES: f32 = 26.0;

#[derive(Debug, Clone, Copy)]
pub struct HandFrame {
    pub wrist: Vec3,
    pub along: Vec3,
    pub across: Vec3,
    pub dorsal: Vec3,
    pub palm: f32,
}

impl HandFrame {
    pub fn new(wrist: Vec3, middle: Vec3, index: Vec3, little: Vec3, thumb_tip: Vec3) -> Self {
        let along = (middle - wrist).normalize_or_zero();
        let across = (little - index).normalize_or_zero();
        let mut dorsal = along.cross(across).normalize_or_zero();
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

    fn place(&self, point: Vec3) -> (f32, f32, f32) {
        let offset = (point - self.wrist) / self.palm;
        (
            offset.dot(self.along),
            offset.dot(self.across),
            offset.dot(self.dorsal),
        )
    }
}

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

    let covered: Vec<bool> = (0..positions.len())
        .map(|i| {
            let carried_by_arm = arm_weighted(&joints[i], &weights[i], on_the_arm);
            let (along, _, _) = frame.place(Vec3::from(positions[i]));
            !carried_by_arm && along >= GLOVE_CUFF_AT
        })
        .collect();

    let mut keep = Vec::new();
    for triangle in indices.chunks_exact(3) {
        if triangle.iter().all(|i| covered[*i as usize]) {
            keep.extend_from_slice(triangle);
        }
    }
    if keep.is_empty() {
        return None;
    }

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

    fn flat() -> HandFrame {
        HandFrame::new(
            Vec3::ZERO,
            Vec3::new(0.0, 100.0, 0.0),
            Vec3::new(-30.0, 100.0, 0.0),
            Vec3::new(30.0, 100.0, 0.0),
            Vec3::new(-40.0, 40.0, -20.0),
        )
    }

    #[test]
    fn the_frame_finds_the_back_of_the_hand() {
        let frame = flat();
        assert!(frame.dorsal.z > 0.9, "dorsal should be +Z: {:?}", frame.dorsal);
        assert!((frame.palm - 100.0).abs() < 1e-3);
    }

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
        let (hand, on_the_arm) = strip();
        let glove = shell(&hand, &on_the_arm, &flat()).expect("a glove");
        for point in positions_of(&glove) {
            assert!(
                (point[2] - GLOVE_STANDOFF).abs() < 1e-6,
                "vertex at {point:?} did not stand off"
            );
        }
    }

    #[test]
    fn the_glove_is_driven_by_the_hands_own_skeleton() {
        let (hand, on_the_arm) = strip();
        let glove = shell(&hand, &on_the_arm, &flat()).expect("a glove");
        assert!(glove.attribute(Mesh::ATTRIBUTE_JOINT_INDEX).is_some());
        assert!(glove.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT).is_some());
        assert!(glove.attribute(Mesh::ATTRIBUTE_TANGENT).is_some());
    }

    #[test]
    fn the_cloth_is_tiled_rather_than_laid_out() {
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
