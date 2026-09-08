//! Physical geometry of a standard 88-key piano keyboard, in millimetres.
//!
//! Every distance in this crate is a real millimetre measurement rather than a
//! semitone count. That matters: a minor third from E to G spans 47 mm while a
//! minor third from F# to A spans 38.5 mm, and the black keys sit 10 mm higher
//! and 55 mm further from the player. A fingering model that counts semitones
//! cannot see any of that.
//!
//! # Coordinate frame
//!
//! Right-handed, origin at the front-left corner of the lowest key (A0):
//!
//! * `+x` runs along the keyboard toward higher pitches
//! * `+y` runs away from the player, into the keyboard
//! * `+z` runs up, `z = 0` being the top surface of an unpressed white key
//!
//! The visualizer looks straight down `-z`, so `x`/`y` map directly to screen.

use glam::Vec3;

/// Lowest key of an 88-key piano (A0).
pub const MIDI_LOWEST: u8 = 21;
/// Highest key of an 88-key piano (C8).
pub const MIDI_HIGHEST: u8 = 108;
/// Number of keys on a standard piano.
pub const KEY_COUNT: usize = (MIDI_HIGHEST - MIDI_LOWEST + 1) as usize;

/// Centre-line distance in millimetres from each pitch class to the semitone above it.
///
/// The values are asymmetric on purpose: C# sits 9.5 mm right of C but D# sits 14 mm
/// right of D, which is what pushes the black keys of the two- and three-key groups
/// outward and leaves usable white "necks" between them. Taken from the physical
/// ruler in pydactyl (David A. Randolph, MIT), re-indexed by pitch class.
///
/// Two neighbouring white keys, however, are always exactly [`WHITE_KEY_WIDTH`] apart,
/// whatever sits between them — that is what it means for them to be the same width and
/// to touch. Pydactyl's numbers sum to 164.0 per octave against the 164.5 that seven
/// keys of 23.5 mm need, and put the whole half-millimetre in one place: G to A came
/// out at 23.0, so every A overlapped the G below it by half a millimetre and the
/// drawn keyboard had a seam in each octave that a real one does not. The half
/// millimetre is split between the two steps either side of G#, which leaves G# exactly
/// midway between its neighbours, where it was before and where it belongs.
const SEMITONE_STEP_MM: [f32; 12] = [
    9.5,  // C  -> C#
    14.0, // C# -> D
    14.0, // D  -> D#
    9.5,  // D# -> E
    23.5, // E  -> F
    8.0,  // F  -> F#
    15.5, // F# -> G
    11.75, // G  -> G#
    11.75, // G# -> A
    15.5, // A  -> A#
    8.0,  // A# -> B
    23.5, // B  -> C
];

/// Width of a white key at the front, where it is unobstructed.
pub const WHITE_KEY_WIDTH: f32 = 23.5;
/// Width of a black key.
///
/// A real black key is about 13.7 mm across its base and tapers slightly toward the
/// top. The base width is what determines how much of the white key beside it is
/// left reachable, which is what this is used for.
pub const BLACK_KEY_WIDTH: f32 = 13.7;
/// Visible length of a white key from front edge to fallboard.
pub const WHITE_KEY_LENGTH: f32 = 150.0;
/// Visible length of a black key.
pub const BLACK_KEY_LENGTH: f32 = 95.0;
/// How far back the front edge of the black keys sits.
pub const BLACK_KEY_FRONT_Y: f32 = WHITE_KEY_LENGTH - BLACK_KEY_LENGTH;
/// Height of a black key playing surface above the white key surface.
pub const BLACK_KEY_HEIGHT: f32 = 10.0;
/// How far the front of a key travels when fully depressed.
pub const KEY_DIP: f32 = 10.0;

/// Whether a pitch class is a black key.
const IS_BLACK_PC: [bool; 12] = [
    false, true, false, true, false, false, true, false, true, false, true, false,
];

/// True if this MIDI pitch is a black key.
#[inline]
pub fn is_black(midi: u8) -> bool {
    IS_BLACK_PC[(midi % 12) as usize]
}

/// True if this MIDI pitch is a white key.
#[inline]
pub fn is_white(midi: u8) -> bool {
    !is_black(midi)
}

