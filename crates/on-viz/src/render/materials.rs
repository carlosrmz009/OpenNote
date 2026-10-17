//! The lacquered key material, the glowing note material, and colour helpers.

use super::*;

pub(crate) type Lacquered = bevy::pbr::ExtendedMaterial<StandardMaterial, Lacquer>;

#[derive(Asset, TypePath, bevy::render::render_resource::AsBindGroup, Clone)]
pub(crate) struct Lacquer {
    #[uniform(100)]
    pub(crate) finish: LacquerFinish,
}

#[derive(bevy::render::render_resource::ShaderType, Clone)]
pub(crate) struct LacquerFinish {
    pub(crate) shine: Vec4,
    pub(crate) eye: Vec4,
    pub(crate) strip: Vec4,
}

impl bevy::pbr::MaterialExtension for Lacquer {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "embedded://on_viz/render/key.wgsl".into()
    }
}

pub(crate) fn key_material(image: Handle<Image>, black: bool, eye: Vec3) -> Lacquered {
    bevy::pbr::ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(image),
            perceptual_roughness: if black { BLACK_KEY_ROUGHNESS } else { WHITE_KEY_ROUGHNESS },
            reflectance: 0.5,
            ..default()
        },
        extension: Lacquer {
            finish: LacquerFinish {
                shine: if black { BLACK_LACQUER } else { WHITE_LACQUER },
                eye: eye.extend(0.0),
                strip: LACQUER_STRIP,
            },
        },
    }
}

#[derive(Asset, TypePath, bevy::render::render_resource::AsBindGroup, Clone)]
pub(crate) struct NoteMaterial {
    #[uniform(0)]
    pub(crate) tint: Vec4,
}

impl Material for NoteMaterial {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "embedded://on_viz/render/note.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}

pub(crate) fn note_material(tint: Color) -> NoteMaterial {
    let colour = tint.to_linear();
    NoteMaterial { tint: Vec4::new(colour.red, colour.green, colour.blue, NOTE_GLOW) }
}

pub(crate) fn glowing(tint: Color, brightness: f32) -> Color {
    let colour = tint.to_linear();
    Color::LinearRgba(LinearRgba {
        red: colour.red * brightness,
        green: colour.green * brightness,
        blue: colour.blue * brightness,
        alpha: 1.0,
    })
}

pub(crate) fn painted(image: Handle<Image>, tint: Color, alpha: AlphaMode) -> StandardMaterial {
    StandardMaterial {
        base_color: tint,
        base_color_texture: Some(image),
        unlit: true,
        alpha_mode: alpha,
        ..default()
    }
}

pub(crate) fn mix(a: Color, b: Color, t: f32) -> Color {
    let (a, b) = (a.to_linear(), b.to_linear());
    let t = t.clamp(0.0, 1.0);
    Color::LinearRgba(bevy::color::LinearRgba {
        red: a.red + (b.red - a.red) * t,
        green: a.green + (b.green - a.green) * t,
        blue: a.blue + (b.blue - a.blue) * t,
        alpha: 1.0,
    })
}
