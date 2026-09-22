//! The look of the keyboard and the falling notes.
//!
//! The keyboard is lit rather than painted: the black keys stand proud of the white
//! ones and cast real shadows across them, which is what stops eighty-eight keys from
//! reading as a barcode. The textures here supply what light alone cannot from
//! straight overhead — the seam between neighbouring white keys, the lacquered sheen
//! along a black key, and the soft edge of a falling note.
//!
//! Notes are drawn from a signed-distance rounded rectangle: square enough to read as
//! a bar, rounded enough not to look like a spreadsheet cell, with a bright core that
//! the bloom pass picks up into a glow.
//!
//! Everything is generated rather than shipped, so there is nothing to license,
//! nothing to keep in sync, and the palette lives in one place.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// Width in pixels of the generated textures. They are stretched across a key or a
/// note, so the resolution only has to be enough for an edge to look smooth.
const TEXTURE_WIDTH: u32 = 128;
/// Height in pixels.
const TEXTURE_HEIGHT: u32 = 512;

/// A colour with an alpha channel, in the sRGB the textures are stored in.
type Rgba = [u8; 4];

/// Linear blend between two colours.
fn blend(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2]), mix(a[3], b[3])]
}

/// Build an image from a function of position, where `u` runs across the key and `v`
/// runs from the far end toward the player.
fn generate(shade: impl Fn(f32, f32) -> Rgba) -> Image {
    sized(TEXTURE_WIDTH, TEXTURE_HEIGHT, shade)
}

