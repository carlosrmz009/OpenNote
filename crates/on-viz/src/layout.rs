//! Where everything sits on screen.
//!
//! The world is measured in millimetres of real piano, and the camera looks straight
//! down at it. That single decision does most of the work: the keyboard is drawn from
//! the same [`on_hand::keyboard::Keyboard`] the fingering engine reasons about, and
//! the 3D hands are posed in the same coordinates, so their sizes agree with each
//! other without anyone having to match them up.
//!
//! ```text
//!            ^ y (mm)
//!            |
//!   +--------------------+  <- notes appear here and fall
//!   |     falling notes  |
//!   |                    |
//!   +--------------------+  <- y = 0, the front edge of the keys
//!   |     the keyboard   |     (drawn below, extending to negative y)
//!   +--------------------+
//! ```
//!
//! The keyboard's own frame runs `+y` *into* the instrument, away from the player.
//! On screen the player is at the bottom, so the keyboard occupies the lower band and
//! the note lane rises above it.

use on_hand::keyboard::{
    Keyboard, BLACK_KEY_LENGTH, MIDI_HIGHEST, MIDI_LOWEST, WHITE_KEY_LENGTH,
};

/// How much of the keyboard to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Extent {
    /// All 88 keys, whether or not the music uses them.
    ///
    /// The default: a display that quietly crops to the keys a piece happens to use
    /// stops looking like a piano.
    #[default]
    FullKeyboard,
    /// Only the range the piece actually touches, with a little margin.
    MusicRange,
}

/// How the view is arranged.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Keyboard geometry, in millimetres.
    pub keyboard: Keyboard,
    /// Lowest key shown.
    pub lowest: u8,
    /// Highest key shown.
    pub highest: u8,
    /// How far the note lane extends above the keys, in millimetres.
    ///
    /// Derived from the keyboard width rather than fixed, so the composition — how
    /// much of the view the keyboard takes up — stays the same whether the piece
    /// spans three octaves or all eighty-eight keys.
    pub lane_height: f32,
    /// How long the visible future is, in seconds. Notes enter the top of the lane
    /// this far before they sound.
    pub lookahead: f64,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            keyboard: Keyboard::new(),
            lowest: MIDI_LOWEST,
            highest: MIDI_HIGHEST,
            lane_height: 0.0,
            lookahead: 3.0,
        }
        .with_derived_lane()
    }
}

/// Height of the note lane as a fraction of the keyboard's width.
///
/// Derived from the width rather than fixed, so the composition — how much of the
/// view is keyboard and how much is lane — stays the same whether the piece spans
/// three octaves or all eighty-eight keys. Cropping to a narrow range then genuinely
/// zooms in, instead of drawing a small keyboard in the middle of the same picture.
///
/// The value is set so that a full eighty-eight keys, the hands below them and the
/// lane above fill a sixteen-by-nine frame exactly.
const LANE_TO_WIDTH: f32 = 0.31;

/// The narrowest range ever shown, in semitones.
///
/// A piece using six notes should not be magnified until the keys are the size of
/// dinner plates.
const MIN_VISIBLE_SPAN: u8 = 48;

impl Layout {
    /// Build a layout showing the range a piece uses, with a few keys of margin so
    /// the outermost notes are not flush against the edge.
    pub fn for_pitches(pitches: impl IntoIterator<Item = u8>, extent: Extent) -> Self {
        let mut layout = Layout::default();
        if extent == Extent::FullKeyboard {
            return layout;
        }
        let mut low = MIDI_HIGHEST;
        let mut high = MIDI_LOWEST;
        let mut any = false;
        for pitch in pitches {
            low = low.min(pitch);
            high = high.max(pitch);
            any = true;
        }
        if !any {
            return layout;
        }
        // A little margin either side, then widened to the minimum span so a narrow
        // piece is not magnified out of all proportion.
        let mut lowest = low.saturating_sub(4).max(MIDI_LOWEST);
        let mut highest = high.saturating_add(4).min(MIDI_HIGHEST);
        while highest - lowest < MIN_VISIBLE_SPAN {
            if lowest > MIDI_LOWEST {
                lowest -= 1;
            }
            if highest < MIDI_HIGHEST {
                highest += 1;
            }
            if lowest == MIDI_LOWEST && highest == MIDI_HIGHEST {
                break;
            }
        }
        layout.lowest = lowest;
        layout.highest = highest;
        layout.with_derived_lane()
    }