/// Where on a key the fingertip lands.
///
/// White keys have two genuinely different playing positions and pianists choose
/// between them constantly: out at the front where the key is a full 23.5 mm wide,
/// or back between the black keys where it narrows to a neck. Playing further in
/// costs forearm extension but shortens the reach to any black key in the same chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StrikeStyle {
    /// Front of a white key, clear of the black keys.
    WhiteFront,
    /// Back of a white key, in the neck between adjacent black keys.
    WhiteNeck,
    /// Top of a black key.
    Black,
}

impl StrikeStyle {
    /// The styles physically available for a given key.
    pub fn available(midi: u8) -> &'static [StrikeStyle] {
        if is_black(midi) {
            &[StrikeStyle::Black]
        } else {
            &[StrikeStyle::WhiteFront, StrikeStyle::WhiteNeck]
        }
    }
}

/// `y` at which a black key is struck: back enough to clear its front lip,
/// forward enough to leave room for the white keys behind it.
const BLACK_STRIKE_Y: f32 = BLACK_KEY_FRONT_Y + 20.0; // 75 mm
/// `y` for a white key played at the front.
///
/// Not at the very front edge. A white key is 150 mm long with the black keys
/// starting 55 mm in, so the free part in front of them is 0–55 mm — but a pianist
/// does not play at the lip of it. RoboPianist, which had to make a robot hand
/// actually depress keys, aims about 49 mm from the front edge; that is close to the
/// far end of the free part and is where a curved finger naturally lands with the
/// hand held over the keys rather than hanging off them.
const WHITE_FRONT_STRIKE_Y: f32 = 45.0;
/// `y` for a white key played back between the black keys.
const WHITE_NECK_STRIKE_Y: f32 = BLACK_STRIKE_Y;

