use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

const TEXTURE_WIDTH: u32 = 128;
const TEXTURE_HEIGHT: u32 = 512;

type Rgba = [u8; 4];

fn blend(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2]), mix(a[3], b[3])]
}

fn generate(shade: impl Fn(f32, f32) -> Rgba) -> Image {
    sized(TEXTURE_WIDTH, TEXTURE_HEIGHT, shade)
}

fn sized(width: u32, height: u32, shade: impl Fn(f32, f32) -> Rgba) -> Image {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let v = y as f32 / (height - 1) as f32;
        for x in 0..width {
            let u = x as f32 / (width - 1) as f32;
            data.extend_from_slice(&shade(u, v));
        }
    }
    Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
}

fn rounded_box_distance(point: (f32, f32), half: (f32, f32), radius: f32) -> f32 {
    let qx = point.0.abs() - half.0 + radius;
    let qy = point.1.abs() - half.1 + radius;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

const SEAM_DARKNESS: f32 = 0.85;

const SEAM_WIDTH: f32 = 0.032;

pub fn white_key() -> Image {
    let ivory: Rgba = [219, 222, 238, 255];
    let seam: Rgba = [146, 120, 82, 255];
    generate(move |u, v| {
        let along = 0.93 + 0.07 * v;
        let base = blend([0, 0, 0, 255], ivory, along);
        let from_edge = u.min(1.0 - u) / SEAM_WIDTH;
        let shade = if from_edge >= 1.0 {
            0.0
        } else {
            (1.0 - from_edge).powi(2) * SEAM_DARKNESS
        };
        blend(base, seam, shade)
    })
}

pub fn black_key() -> Image {
    let body: Rgba = [5, 4, 27, 255];
    let sheen: Rgba = [46, 40, 88, 255];
    let tip: Rgba = [58, 52, 104, 255];
    generate(move |u, v| {
        let from_band = ((u - 0.40) / 0.14).abs();
        let gloss = (1.0 - from_band).clamp(0.0, 1.0).powi(3);
        let mut colour = blend(body, sheen, gloss * 0.40);
        if v > 0.93 {
            colour = blend(colour, tip, ((v - 0.93) / 0.07).powf(0.7));
        }
        let from_edge = u.min(1.0 - u) / 0.03;
        if from_edge < 1.0 {
            colour = blend(colour, [0, 0, 0, 255], (1.0 - from_edge) * 0.55);
        }
        colour
    })
}

const NOTE_CORNER: f32 = 0.21;

pub fn note_bar() -> Image {
    sized(NOTE_TEXTURE_SIZE, NOTE_TEXTURE_SIZE, |u, v| {
        let point = (u - 0.5, v - 0.5);
        let half = (0.5 - EDGE_FEATHER, 0.5 - EDGE_FEATHER);
        let distance = rounded_box_distance(point, half, NOTE_CORNER);

        let coverage = (-distance / EDGE_FEATHER).clamp(0.0, 1.0);
        if coverage <= 0.0 {
            return [255, 255, 255, 0];
        }

        let inside = (-distance).max(0.0);
        let rim = (1.0 - inside / RIM_WIDTH).clamp(0.0, 1.0).powf(1.1);

        let level = (NOTE_BODY * crystal(u, v) + (1.0 - NOTE_BODY) * rim).clamp(0.0, 1.0);
        let level = (level * 255.0) as u8;
        [level, level, level, (coverage * 255.0) as u8]
    })
}

fn crystal(u: f32, v: f32) -> f32 {
    let phase = (u * CRYSTAL_SLANT + v / NOTE_BAND) * std::f32::consts::TAU;
    let broad = phase.sin();
    let fine = (phase * 2.0 + 1.1).sin();
    1.0 + CRYSTAL_DEPTH * (0.68 * broad + 0.32 * fine)
}

pub const NOTE_TEXTURE_SIZE: u32 = 128;

pub const NOTE_CAP: f32 = 0.3;

pub const NOTE_BAND: f32 = 1.0 - 2.0 * NOTE_CAP;

const NOTE_BODY: f32 = 0.26;

const CRYSTAL_SLANT: f32 = 0.45;
const CRYSTAL_DEPTH: f32 = 0.38;

const EDGE_FEATHER: f32 = 0.035;

const RIM_WIDTH: f32 = 0.11;

pub fn back_rail() -> Image {
    let felt: Rgba = [104, 24, 34, 255];
    let dark: Rgba = [26, 6, 9, 255];
    generate(move |_, v| blend(dark, felt, v.powf(0.55)))
}

pub fn back_glow() -> Image {
    generate(|_, v| {
        let level = v.powf(4.5);
        [255, 255, 255, (level * 255.0) as u8]
    })
}

pub fn hit_line() -> Image {
    generate(|_, v| {
        let across = (1.0 - (v - 0.5).abs() * 2.0).clamp(0.0, 1.0);
        let core = across.powf(9.0);
        let halo = across.powf(1.6);
        let level = (core + halo * 0.42).min(1.0);
        [255, 255, 255, (level * 255.0) as u8]
    })
}

pub fn case() -> Image {
    let near: Rgba = [0, 0, 0, 255];
    let far: Rgba = [11, 11, 14, 255];
    generate(move |_, v| {
        let sheen = v.powf(2.4);
        blend(near, far, sheen)
    })
}

pub fn octave_line() -> Image {
    generate(|u, _| {
        let across = (1.0 - (u - 0.5).abs() * 2.0).clamp(0.0, 1.0);
        let core = across.powf(24.0);
        let halo = across.powf(3.0);
        [255, 255, 255, ((core * 0.55 + halo * 0.16) * 255.0) as u8]
    })
}

pub fn flash() -> Image {
    generate(|u, v| {
        let (dx, dy) = ((u - 0.5) * 2.0, (v - 0.5) * 2.6);
        let radius = (dx * dx + dy * dy).sqrt();
        let falloff = (1.0 - radius).clamp(0.0, 1.0);

        let core = falloff.powf(7.0);
        let body = falloff.powf(2.4);

        let level = (core + body * 0.34).clamp(0.0, 1.0);
        [255, 255, 255, (level * 255.0) as u8]
    })
}

pub fn spark() -> Image {
    generate(|u, v| {
        let (dx, dy) = (u - 0.5, v - 0.5);
        let radius = (dx * dx + dy * dy).sqrt() * 2.0;
        let core = (1.0 - radius).clamp(0.0, 1.0).powf(0.45);
        let halo = (1.0 - radius).clamp(0.0, 1.0).powf(3.0);
        let level = (core * 0.85 + halo * 0.15).min(1.0);
        [255, 255, 255, (level * 255.0) as u8]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(image: &Image, u: f32, v: f32) -> Rgba {
        let (width, height) = (image.width(), image.height());
        let x = (u * (width - 1) as f32).round() as u32;
        let y = (v * (height - 1) as f32).round() as u32;
        let index = ((y * width + x) * 4) as usize;
        let data = image.data.as_ref().expect("generated images keep their data");
        [data[index], data[index + 1], data[index + 2], data[index + 3]]
    }

    fn luminance(c: Rgba) -> f32 {
        0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32
    }

    #[test]
    fn the_generated_images_are_the_size_they_claim() {
        for image in [
            white_key(),
            black_key(),
            back_rail(),
            back_glow(),
            hit_line(),
            spark(),
            case(),
            octave_line(),
            flash(),
        ] {
            assert_eq!(image.width(), TEXTURE_WIDTH);
            assert_eq!(image.height(), TEXTURE_HEIGHT);
            assert_eq!(
                image.data.as_ref().unwrap().len(),
                (TEXTURE_WIDTH * TEXTURE_HEIGHT * 4) as usize
            );
        }
    }

    #[test]
    fn the_note_is_square_so_its_ends_keep_their_shape() {
        let image = note_bar();
        assert_eq!(image.width(), NOTE_TEXTURE_SIZE);
        assert_eq!(image.height(), NOTE_TEXTURE_SIZE);
        assert_eq!(
            image.data.as_ref().unwrap().len(),
            (NOTE_TEXTURE_SIZE * NOTE_TEXTURE_SIZE * 4) as usize
        );
    }

    #[test]
    fn a_white_key_is_pale_in_the_middle_and_has_a_seam_at_each_side() {
        let image = white_key();
        let middle = luminance(pixel(&image, 0.5, 0.5));
        let edge = luminance(pixel(&image, 0.004, 0.5));
        assert!(
            (150.0..238.0).contains(&middle),
            "a white key should read as a light grey: {middle}"
        );
        let seam = pixel(&image, 0.004, 0.5);
        assert!(edge < middle * 0.80, "the seam should be clearly darker: {edge}");
        assert!(
            seam[0] as i32 - seam[2] as i32 > 20,
            "the seam should be warm, not neutral: {seam:?}"
        );
    }

    #[test]
    fn a_black_key_is_dark_but_carries_a_highlight() {
        let image = black_key();
        let band = luminance(pixel(&image, 0.40, 0.4));
        let away = luminance(pixel(&image, 0.85, 0.4));
        assert!(away < 45.0, "a black key should be dark away from the gloss: {away}");
        assert!(band > away * 2.0, "the gloss band should stand out: {band} vs {away}");
    }

    #[test]
    fn a_black_key_is_darker_than_a_white_one_everywhere() {
        let (black, white) = (black_key(), white_key());
        for v in [0.1f32, 0.5, 0.9] {
            assert!(luminance(pixel(&black, 0.5, v)) < luminance(pixel(&white, 0.5, v)));
        }
    }

    #[test]
    fn the_signed_distance_is_negative_inside_and_positive_outside() {
        let half = (0.5, 2.0);
        assert!(rounded_box_distance((0.0, 0.0), half, 0.25) < 0.0);
        assert!(rounded_box_distance((0.9, 0.0), half, 0.25) > 0.0);
        assert!(rounded_box_distance((0.0, 3.0), half, 0.25) > 0.0);
        assert!(rounded_box_distance((0.49, 1.99), half, 0.25) > 0.0);
    }

    #[test]
    fn a_note_is_a_rounded_rectangle_rather_than_a_capsule() {
        let image = note_bar();
        assert_eq!(pixel(&image, 0.5, 0.5)[3], 255);
        assert!(pixel(&image, 0.90, 0.5)[3] > 200, "the sides should still be square");
        assert_eq!(pixel(&image, 0.02, 0.002)[3], 0, "the corner should be cut away");
        assert_eq!(pixel(&image, 0.98, 0.998)[3], 0, "and the opposite corner too");
    }

    #[test]
    fn a_note_has_a_brighter_rim_than_its_middle() {
        let image = note_bar();
        let centre = luminance(pixel(&image, 0.5, 0.5));
        let rim = luminance(pixel(&image, 0.075, 0.5));
        assert!(rim > centre, "the rim {rim} should be brighter than the body {centre}");
    }

    #[test]
    fn a_notes_edge_is_soft_rather_than_a_hard_step() {
        let image = note_bar();
        let alphas: Vec<u8> = (0..14).map(|i| pixel(&image, i as f32 / 128.0, 0.5)[3]).collect();
        assert!(
            alphas.iter().any(|a| (20..235).contains(a)),
            "no partly covered pixels along the edge: {alphas:?}"
        );
    }

    #[test]
    fn the_hit_line_is_brightest_along_its_centre() {
        let image = hit_line();
        assert!(pixel(&image, 0.5, 0.5)[3] > pixel(&image, 0.5, 0.1)[3]);
        assert!(pixel(&image, 0.5, 0.5)[3] > pixel(&image, 0.5, 0.9)[3]);
    }
}