    /// Size the note lane to the keyboard, so the view is composed the same way at
    /// any zoom.
    fn with_derived_lane(mut self) -> Self {
        self.lane_height = self.width_mm() * LANE_TO_WIDTH;
        self
    }

    /// Left edge of the visible keyboard, in millimetres.
    pub fn left_mm(&self) -> f32 {
        self.keyboard.footprint(self.lowest).0
    }

    /// Right edge of the visible keyboard, in millimetres.
    pub fn right_mm(&self) -> f32 {
        self.keyboard.footprint(self.highest).1
    }

    /// Width of the visible keyboard, in millimetres.
    pub fn width_mm(&self) -> f32 {
        self.right_mm() - self.left_mm()
    }

    /// Total height of the view, keyboard plus lane plus the margin below, in
    /// millimetres.
    pub fn height_mm(&self) -> f32 {
        WHITE_KEY_LENGTH + BOTTOM_MARGIN_MM + self.lane_height
    }

    /// The bottom edge of the view, in millimetres. Below the keys, with room for the
    /// hands and their wrists.
    pub fn bottom_mm(&self) -> f32 {
        -WHITE_KEY_LENGTH - BOTTOM_MARGIN_MM
    }

    /// Centre of the view in world millimetres, for the camera to sit over.
    ///
    /// The view spans from the back of the keyboard, at `-WHITE_KEY_LENGTH`, up to
    /// the top of the note lane.
    pub fn centre_mm(&self) -> (f32, f32) {
        // Shifted up a little, which leaves a margin under the keyboard rather than
        // running the keys off the bottom edge of the window.
        let bottom = -WHITE_KEY_LENGTH - BOTTOM_MARGIN_MM;
        (
            (self.left_mm() + self.right_mm()) / 2.0,
            (self.lane_height + bottom) / 2.0,
        )
    }

    /// Whether a pitch is on screen.
    pub fn shows(&self, midi: u8) -> bool {
        (self.lowest..=self.highest).contains(&midi)
    }

    /// Every visible key, low to high.
    pub fn keys(&self) -> impl Iterator<Item = u8> + '_ {
        self.lowest..=self.highest
    }

    /// The `y` a note sits at in the lane, given how far in the future it sounds.
    ///
    /// Zero is the front edge of the keys, so a note that is sounding right now is at
    /// the keyboard and one that sounds in `lookahead` seconds is at the top.
    pub fn note_y(&self, seconds_away: f64) -> f32 {
        (seconds_away / self.lookahead) as f32 * self.lane_height
    }

    /// How tall a note of a given duration is drawn, in millimetres.
    pub fn note_height(&self, duration_seconds: f64) -> f32 {
        (duration_seconds / self.lookahead) as f32 * self.lane_height
    }

    /// The rectangle a falling note occupies: `(x_min, x_max, y_bottom, y_top)`.
    ///
    /// Notes are drawn over the key they belong to and are as wide as it is, which is
    /// what makes the display readable: a black-key note is a narrow bar sitting
    /// between two wider ones.
    pub fn note_rect(
        &self,
        midi: u8,
        start_seconds_away: f64,
        duration_seconds: f64,
    ) -> (f32, f32, f32, f32) {
        let (x0, x1, _, _) = self.keyboard.footprint(midi);
        let bottom = self.note_y(start_seconds_away);
        let top = bottom + self.note_height(duration_seconds).max(MIN_NOTE_HEIGHT_MM);
        (x0, x1, bottom, top)
    }

    /// The rectangle a key occupies on screen: `(x_min, x_max, y_min, y_max)`.
    ///
    /// The keyboard sits below the lane, running from `-WHITE_KEY_LENGTH` up to zero.
    /// It keeps the instrument's own orientation rather than being flipped: the far
    /// end of the keys, where the black keys are, is the end nearest the lane, which
    /// is where the notes arrive. Flipping it would put the black keys at the bottom
    /// and the display would read backwards.
    pub fn key_rect(&self, midi: u8) -> (f32, f32, f32, f32) {
        let (x0, x1, y0, y1) = self.keyboard.footprint(midi);
        (x0, x1, y0 - WHITE_KEY_LENGTH, y1 - WHITE_KEY_LENGTH)
    }

    /// How far up the key a black key starts, for drawing it over the white ones.
    pub fn black_key_depth_mm(&self) -> f32 {
        BLACK_KEY_LENGTH
    }
}

