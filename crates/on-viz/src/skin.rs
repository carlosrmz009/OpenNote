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
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
}

const UNDER_RAIL: f32 = 0.05;

pub fn white_key() -> Image {
    let back: Rgba = [198, 193, 181, 255];
    let front: Rgba = [228, 225, 216, 255];
    generate(move |_, v| {
        let colour = blend(back, front, v.powf(0.8));
        let shade = (1.0 - v / UNDER_RAIL).clamp(0.0, 1.0).powi(2);
        blend(colour, [120, 112, 100, 255], shade * 0.5)
    })
}

const BEVEL: f32 = 0.14;

pub fn black_key() -> Image {
    let back: Rgba = [16, 15, 15, 255];
    let front: Rgba = [6, 6, 6, 255];
    generate(move |u, v| {
        let colour = blend(back, front, v);
        let lit = ((u - (1.0 - BEVEL)) / BEVEL).clamp(0.0, 1.0);
        let colour = blend(colour, [62, 60, 58, 255], lit * 0.8);
        let shaded = (1.0 - u / BEVEL).clamp(0.0, 1.0);
        let colour = blend(colour, [0, 0, 0, 255], shaded);
        let across = (1.0 - ((u - 0.5) / 0.2).abs()).clamp(0.0, 1.0).powf(0.6);
        let fade = (1.0 - v / 0.85).clamp(0.0, 1.0);
        blend(colour, [255, 255, 255, 255], across * fade * 0.32)
    })
}

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

pub const FLASH_STREAK: f32 = 2.4;

pub fn flash() -> Image {
    generate(|u, v| {
        let (dx, dy) = ((u - 0.5) * 2.0 * FLASH_STREAK, (v - 0.5) * 2.6);
        let radius = (dx * dx + dy * dy).sqrt();
        let falloff = (1.0 - radius).clamp(0.0, 1.0);

        let core = falloff.powf(7.0);
        let body = falloff.powf(2.4);

        let reach = (1.0 - (u - 0.5).abs() * 2.0).clamp(0.0, 1.0);
        let streak = (-(dy / 0.07).powi(2)).exp() * reach.powf(2.2);

        let level = (core + body * 0.34 + streak * 0.30).clamp(0.0, 1.0);
        [255, 255, 255, (level * 255.0) as u8]
    })
}

pub fn beam() -> Image {
    generate(|u, v| {
        let across = (u - 0.5) * 2.0;
        let shaft = (-(across / 0.30).powi(2)).exp();
        let haze = (1.0 - across.abs()).clamp(0.0, 1.0).powf(1.5);
        let rise = v.powf(2.6);
        let level = ((shaft * 0.75 + haze * 0.25) * rise).clamp(0.0, 1.0);
        [255, 255, 255, (level * 255.0) as u8]
    })
}

pub fn spark() -> Image {
    generate(|u, v| {
        let (dx, dy) = (u - 0.5, v - 0.5);
        let radius = (dx * dx + dy * dy).sqrt() * 2.0;
        let core = (1.0 - radius).clamp(0.0, 1.0).powf(0.45);
        let halo = (1.0 - radius).clamp(0.0, 1.0).powf(3.0);
        let head = 0.35 + 0.65 * (1.0 - v).powf(1.4);
        let level = ((core * 0.85 + halo * 0.15) * head).min(1.0);
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
            beam(),
        ] {
            assert_eq!(image.width(), TEXTURE_WIDTH);
            assert_eq!(image.height(), TEXTURE_HEIGHT);
            assert_eq!(image.data.as_ref().unwrap().len(), (TEXTURE_WIDTH * TEXTURE_HEIGHT * 4) as usize);
        }
    }

    #[test]
    fn a_white_key_is_warm_ivory_that_brightens_toward_the_player() {
        let image = white_key();
        let middle = pixel(&image, 0.5, 0.5);
        assert!(luminance(middle) > 180.0, "a white key should be pale: {middle:?}");
        assert!(middle[0] > middle[2], "ivory is warm, not blue: {middle:?}");
        assert!(luminance(pixel(&image, 0.5, 0.95)) > luminance(pixel(&image, 0.5, 0.2)));
        assert!(luminance(pixel(&image, 0.5, 0.0)) < luminance(middle) * 0.85);
    }

    #[test]
    fn a_black_key_is_dark_but_carries_a_highlight() {
        let image = black_key();
        let band = luminance(pixel(&image, 0.5, 0.1));
        let away = luminance(pixel(&image, 0.85, 0.1));
        assert!(away < 45.0, "a black key should be dark away from the gloss: {away}");
        assert!(band > away * 1.4, "the gloss band should stand out: {band} vs {away}");
    }

    #[test]
    fn a_black_key_is_darker_than_a_white_one_everywhere() {
        let (black, white) = (black_key(), white_key());
        for v in [0.1f32, 0.5, 0.9] {
            assert!(luminance(pixel(&black, 0.5, v)) < luminance(pixel(&white, 0.5, v)));
        }
    }

    #[test]
    fn the_hit_line_is_brightest_along_its_centre() {
        let image = hit_line();
        assert!(pixel(&image, 0.5, 0.5)[3] > pixel(&image, 0.5, 0.1)[3]);
        assert!(pixel(&image, 0.5, 0.5)[3] > pixel(&image, 0.5, 0.9)[3]);
    }
}