/// A precomputed 88-key keyboard.
#[derive(Debug, Clone)]
pub struct Keyboard {
    /// Centre-line `x` of every key, indexed by `midi - MIDI_LOWEST`.
    centre_x: [f32; KEY_COUNT],
    /// For white keys, the `x` range still free at `WHITE_NECK_STRIKE_Y`.
    neck: [(f32, f32); KEY_COUNT],
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Keyboard {
    /// Build the keyboard, precomputing key centres and white-key neck extents.
    pub fn new() -> Self {
        let mut centre_x = [0.0f32; KEY_COUNT];
        let mut x = WHITE_KEY_WIDTH / 2.0; // A0 centre line
        for (i, slot) in centre_x.iter_mut().enumerate() {
            *slot = x;
            let pc = (MIDI_LOWEST as usize + i) % 12;
            x += SEMITONE_STEP_MM[pc];
        }

        // A white key neck is bounded by whichever black keys flank it.
        let mut neck = [(0.0f32, 0.0f32); KEY_COUNT];
        for i in 0..KEY_COUNT {
            let midi = MIDI_LOWEST + i as u8;
            if is_black(midi) {
                neck[i] = (
                    centre_x[i] - BLACK_KEY_WIDTH / 2.0,
                    centre_x[i] + BLACK_KEY_WIDTH / 2.0,
                );
                continue;
            }
            let mut lo = centre_x[i] - WHITE_KEY_WIDTH / 2.0;
            let mut hi = centre_x[i] + WHITE_KEY_WIDTH / 2.0;
            if midi > MIDI_LOWEST && is_black(midi - 1) {
                lo = centre_x[i - 1] + BLACK_KEY_WIDTH / 2.0;
            }
            if midi < MIDI_HIGHEST && is_black(midi + 1) {
                hi = centre_x[i + 1] - BLACK_KEY_WIDTH / 2.0;
            }
            neck[i] = (lo, hi);
        }

        Self { centre_x, neck }
    }

    #[inline]
    fn index(midi: u8) -> usize {
        debug_assert!(
            (MIDI_LOWEST..=MIDI_HIGHEST).contains(&midi),
            "midi {midi} is off the keyboard"
        );
        (midi.clamp(MIDI_LOWEST, MIDI_HIGHEST) - MIDI_LOWEST) as usize
    }

    /// Centre-line `x` of a key, in millimetres from the left edge of the keyboard.
    #[inline]
    pub fn centre_x(&self, midi: u8) -> f32 {
        self.centre_x[Self::index(midi)]
    }

    /// Signed horizontal distance in millimetres from one key to another.
    #[inline]
    pub fn interval_mm(&self, from: u8, to: u8) -> f32 {
        self.centre_x(to) - self.centre_x(from)
    }

    /// The `x` range of a white key neck, i.e. the part still reachable when the
    /// hand is far enough in to be among the black keys.
    #[inline]
    pub fn neck_span(&self, midi: u8) -> (f32, f32) {
        self.neck[Self::index(midi)]
    }

    /// Total width of the keyboard.
    pub fn width(&self) -> f32 {
        self.centre_x[KEY_COUNT - 1] + WHITE_KEY_WIDTH / 2.0
    }

    /// The point a fingertip must reach to sound this key in the given style.
    pub fn strike_point(&self, midi: u8, style: StrikeStyle) -> Vec3 {
        let cx = self.centre_x(midi);
        match style {
            StrikeStyle::Black => Vec3::new(cx, BLACK_STRIKE_Y, BLACK_KEY_HEIGHT),
            StrikeStyle::WhiteFront => Vec3::new(cx, WHITE_FRONT_STRIKE_Y, 0.0),
            StrikeStyle::WhiteNeck => {
                let (lo, hi) = self.neck_span(midi);
                Vec3::new((lo + hi) / 2.0, WHITE_NECK_STRIKE_Y, 0.0)
            }
        }
    }

    /// The strike point a lone finger would naturally use for this key.
    pub fn nominal_strike(&self, midi: u8) -> Vec3 {
        if is_black(midi) {
            self.strike_point(midi, StrikeStyle::Black)
        } else {
            self.strike_point(midi, StrikeStyle::WhiteFront)
        }
    }

    /// Rectangular footprint of a key on the `xy` plane: `(x_min, x_max, y_min, y_max)`.
    pub fn footprint(&self, midi: u8) -> (f32, f32, f32, f32) {
        let cx = self.centre_x(midi);
        if is_black(midi) {
            (
                cx - BLACK_KEY_WIDTH / 2.0,
                cx + BLACK_KEY_WIDTH / 2.0,
                BLACK_KEY_FRONT_Y,
                WHITE_KEY_LENGTH,
            )
        } else {
            (
                cx - WHITE_KEY_WIDTH / 2.0,
                cx + WHITE_KEY_WIDTH / 2.0,
                0.0,
                WHITE_KEY_LENGTH,
            )
        }
    }

    /// Every key on the instrument, low to high.
    pub fn keys(&self) -> impl Iterator<Item = u8> {
        MIDI_LOWEST..=MIDI_HIGHEST
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kb() -> Keyboard {
        Keyboard::new()
    }

    #[test]
    fn an_octave_is_seven_white_keys_wide_wherever_it_starts() {
        // Stated as seven key widths rather than as a number, because that is what an
        // octave is. Written down separately the two drifted apart by half a
        // millimetre, and the whole of the difference landed between G and A, so every
        // A overlapped the G below it.
        let kb = kb();
        let octave = 7.0 * WHITE_KEY_WIDTH;
        for midi in MIDI_LOWEST..=(MIDI_HIGHEST - 12) {
            let span = kb.interval_mm(midi, midi + 12);
            assert!((span - octave).abs() < 1e-3, "octave at {midi} was {span} mm");
        }
    }

    #[test]
    fn two_white_keys_side_by_side_are_one_key_apart() {
        // Whatever sits between them. This is what it means for the keys to be the same
        // width and to touch, and it is the property the octave span has to respect.
        let kb = kb();
        let mut previous: Option<(u8, f32)> = None;
        for midi in MIDI_LOWEST..=MIDI_HIGHEST {
            if is_black(midi) {
                continue;
            }
            let x = kb.centre_x(midi);
            if let Some((was, was_x)) = previous {
                let step = x - was_x;
                assert!(
                    (step - WHITE_KEY_WIDTH).abs() < 1e-3,
                    "{was} to {midi} was {step} mm"
                );
            }
            previous = Some((midi, x));
        }
    }

    #[test]
    fn adjacent_white_keys_are_one_key_width_apart() {
        let kb = kb();
        // Every white-to-white step is 23.5 mm except G to A, which the standard
        // layout squeezes to 23.0 to keep G# exactly centred between them.
        for midi in MIDI_LOWEST..MIDI_HIGHEST {
            if !is_white(midi) {
                continue;
            }
            let Some(next) = (midi + 1..=MIDI_HIGHEST).find(|m| is_white(*m)) else {
                continue;
            };
            let d = kb.interval_mm(midi, next);
            assert!(
                (23.0..=23.5).contains(&d),
                "white step {midi} to {next} was {d} mm"
            );
        }
    }

    #[test]
    fn seven_white_keys_make_an_octave() {
        let kb = kb();
        let whites = (60..72).filter(|m| is_white(*m)).count();
        assert_eq!(whites, 7);
        assert!((kb.interval_mm(60, 72) - whites as f32 * WHITE_KEY_WIDTH).abs() < 1e-3);
    }

    #[test]
    fn black_keys_sit_where_a_real_piano_puts_them() {
        let kb = kb();
        // C# is pulled left of the C-D midpoint and D# right of the D-E midpoint
        // by the same amount. That asymmetry is what makes the three-key group work.
        let cs_off = kb.centre_x(61) - (kb.centre_x(60) + kb.centre_x(62)) / 2.0;
        let ds_off = kb.centre_x(63) - (kb.centre_x(62) + kb.centre_x(64)) / 2.0;
        assert!((cs_off + 2.25).abs() < 1e-3, "C# offset {cs_off}");
        assert!((ds_off - 2.25).abs() < 1e-3, "D# offset {ds_off}");

        // G# is centred exactly between G and A.
        let gs_off = kb.centre_x(68) - (kb.centre_x(67) + kb.centre_x(69)) / 2.0;
        assert!(gs_off.abs() < 1e-3, "G# offset {gs_off}");
    }

    #[test]
    fn white_necks_narrow_by_different_amounts() {
        let kb = kb();
        let neck = |midi: u8| {
            let (lo, hi) = kb.neck_span(midi);
            hi - lo
        };

        // Every white key loses width once the hand is far enough in to be among
        // the black keys.
        for midi in 60..=71 {
            if is_white(midi) {
                assert!(neck(midi) < WHITE_KEY_WIDTH, "midi {midi} neck too wide");
            }
        }

        // How much each loses is not uniform, because the black keys of each group
        // are pushed outward. F and B end up with the narrowest necks on the
        // keyboard, which is why they are the easiest white keys to miss when
        // playing back among the black keys.
        let (c, d, e) = (neck(60), neck(62), neck(64));
        let (f, g, a, b) = (neck(65), neck(67), neck(69), neck(71));
        for (name, width) in [("C", c), ("D", d), ("E", e), ("F", f), ("G", g), ("A", a), ("B", b)]
        {
            assert!(
                (12.0..15.0).contains(&width),
                "the {name} neck is {width} mm, which is not a key width any hand has met"
            );
        }
        assert!((f - b).abs() < 0.01, "F and B are symmetric: {f} and {b}");
        assert!((c - e).abs() < 0.01, "C and E are symmetric: {c} and {e}");
        assert!((g - a).abs() < 0.01, "G and A are symmetric: {g} and {a}");
        assert!(f < g, "F should be the tighter of the two-black-key group");
        assert!(g < c, "and the three-key group leaves more room than the two-key one");
    }

    #[test]
    fn thirds_are_not_all_the_same_width() {
        let kb = kb();
        // The whole point of using millimetres: E-G and F#-A are both minor thirds.
        let e_g = kb.interval_mm(64, 67);
        let fs_a = kb.interval_mm(66, 69);
        assert!((e_g - 47.0).abs() < 1e-3, "E-G was {e_g}");
        assert!((fs_a - 39.0).abs() < 1e-3, "F#-A was {fs_a}");
    }

    #[test]
    fn keyboard_is_about_1220_mm_wide() {
        let kb = kb();
        let w = kb.width();
        assert!((1200.0..1240.0).contains(&w), "keyboard width was {w} mm");
    }

    #[test]
    fn strike_points_land_on_their_keys() {
        let kb = kb();
        for midi in kb.keys() {
            for style in StrikeStyle::available(midi) {
                let p = kb.strike_point(midi, *style);
                let (x0, x1, y0, y1) = kb.footprint(midi);
                assert!(
                    p.x >= x0 - 0.01 && p.x <= x1 + 0.01,
                    "{midi} {style:?} x={} outside {x0}..{x1}",
                    p.x
                );
                assert!(
                    p.y >= y0 - 0.01 && p.y <= y1 + 0.01,
                    "{midi} {style:?} y={} outside {y0}..{y1}",
                    p.y
                );
            }
        }
    }
}