/// The shortest a note is ever drawn, so a grace note is still visible.
const MIN_NOTE_HEIGHT_MM: f32 = 6.0;

/// Space left below the keyboard, in millimetres.
///
/// Not decoration: this is where the hands are. A fingertip plays a white key three
/// to five centimetres from its front edge, and the wrist is a hand's length in front
/// of that, so without room here the palms fall off the bottom of the picture and all
/// that shows of a hand is its fingertips.
const BOTTOM_MARGIN_MM: f32 = 165.0;

#[cfg(test)]
mod tests {
    use super::*;
    use on_hand::keyboard::is_black;

    #[test]
    fn the_full_keyboard_is_about_a_metre_and_a_quarter_wide() {
        let layout = Layout::default();
        let width = layout.width_mm();
        assert!((1200.0..1240.0).contains(&width), "{width}");
    }

    #[test]
    fn a_narrow_piece_gets_a_narrower_view_than_the_whole_keyboard() {
        let layout = Layout::for_pitches([60u8, 62, 64, 65, 67], Extent::MusicRange);
        assert!(layout.lowest < 60 && layout.highest > 67);
        assert!(layout.shows(60) && layout.shows(67));
        assert!(
            layout.width_mm() < Layout::default().width_mm() * 0.75,
            "{}",
            layout.width_mm()
        );
    }

    #[test]
    fn asking_for_the_whole_keyboard_gets_the_whole_keyboard() {
        let layout = Layout::for_pitches([60u8], Extent::FullKeyboard);
        assert_eq!(layout.lowest, MIDI_LOWEST);
        assert_eq!(layout.highest, MIDI_HIGHEST);
    }

    #[test]
    fn a_note_sounding_now_is_at_the_keyboard_and_a_future_one_is_above() {
        let layout = Layout::default();
        assert_eq!(layout.note_y(0.0), 0.0);
        assert!(layout.note_y(1.0) > 0.0);
        assert!((layout.note_y(layout.lookahead) - layout.lane_height).abs() < 1e-3);
    }

    #[test]
    fn notes_are_drawn_over_the_key_they_belong_to() {
        // Every key, not a handful: a note that does not line up with its key is the
        // first thing anyone notices, and it is not something to find out by looking at
        // a render.
        let layout = Layout::default();
        for midi in 21..=108u8 {
            let (x0, x1, _, _) = layout.note_rect(midi, 0.5, 0.5);
            let (k0, k1, _, _) = layout.key_rect(midi);
            assert_eq!((x0, x1), (k0, k1), "note {midi} is not over its key");
        }
    }

    #[test]
    fn black_key_notes_are_narrower_than_white_key_notes() {
        let layout = Layout::default();
        let width = |midi: u8| {
            let (x0, x1, _, _) = layout.note_rect(midi, 0.0, 1.0);
            x1 - x0
        };
        assert!(width(61) < width(60), "a C sharp bar should be narrower than a C");
        for midi in 60..72u8 {
            if is_black(midi) {
                assert!(width(midi) < 16.0, "black bar at {midi} is {}", width(midi));
            } else {
                assert!(width(midi) > 20.0, "white bar at {midi} is {}", width(midi));
            }
        }
    }

    #[test]
    fn even_the_shortest_note_is_visible() {
        let layout = Layout::default();
        let (_, _, bottom, top) = layout.note_rect(60, 0.0, 0.0);
        assert!(top - bottom >= MIN_NOTE_HEIGHT_MM);
    }

