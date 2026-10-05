use glam::Vec3;

pub const MIDI_LOWEST: u8 = 21;
pub const MIDI_HIGHEST: u8 = 108;
pub const KEY_COUNT: usize = (MIDI_HIGHEST - MIDI_LOWEST + 1) as usize;

pub fn name(midi: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", NAMES[usize::from(midi) % 12], i32::from(midi) / 12 - 1)
}

const SEMITONE_STEP_MM: [f32; 12] = [
    9.5,
    14.0,
    14.0,
    9.5,
    23.5,
    8.0,
    15.5,
    11.75,
    11.75,
    15.5,
    8.0,
    23.5,
];

pub const WHITE_KEY_WIDTH: f32 = 23.5;
pub const BLACK_KEY_WIDTH: f32 = 13.7;
pub const WHITE_KEY_LENGTH: f32 = 150.0;
pub const BLACK_KEY_LENGTH: f32 = 95.0;
pub const BLACK_KEY_FRONT_Y: f32 = WHITE_KEY_LENGTH - BLACK_KEY_LENGTH;
pub const BLACK_KEY_HEIGHT: f32 = 10.0;
pub const KEY_DIP: f32 = 10.0;

const IS_BLACK_PC: [bool; 12] = [
    false, true, false, true, false, false, true, false, true, false, true, false,
];

#[inline]
pub fn is_black(midi: u8) -> bool {
    IS_BLACK_PC[(midi % 12) as usize]
}

#[inline]
pub fn is_white(midi: u8) -> bool {
    !is_black(midi)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StrikeStyle {
    WhiteFront,
    WhiteNeck,
    Black,
}

impl StrikeStyle {
    pub fn available(midi: u8) -> &'static [StrikeStyle] {
        if is_black(midi) {
            &[StrikeStyle::Black]
        } else {
            &[StrikeStyle::WhiteFront, StrikeStyle::WhiteNeck]
        }
    }
}

const BLACK_STRIKE_Y: f32 = BLACK_KEY_FRONT_Y + 20.0;
const WHITE_FRONT_STRIKE_Y: f32 = 45.0;
const WHITE_NECK_STRIKE_Y: f32 = BLACK_STRIKE_Y;

#[derive(Debug, Clone)]
pub struct Keyboard {
    centre_x: [f32; KEY_COUNT],
    neck: [(f32, f32); KEY_COUNT],
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Keyboard {
    pub fn new() -> Self {
        let mut centre_x = [0.0f32; KEY_COUNT];
        let mut x = WHITE_KEY_WIDTH / 2.0;
        for (i, slot) in centre_x.iter_mut().enumerate() {
            *slot = x;
            let pc = (MIDI_LOWEST as usize + i) % 12;
            x += SEMITONE_STEP_MM[pc];
        }

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

    #[inline]
    pub fn centre_x(&self, midi: u8) -> f32 {
        self.centre_x[Self::index(midi)]
    }

    #[inline]
    pub fn interval_mm(&self, from: u8, to: u8) -> f32 {
        self.centre_x(to) - self.centre_x(from)
    }

    #[inline]
    pub fn neck_span(&self, midi: u8) -> (f32, f32) {
        self.neck[Self::index(midi)]
    }

    pub fn width(&self) -> f32 {
        self.centre_x[KEY_COUNT - 1] + WHITE_KEY_WIDTH / 2.0
    }

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

    pub fn nominal_strike(&self, midi: u8) -> Vec3 {
        if is_black(midi) {
            self.strike_point(midi, StrikeStyle::Black)
        } else {
            self.strike_point(midi, StrikeStyle::WhiteFront)
        }
    }

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
        let kb = kb();
        let octave = 7.0 * WHITE_KEY_WIDTH;
        for midi in MIDI_LOWEST..=(MIDI_HIGHEST - 12) {
            let span = kb.interval_mm(midi, midi + 12);
            assert!((span - octave).abs() < 1e-3, "octave at {midi} was {span} mm");
        }
    }

    #[test]
    fn two_white_keys_side_by_side_are_one_key_apart() {
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
        let cs_off = kb.centre_x(61) - (kb.centre_x(60) + kb.centre_x(62)) / 2.0;
        let ds_off = kb.centre_x(63) - (kb.centre_x(62) + kb.centre_x(64)) / 2.0;
        assert!((cs_off + 2.25).abs() < 1e-3, "C# offset {cs_off}");
        assert!((ds_off - 2.25).abs() < 1e-3, "D# offset {ds_off}");

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

        for midi in 60..=71 {
            if is_white(midi) {
                assert!(neck(midi) < WHITE_KEY_WIDTH, "midi {midi} neck too wide");
            }
        }

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