/// The same, at a size of your choosing.
fn sized(width: u32, height: u32, shade: impl Fn(f32, f32) -> Rgba) -> Image {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        // Bevy's images run top-down, and the key's far end is at the top of the
        // screen, which is also where the notes arrive from.
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

/// Distance to a rounded rectangle, negative inside.
///
/// The standard signed-distance formulation. Working from a distance rather than a
/// mask is what gives a corner that is genuinely round at any aspect ratio and an
/// edge that can be softened by a known number of pixels.
fn rounded_box_distance(point: (f32, f32), half: (f32, f32), radius: f32) -> f32 {
    let qx = point.0.abs() - half.0 + radius;
    let qy = point.1.abs() - half.1 + radius;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

/// How dark the very edge of a white key is, as a fraction of the way to the seam
/// colour. The gap between two white keys is the only thing separating them.
const SEAM_DARKNESS: f32 = 0.85;

/// Width of that seam, as a fraction of the key.
const SEAM_WIDTH: f32 = 0.032;

/// The face of a white key: a defined seam down each side and a slight falloff toward
/// the back, where the fallboard shades it.
///
/// Deliberately not white. A falling-note display is lit by its notes, and a keyboard
/// painted the brightness of real ivory takes over the picture: the eye goes to the
/// biggest bright area, which is exactly where the music has already been. Holding the
/// keys at a light grey leaves the notes as the brightest things on screen without
/// making the keyboard look dirty — and a struck key, which glows, then reads as a key
/// that has been *lit* rather than one that has changed colour.
pub fn white_key() -> Image {
    let ivory: Rgba = [219, 222, 238, 255];
    // Warm, not black. The line between two white keys on a real instrument is a
    // shadowed gap with wood behind it, and drawing it dark grey is what makes a
    // keyboard read as a barcode rather than as a row of separate keys.
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

/// The top face of a black key: near-black lacquer with the bright band along its
/// length that a curved gloss surface shows from above, and a lighter chamfer at the
/// tip where it turns over toward the player.
pub fn black_key() -> Image {
    // Indigo rather than neutral. Black lacquer under a cool light is never grey, and
    // the blue is what separates the keys from their own shadows on the white ones.
    let body: Rgba = [5, 4, 27, 255];
    let sheen: Rgba = [46, 40, 88, 255];
    let tip: Rgba = [58, 52, 104, 255];
    generate(move |u, v| {
        // A narrow specular band a little off-centre, as though the light were
        // slightly to one side. Narrow and bright is what reads as lacquer.
        let from_band = ((u - 0.40) / 0.14).abs();
        let gloss = (1.0 - from_band).clamp(0.0, 1.0).powi(3);
        // Restrained. A wide bright band down the middle of every black key is the
        // single fastest way to turn a keyboard into a row of grey smudges.
        let mut colour = blend(body, sheen, gloss * 0.40);
        // The chamfer at the front, where the key turns over towards the player. It
        // catches the overhead light and is the one clearly lit part of an otherwise
        // dark key, which is what gives the row its depth.
        if v > 0.93 {
            colour = blend(colour, tip, ((v - 0.93) / 0.07).powf(0.7));
        }
        // A hairline down each side, and no more. The key is a real box standing proud
        // of the white ones, so its edges are already there in the geometry; painting a
        // gradient a tenth of the key wide on top of them only turns a crisp edge into
        // a smudge.
        let from_edge = u.min(1.0 - u) / 0.03;
        if from_edge < 1.0 {
            colour = blend(colour, [0, 0, 0, 255], (1.0 - from_edge) * 0.55);
        }
        colour
    })
}

/// Corner radius of a falling note, as a fraction of its width.
///
/// A quarter of the width reads as a rounded rectangle. Much more and the bar turns
/// into a capsule, which loses the sense of a note having a definite start and end.
const NOTE_CORNER: f32 = 0.21;

/// A falling note: a rounded rectangle of lit glass.
///
/// Not a flat slab. The reference this is built to has notes that look like backlit
/// stained glass or ice — a bright body with cloudy veins running through it, brighter
/// at the edges where the light catches, and a long streaked grain down the length. A
/// flat bar reads as a user-interface element; this reads as something lit from
/// behind, which is what makes a wall of them look like anything at all.
///
/// The veining is fractal noise stretched hard along the bar, so a long note gets long
/// streaks rather than a repeating blob. Drawn white and tinted per hand by the
/// One falling note.
///
/// Square, and cut into three kinds of piece: the top of the texture is the note's
/// head, the bottom is its tail, and the band across the middle is repeated as many
/// times as it takes to cover everything in between. That is what keeps a corner the
/// same shape on a semiquaver and on a note held for eight bars — the parts that have
/// a shape in them are never stretched.
///
/// Repeating the middle rather than stretching one row of it is what lets the note
/// carry a grain along its length. Stretch a row and you get vertical stripes and
/// nothing else; repeat a band and the grain stays the size it was drawn however long
/// the note is. The band is exactly one period of that grain, so the repeats join
/// without a seam, and the two caps continue the same function so their joins do too.
///
/// The body is dim and the rim is left at full scale. The material multiplies both by
/// a colour taken well past white, so the body lands about at full brightness and reads
/// as its own colour while the rim blows out to near-white and the bloom pass turns it
/// into the halo around the note. A note bright all the way through has every channel
/// clipped and comes out white.
pub fn note_bar() -> Image {
    sized(NOTE_TEXTURE_SIZE, NOTE_TEXTURE_SIZE, |u, v| {
        let point = (u - 0.5, v - 0.5);
        let half = (0.5 - EDGE_FEATHER, 0.5 - EDGE_FEATHER);
        let distance = rounded_box_distance(point, half, NOTE_CORNER);

        // Coverage: 1 well inside, falling to 0 across the feather.
        let coverage = (-distance / EDGE_FEATHER).clamp(0.0, 1.0);
        if coverage <= 0.0 {
            return [255, 255, 255, 0];
        }

        // The rim, measured inward from the edge.
        let inside = (-distance).max(0.0);
        let rim = (1.0 - inside / RIM_WIDTH).clamp(0.0, 1.0).powf(1.1);

        let level = (NOTE_BODY * crystal(u, v) + (1.0 - NOTE_BODY) * rim).clamp(0.0, 1.0);
        let level = (level * 255.0) as u8;
        [level, level, level, (coverage * 255.0) as u8]
    })
}

/// The grain inside a note: broad diagonal bands, like light through cut glass.
///
/// A function of one phase, so it tiles: the middle band of the texture spans exactly
/// one period, and every repeat and both caps continue the same count. Two harmonics
/// rather than one, because a pure sine reads as a painted stripe and what is wanted is
/// a facet — a wide dim sweep with a narrower bright one riding on it.
fn crystal(u: f32, v: f32) -> f32 {
    let phase = (u * CRYSTAL_SLANT + v / NOTE_BAND) * std::f32::consts::TAU;
    let broad = phase.sin();
    let fine = (phase * 2.0 + 1.1).sin();
    1.0 + CRYSTAL_DEPTH * (0.68 * broad + 0.32 * fine)
}

/// Pixels across the note texture. Square, because it is cut into a head, a tail and a
/// repeating middle, and the head has to keep the aspect it was drawn at.
pub const NOTE_TEXTURE_SIZE: u32 = 128;

/// How much of the texture, top and bottom, is the note's cap. What is left between
/// them is the band that repeats.
pub const NOTE_CAP: f32 = 0.3;

/// The height of that band, which is one period of the grain.
pub const NOTE_BAND: f32 = 1.0 - 2.0 * NOTE_CAP;

/// How bright the inside of a note is, against a rim of 1.
///
/// The material multiplies this by a colour taken nine times past full scale, so the
/// rim — left at one — blows out to white and the bloom pass finds it. The inside has
/// to land clearly below that or it clips in its own strong channels, comes out the
/// same white as the rim, and the note stops being a coloured slab with an outline and
/// becomes a light tube. The rim is the whole shape of the reference's notes and needs
/// something to stand against.
///
/// This number is a level in an sRGB texture, not a linear one, which is a factor of
/// about eight and worth saying out loud: a stored 0.26 arrives at the shader as 0.055.
const NOTE_BODY: f32 = 0.26;

/// How far the grain leans over as it crosses the note, and how strongly it shows.
const CRYSTAL_SLANT: f32 = 0.45;
const CRYSTAL_DEPTH: f32 = 0.38;

/// How far the note's edge is softened, in texture units across its width.
const EDGE_FEATHER: f32 = 0.035;

/// Width of the brighter rim just inside a note's edge.
const RIM_WIDTH: f32 = 0.11;

/// The felt strip behind the keys, which gives the keyboard a top edge to sit
/// against and hides where the keys meet the lane.
pub fn back_rail() -> Image {
    let felt: Rgba = [104, 24, 34, 255];
    let dark: Rgba = [26, 6, 9, 255];
    generate(move |_, v| blend(dark, felt, v.powf(0.55)))
}

/// The band of light standing above the back of the keyboard.
///
/// Separate from the line itself, and it has to be, because the two are different
/// colours. Measured up from the keys the reference is pure blue — red and green sit at
/// nothing for twenty millimetres while blue climbs from three to seventy-four — and
/// only at the line does it go cyan-white. That cannot come from blooming a cyan line,
/// which spreads cyan; it is its own thing standing behind the keyboard.
///
/// Brightest against the keys and gone within a couple of centimetres.
pub fn back_glow() -> Image {
    generate(|_, v| {
        // v runs from the top of the band down to the keys, so the light is at v = 1.
        let level = v.powf(4.5);
        [255, 255, 255, (level * 255.0) as u8]
    })
}

/// The strip of light along the line where the notes meet the keys.
///
/// Every falling-note display has one. It tells the eye exactly where "now" is, which
/// a keyboard alone does not — and it is the brightest thing in the picture, so it is
/// drawn as a hard core with a wide soft falloff either side rather than a plain bar.
/// The core is what reads as a line; the falloff is what the bloom pass turns into a
/// band of light lying across the whole instrument.
pub fn hit_line() -> Image {
    generate(|_, v| {
        let across = (1.0 - (v - 0.5).abs() * 2.0).clamp(0.0, 1.0);
        let core = across.powf(9.0);
        let halo = across.powf(1.6);
        let level = (core + halo * 0.42).min(1.0);
        [255, 255, 255, (level * 255.0) as u8]
    })
}

/// The instrument's front, below the keys — where the hands come in from.
///
/// Dark, but not black: a piano's fallboard and cheek blocks are lacquered, and from
/// above they carry a soft sheen that falls off toward the player. Black would leave
/// the hands floating in a void; this gives them something to sit on.
pub fn case() -> Image {
    let near: Rgba = [0, 0, 0, 255];
    let far: Rgba = [11, 11, 14, 255];
    generate(move |_, v| {
        // v runs from the player's edge to the keys, so the sheen gathers under the
        // keyboard where the light catches the lacquer.
        let sheen = v.powf(2.4);
        blend(near, far, sheen)
    })
}

/// The vertical line that marks an octave in the note lane.
///
/// Piano music is read in octaves, and a bare lane gives the eye nothing to measure a
/// leap against. One faint line at every C is enough to place a note without becoming
/// a grid: it has no hard edge at all, just a narrow core fading out either side, so it
/// sits behind the music rather than in front of it.
pub fn octave_line() -> Image {
    generate(|u, _| {
        let across = (1.0 - (u - 0.5).abs() * 2.0).clamp(0.0, 1.0);
        let core = across.powf(24.0);
        let halo = across.powf(3.0);
        [255, 255, 255, ((core * 0.55 + halo * 0.16) * 255.0) as u8]
    })
}

/// The burst of light at a key that is sounding.
///
/// A soft white core sitting on the key, and nothing else. There was a star here, with
/// rays running out of it the way a bright point looks through a lens — and the
/// reference has no such thing. What it has is a blob: very bright in the middle, wider
/// than it is tall, fading out with no edge anywhere. The rays read as drawn, which is
/// the one thing this must not.
pub fn flash() -> Image {
    generate(|u, v| {
        // Wider than tall. The light is coming off a key, which is a wide flat thing.
        let (dx, dy) = ((u - 0.5) * 2.0, (v - 0.5) * 2.6);
        let radius = (dx * dx + dy * dy).sqrt();
        let falloff = (1.0 - radius).clamp(0.0, 1.0);

        // Two widths of the same falloff: a small white-hot middle and a wider body
        // around it. Adding them rather than choosing between them gives a core with no
        // edge where it stops being a core.
        //
        // There was a third, a broad low haze, and it had to go. Against a black
        // background a few levels of alpha spread over a hundred millimetres is not a
        // haze, it is a visible grey oval lying over the keys.
        let core = falloff.powf(7.0);
        let body = falloff.powf(2.4);

        let level = (core + body * 0.34).clamp(0.0, 1.0);
        [255, 255, 255, (level * 255.0) as u8]
    })
}

/// A single mote of the dust a sounding key throws up.
///
/// Small and hard-edged, with only a breath of a halo. These are drawn about a
/// millimetre across and there are thousands of them, and at that size a soft blob is
/// just a dim smudge — what reads as dust is a crisp point of light. The halo is there
/// only to keep the smallest ones from flickering as they cross a pixel boundary.
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

    /// Read a pixel back out of a generated image, at its own size.
    ///
    /// Not every image is the same shape: the note is square, because it is sliced
    /// into a head, a tail and a stretched middle and its ends have to keep the aspect
    /// they were drawn at.
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
        // Bright, but not white. The ceiling used to be lower, to stop the keyboard
        // hazing over the whole frame — but that haze was the bloom pass being computed
        // on a thumbnail and upsampled, not the keys being too pale, and darkening the
        // texture was treating a symptom. With the bloom fixed the keys can carry the
        // brightness real ivory has. The ceiling stays only to keep them off pure white,
        // which would lose the seams and the shadows the black keys cast.
        assert!(
            (150.0..238.0).contains(&middle),
            "a white key should read as a light grey: {middle}"
        );
        // Darker than the key, and warmer than it. The warmth is the point: the gap
        // between two white keys shows the wood behind them, and a neutral dark line
        // there is what makes a keyboard read as a barcode. A tan seam is necessarily
        // less dark than a black one, so the ratio here is looser than it was and the
        // hue check carries the rest of the meaning.
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
        // The corner is cut away, so a point just inside the bounding box but outside
        // the rounded corner reads as outside.
        assert!(rounded_box_distance((0.49, 1.99), half, 0.25) > 0.0);
    }

    #[test]
    fn a_note_is_a_rounded_rectangle_rather_than_a_capsule() {
        let image = note_bar();
        // Solid down the middle and out to the sides at mid-height: a capsule would
        // have curved away by now.
        assert_eq!(pixel(&image, 0.5, 0.5)[3], 255);
        assert!(pixel(&image, 0.90, 0.5)[3] > 200, "the sides should still be square");
        // The corners are cut.
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
        // Sample across the left edge; the alpha should pass through a middle value
        // rather than jumping from nothing to everything.
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