    #[test]
    fn the_view_is_centred_between_the_keys_and_the_top_of_the_lane() {
        let layout = Layout::default();
        let (_, cy) = layout.centre_mm();
        let (_, _, key_bottom, _) = layout.key_rect(60);
        assert!(cy > key_bottom, "the view centre should be above the keys");
        assert!(cy < layout.lane_height, "and below the top of the lane");
        // Halfway between the top of the lane and the margin below the keys.
        let bottom = key_bottom - BOTTOM_MARGIN_MM;
        assert!((cy - (layout.lane_height + bottom) / 2.0).abs() < 1e-3, "centre {cy}");
    }

    #[test]
    fn the_keyboard_is_drawn_below_the_lane() {
        let layout = Layout::default();
        let (_, _, y0, y1) = layout.key_rect(60);
        assert!(y1 <= 0.0, "the keys should sit below the lane, got {y0}..{y1}");
        assert!((y1 - y0 - WHITE_KEY_LENGTH).abs() < 1e-3);
    }

    #[test]
    fn the_black_keys_are_the_end_nearest_the_lane() {
        // The notes fall onto the far end of the keys, which is where the black keys
        // are. If this is the wrong way round the whole display reads backwards.
        let layout = Layout::default();
        let (_, _, white_bottom, white_top) = layout.key_rect(60);
        let (_, _, black_bottom, black_top) = layout.key_rect(61);
        assert!((black_top - white_top).abs() < 1e-3, "both end at the lane");
        assert!(black_bottom > white_bottom, "the black key is the shorter one");
    }

    // A hand is about 180 mm from wrist to fingertip and plays a few centimetres in
    // from the front edge, so the band below the keys has to be most of that or the
    // palms fall off the picture. Checked once, when the file is compiled, rather than
    // once per case of a test that cannot make it false.
    const _: () = assert!(BOTTOM_MARGIN_MM > 150.0);

    #[test]
    fn there_is_always_room_for_the_keys_the_hands_and_a_readable_lane() {
        for layout in [
            Layout::for_pitches([60u8, 64], Extent::MusicRange),
            Layout::for_pitches([21u8, 108], Extent::MusicRange),
            Layout::for_pitches([60u8], Extent::FullKeyboard),
        ] {
            // And the lane has to be worth looking at: at least as deep as the keys
            // are long, whatever the zoom.
            assert!(
                layout.lane_height > WHITE_KEY_LENGTH,
                "the lane is only {} mm",
                layout.lane_height
            );
        }
    }

    /// A sixteen-by-nine frame showing all eighty-eight keys should be filled by
    /// them: the proportions are chosen so the two ways of fitting the view meet.
    #[test]
    fn a_full_keyboard_fills_a_widescreen_frame() {
        let layout = Layout::default();
        let aspect = layout.width_mm() / layout.height_mm();
        assert!(
            (aspect - 16.0 / 9.0).abs() < 0.08,
            "a full keyboard frames at {aspect}"
        );
    }

    #[test]
    fn a_tiny_piece_still_gets_a_reasonable_range() {
        let layout = Layout::for_pitches([60u8, 62], Extent::MusicRange);
        assert!(
            layout.highest - layout.lowest >= MIN_VISIBLE_SPAN,
            "{}..{}",
            layout.lowest,
            layout.highest
        );
    }

    #[test]
    fn a_longer_note_is_drawn_taller() {
        let layout = Layout::default();
        assert!(layout.note_height(2.0) > layout.note_height(1.0));
        assert!((layout.note_height(layout.lookahead) - layout.lane_height).abs() < 1e-3);
    }

    #[test]
    fn every_key_sits_to_the_right_of_the_one_below_it() {
        // Sorted, non-overlapping among the white keys, and the black keys tucked
        // between them. A keyboard that is out of order would still render, and would
        // still line its notes up with its keys, and would still be wrong.
        let layout = Layout::default();
        let mut previous_white: Option<(f32, f32)> = None;
        for midi in 21..=108u8 {
            let (x0, x1, _, _) = layout.key_rect(midi);
            assert!(x0 < x1, "midi {midi} has no width");
            if is_black(midi) {
                continue;
            }
            if let Some((_, was_x1)) = previous_white {
                assert!(
                    (x0 - was_x1).abs() < 0.01,
                    "white key {midi} starts at {x0} where the one below ends at {was_x1}"
                );
            }
            previous_white = Some((x0, x1));
        }
    }
}
