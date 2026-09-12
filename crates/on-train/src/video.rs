//! Learning fingerings by watching a pianist.
//!
//! There is a great deal of piano on the internet filmed from directly overhead, and
//! every one of those videos is a pianist showing you their fingering. This turns that
//! into training data: hand landmarks from the video, note onsets from a MIDI file of
//! the same performance, and the question "which finger was on that key when it went
//! down?" answered frame by frame.
//!
//! The method is PianoMime's (Qian, Urain, Zakka & Peters, 2024), reimplemented here:
//! a homography from the video frame to the keyboard, then a greedy nearest-finger
//! assignment against the notes that are sounding, with continuity across frames so a
//! held note keeps its finger. Their published pipeline was aimed at driving a robot
//! hand; the fingering extraction in the middle of it is exactly what a fingering
//! model wants to learn from.
//!
//! # What is here and what is not
//!
//! Finding hands in a video frame is a neural network's job, and not one worth
//! reimplementing: `tools/watch_pianist.py` runs MediaPipe's hand landmarker and
//! writes what it found. Everything downstream of that — where the keyboard is, which
//! finger is over which key, and what to believe — is here, where it can be tested
//! and where it does not depend on a Python environment.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use on_hand::keyboard::{is_black, Keyboard, MIDI_HIGHEST, MIDI_LOWEST, WHITE_KEY_LENGTH};
use on_hand::{Finger, Hand};
use serde::{Deserialize, Serialize};

use crate::corpus::{FingeredNote, HandLabel, Piece};

/// How far from a key's centre line a fingertip may be and still be credited with
/// playing it, in white-key widths.
///
/// Two is generous — a finger two keys away is not playing this one — but hand
/// tracking is noisy and a stricter bound throws away real data. What stops a wrong
/// finger being credited is that the nearest one wins and each is used once.
const MAX_OFFSET_KEYS: f32 = 2.0;

/// How far in front of a key's front edge a fingertip may be, in millimetres.
///
/// The thumb often tracks slightly off the near edge of the keyboard, so a little
/// slack is needed; much more than this and the hand is not over the keys at all.
const MAX_OVERHANG_MM: f32 = 3.0;

/// How close two frames' key sets have to be in time to count as the same event.
const ONSET_WINDOW_SECONDS: f64 = 0.05;

/// A point in a video frame, in pixels.
pub type Pixel = (f64, f64);

/// A point on the keyboard, in millimetres: `x` along the keys, `y` into the
/// instrument with zero at the players' edge.
pub type Millimetres = (f64, f64);

/// A projective map from the video frame to the keyboard.
///
/// A piano filmed from overhead is a plane seen through a camera, so the two are
/// related by a homography — eight numbers — whatever the lens and however the camera
/// was set up. One calibration serves every video shot from the same position, which
/// in practice means every video on a given channel.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Homography {
    /// Row-major, with `m[2][2]` normalised to one.
    pub m: [[f64; 3]; 3],
}

impl Homography {
    /// Fit a homography to at least four point correspondences.
    ///
    /// The direct linear transform, with Hartley normalisation. Normalising is not
    /// optional: pixel coordinates run to the thousands and millimetres to over a
    /// thousand, and solving in those units directly gives a system whose condition
    /// number loses most of the precision in the answer.
    pub fn fit(correspondences: &[(Pixel, Millimetres)]) -> Result<Self> {
        if correspondences.len() < 4 {
            bail!(
                "a homography needs at least four points, and {} were given",
                correspondences.len()
            );
        }
        let (from, to): (Vec<Pixel>, Vec<Millimetres>) =
            correspondences.iter().copied().unzip();
        let norm_from = normalising(&from);
        let norm_to = normalising(&to);
        // The map is fitted between normalised frames and then unwound, which is what
        // Hartley normalisation means in practice.
        let inverse_to = invert(&norm_to)?;

        // Two rows per correspondence, eight unknowns; h33 is fixed at one.
        let mut ata = [[0.0f64; 8]; 8];
        let mut atb = [0.0f64; 8];
        for (pixel, millimetres) in correspondences {
            let (x, y) = apply(&norm_from, *pixel);
            let (u, v) = apply(&norm_to, *millimetres);
            let rows = [
                ([x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y], u),
                ([0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y], v),
            ];
            for (row, target) in rows {
                for i in 0..8 {
                    atb[i] += row[i] * target;
                    for j in 0..8 {
                        ata[i][j] += row[i] * row[j];
                    }
                }
            }
        }
        let h = solve(ata, atb).context("the calibration points are degenerate")?;
        let normalised = [
            [h[0], h[1], h[2]],
            [h[3], h[4], h[5]],
            [h[6], h[7], 1.0],
        ];
        // Unwind the normalisation: pixels in, normalised, mapped, then taken back
        // out into millimetres.
        let m = multiply(&multiply(&inverse_to, &normalised), &norm_from);
        Ok(Self { m: rescale(m) })
    }

    /// Fit from the four corners of the visible white keys.
    ///
    /// The easiest calibration to actually perform: pause the video, click the four
    /// corners of the keyboard, and say which white keys they belong to. Everything
    /// else — where every other key is — follows from the instrument's geometry,
    /// which is already known to the millimetre.
    ///
    /// Corners are given in the order the eye reads them: far left, far right, near
    /// right, near left. "Far" is the end of the keys away from the player, which in
    /// an overhead shot is the top of the frame.
    pub fn from_keyboard_corners(
        corners: [Pixel; 4],
        lowest_white: u8,
        highest_white: u8,
    ) -> Result<Self> {
        if is_black(lowest_white) || is_black(highest_white) {
            bail!("the corners of a keyboard are white keys; {lowest_white} and {highest_white} are not both white");
        }
        if lowest_white >= highest_white {
            bail!("the lowest visible key must be below the highest");
        }
        let keyboard = Keyboard::new();
        let left = f64::from(keyboard.footprint(lowest_white).0);
        let right = f64::from(keyboard.footprint(highest_white).1);
        let far = f64::from(WHITE_KEY_LENGTH);
        Self::fit(&[
            (corners[0], (left, far)),
            (corners[1], (right, far)),
            (corners[2], (right, 0.0)),
            (corners[3], (left, 0.0)),
        ])
    }

    /// Where a pixel lands on the keyboard.
    pub fn to_keyboard(&self, pixel: Pixel) -> Millimetres {
        apply(&self.m, pixel)
    }

    /// Read a calibration.
    pub fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("{} is not a calibration", path.display()))
    }

    /// Write a calibration.
    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

/// The four corners of a keyboard, as clicked on a video frame.
///
/// What `tools/watch_pianist.py calibrate` writes. Kept separate from the homography
/// itself so the tool only has to record where somebody pointed, and the arithmetic
/// stays here where it is tested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Corners {
    /// Far left, far right, near right, near left — the order the eye reads them.
    /// "Far" is the end of the keys away from the player.
    pub corners: [Pixel; 4],
    /// The leftmost white key visible.
    pub lowest_white: u8,
    /// The rightmost white key visible.
    pub highest_white: u8,
}

impl Corners {
    /// Read a corners file.
    pub fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| {
            format!(
                "{} is not a corners file — `python tools/watch_pianist.py calibrate` \
                 writes one",
                path.display()
            )
        })
    }

    /// The map from the frame to the keyboard.
    pub fn fit(&self) -> Result<Homography> {
        Homography::from_keyboard_corners(self.corners, self.lowest_white, self.highest_white)
    }
}

/// Apply a 3×3 projective map to a point.
fn apply(m: &[[f64; 3]; 3], p: (f64, f64)) -> (f64, f64) {
    let w = m[2][0] * p.0 + m[2][1] * p.1 + m[2][2];
    let w = if w.abs() < 1e-12 { 1e-12 } else { w };
    (
        (m[0][0] * p.0 + m[0][1] * p.1 + m[0][2]) / w,
        (m[1][0] * p.0 + m[1][1] * p.1 + m[1][2]) / w,
    )
}

/// The similarity that centres a point set on the origin at an average distance of √2.
fn normalising(points: &[(f64, f64)]) -> [[f64; 3]; 3] {
    let n = points.len() as f64;
    let cx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let cy = points.iter().map(|p| p.1).sum::<f64>() / n;
    let spread = points
        .iter()
        .map(|p| ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt())
        .sum::<f64>()
        / n;
    let scale = if spread > 1e-12 {
        std::f64::consts::SQRT_2 / spread
    } else {
        1.0
    };
    [
        [scale, 0.0, -scale * cx],
        [0.0, scale, -scale * cy],
        [0.0, 0.0, 1.0],
    ]
}

fn multiply(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// Divide through so the bottom-right entry is one, which a homography allows.
fn rescale(mut m: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let d = m[2][2];
    if d.abs() > 1e-12 {
        for row in m.iter_mut() {
            for cell in row.iter_mut() {
                *cell /= d;
            }
        }
    }
    m
}

fn invert(m: &[[f64; 3]; 3]) -> Result<[[f64; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-15 {
        bail!("the calibration is degenerate");
    }
    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let (a, b) = ((i + 1) % 3, (i + 2) % 3);
            let (c, d) = ((j + 1) % 3, (j + 2) % 3);
            // Transposed cofactor: the adjugate.
            out[j][i] = (m[a][c] * m[b][d] - m[a][d] * m[b][c]) / det;
        }
    }
    Ok(out)
}

/// Gaussian elimination with partial pivoting, for the small dense system the DLT
/// produces.
fn solve(mut a: [[f64; 8]; 8], mut b: [f64; 8]) -> Option<[f64; 8]> {
    for column in 0..8 {
        let pivot = (column..8).max_by(|&i, &j| a[i][column].abs().total_cmp(&a[j][column].abs()))?;
        if a[pivot][column].abs() < 1e-12 {
            return None;
        }
        a.swap(column, pivot);
        b.swap(column, pivot);
        for row in (column + 1)..8 {
            let factor = a[row][column] / a[column][column];
            if factor == 0.0 {
                continue;
            }
            for k in column..8 {
                a[row][k] -= factor * a[column][k];
            }
            b[row] -= factor * b[column];
        }
    }
    let mut x = [0.0; 8];
    for row in (0..8).rev() {
        let sum: f64 = ((row + 1)..8).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - sum) / a[row][row];
    }
    Some(x)
}

/// One hand as a landmark detector saw it in one frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeenHand {
    /// Which hand the detector thought it was.
    pub hand: HandLabel,
    /// The five fingertips in image pixels, thumb first.
    ///
    /// MediaPipe's hand model numbers its twenty-one landmarks with the fingertips at
    /// 4, 8, 12, 16 and 20; the tool that writes this file picks those out, so nothing
    /// downstream has to know that.
    pub fingertips: [Pixel; 5],
    /// The wrist, which is what settles handedness when the detector is unsure.
    pub wrist: Pixel,
}

/// One video frame's worth of detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    /// When in the video, in seconds.
    pub time: f64,
    /// The hands found. Zero, one or two.
    pub hands: Vec<SeenHand>,
}

/// What `tools/watch_pianist.py` writes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watched {
    /// Where the video came from, for the record.
    #[serde(default)]
    pub source: String,
    /// Seconds to add to a video timestamp to line it up with the MIDI file.
    #[serde(default)]
    pub offset: f64,
    /// The frames, in time order.
    pub frames: Vec<Frame>,
}

impl Watched {
    /// Read a landmark file.
    pub fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let watched: Self = serde_json::from_str(&text)
            .with_context(|| format!("{} is not a landmark file", path.display()))?;
        if watched.frames.is_empty() {
            bail!("{} has no frames in it", path.display());
        }
        Ok(watched)
    }
}

/// A note the MIDI file says was played.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Onset {
    pub midi: u8,
    pub time: f64,
}

/// Turn a watched performance and its score into fingerings.
///
/// The video says where the fingers were; the MIDI says which keys went down and
/// when. Neither alone is a fingering — this is the join.
pub fn extract(
    watched: &Watched,
    onsets: &[Onset],
    homography: &Homography,
    piece: &str,
    annotator: &str,
) -> Piece {
    let keyboard = Keyboard::new();
    // Notes grouped into the events they were struck as, since the fingers used for a
    // chord have to be chosen together.
    let mut events: Vec<(f64, Vec<u8>)> = Vec::new();
    let mut sorted: Vec<Onset> = onsets.to_vec();
    sorted.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.midi.cmp(&b.midi)));
    for onset in sorted {
        match events.last_mut() {
            Some((time, keys)) if onset.time - *time < ONSET_WINDOW_SECONDS => keys.push(onset.midi),
            _ => events.push((onset.time, vec![onset.midi])),
        }
    }

    let mut notes = Vec::new();
    for (time, keys) in &events {
        let Some(frame) = nearest_frame(watched, *time) else {
            continue;
        };
        for (midi, finger, hand) in assign(&keyboard, keys, frame, homography) {
            notes.push(FingeredNote {
                midi,
                onset: *time,
                // Watching a recording finds when a key goes down and not when it comes
                // back up, so there is nothing better to say than the default.
                duration: crate::corpus::ASSUMED_DURATION,
                hand,
                finger: Some(finger.number()),
            });
        }
    }
    notes.sort_by(|a, b| a.onset.total_cmp(&b.onset).then(a.midi.cmp(&b.midi)));

    Piece {
        piece: piece.to_string(),
        annotator: annotator.to_string(),
        source: watched.source.clone(),
        notes,
    }
}

/// The frame closest in time to a moment, once the video's own offset is applied.
fn nearest_frame(watched: &Watched, time: f64) -> Option<&Frame> {
    let wanted = time - watched.offset;
    watched
        .frames
        .iter()
        .min_by(|a, b| (a.time - wanted).abs().total_cmp(&(b.time - wanted).abs()))
        .filter(|frame| (frame.time - wanted).abs() < 0.25)
}

/// A fingertip on the keyboard.
struct Tip {
    hand: HandLabel,
    finger: Finger,
    /// Along the keys, in millimetres.
    x: f64,
    /// Into the instrument, in millimetres.
    y: f64,
}

/// Choose a finger for every key struck at one moment.
///
/// Greedy and nearest-first, which is what PianoMime does and what the data supports:
/// the tracker is accurate enough to say which finger is over a key and not accurate
/// enough to support anything cleverer. Two rules keep it honest — a finger plays at
/// most one key at a time, and a fingertip that is not over the keyboard at all is not
/// playing anything.
fn assign(
    keyboard: &Keyboard,
    keys: &[u8],
    frame: &Frame,
    homography: &Homography,
) -> Vec<(u8, Finger, HandLabel)> {
    let tips = fingertips(frame, homography);
    let reach = f64::from(on_hand::keyboard::WHITE_KEY_WIDTH) * f64::from(MAX_OFFSET_KEYS);

    // Nearest first over all (key, finger) pairs, rather than key by key, so a key
    // with only one plausible finger is not left without it because an earlier key
    // took that finger on a worse match.
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (k, midi) in keys.iter().enumerate() {
        if !(MIDI_LOWEST..=MIDI_HIGHEST).contains(midi) {
            continue;
        }
        let centre = f64::from(keyboard.centre_x(*midi));
        for (t, tip) in tips.iter().enumerate() {
            if tip.y < f64::from(-MAX_OVERHANG_MM) {
                continue;
            }
            let offset = (tip.x - centre).abs();
            if offset > reach {
                continue;
            }
            pairs.push((offset, k, t));
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut taken_key = vec![false; keys.len()];
    let mut taken_tip = vec![false; tips.len()];
    let mut chosen: Vec<(u8, Finger, HandLabel)> = Vec::new();
    for (_, k, t) in pairs {
        if taken_key[k] || taken_tip[t] {
            continue;
        }
        taken_key[k] = true;
        taken_tip[t] = true;
        chosen.push((keys[k], tips[t].finger, tips[t].hand));
    }
    enforce_ordering(&mut chosen);
    chosen
}

/// Every fingertip the frame shows, on the keyboard.
fn fingertips(frame: &Frame, homography: &Homography) -> Vec<Tip> {
    let mut hands: Vec<&SeenHand> = frame.hands.iter().collect();
    // Two hands: the one further to the players' left is the left hand, whatever the
    // detector said. Handedness is the least reliable thing it reports, and this is
    // never wrong for somebody sitting at a piano.
    if hands.len() == 2 {
        let left_first = homography.to_keyboard(hands[0].wrist).0
            <= homography.to_keyboard(hands[1].wrist).0;
        let (a, b) = if left_first { (0, 1) } else { (1, 0) };
        let ordered = vec![hands[a], hands[b]];
        hands = ordered;
    }

    let mut tips = Vec::with_capacity(10);
    for (index, seen) in hands.iter().enumerate() {
        let hand = if frame.hands.len() == 2 {
            if index == 0 {
                HandLabel::Left
            } else {
                HandLabel::Right
            }
        } else {
            seen.hand
        };
        for (slot, pixel) in seen.fingertips.iter().enumerate() {
            let (x, y) = homography.to_keyboard(*pixel);
            tips.push(Tip {
                hand,
                finger: Finger::from_index(slot),
                x,
                y,
            });
        }
    }
    tips
}

/// Fingers of one hand cannot cross over each other.
///
/// A pianist's fingers stay in pitch order within a hand — the exception, a thumb
/// crossing under, is a moment in a run rather than a chord shape, and it never shows
/// up as two fingers of one hand in the wrong order at the same instant. Tracking
/// noise does produce that, so it is corrected here rather than learned as if it were
/// real.
fn enforce_ordering(chosen: &mut [(u8, Finger, HandLabel)]) {
    for hand in [HandLabel::Left, HandLabel::Right] {
        let mut slots: Vec<usize> = (0..chosen.len()).filter(|i| chosen[*i].2 == hand).collect();
        if slots.len() < 2 {
            continue;
        }
        slots.sort_by_key(|i| chosen[*i].0);
        let mut fingers: Vec<Finger> = slots.iter().map(|i| chosen[*i].1).collect();
        // Low pitch to high runs 5-4-3-2-1 in the left hand and 1-2-3-4-5 in the right.
        fingers.sort_by_key(|f| f.number());
        if hand == HandLabel::Left {
            fingers.reverse();
        }
        for (slot, finger) in slots.into_iter().zip(fingers) {
            chosen[slot].1 = finger;
        }
    }
}

/// How much of a performance was recovered, for the report.
pub fn coverage(piece: &Piece, onsets: &[Onset]) -> (usize, usize) {
    (piece.notes.len(), onsets.len())
}

/// The onsets of a MIDI or MusicXML file, for matching against a video.
pub fn onsets_of(path: &Path) -> Result<Vec<Onset>> {
    let document = on_score::Document::open(path)?;
    let mut onsets: Vec<Onset> = document
        .score()
        .notes
        .iter()
        .filter(|note| !note.tie.is_continuation())
        .map(|note| Onset {
            midi: note.midi,
            time: note.onset_seconds,
        })
        .collect();
    onsets.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.midi.cmp(&b.midi)));
    Ok(onsets)
}

/// Group notes by the hand they were assigned, for a summary line.
pub fn per_hand(piece: &Piece) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::new();
    for note in &piece.notes {
        let key = match note.hand {
            HandLabel::Left => "left",
            HandLabel::Right => "right",
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

/// Which hand a label means, for callers that want the real type.
pub fn hand_of(label: HandLabel) -> Hand {
    Hand::from(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A camera looking straight down at a keyboard, with the image 1920 wide.
    fn overhead() -> Homography {
        let keyboard = Keyboard::new();
        let left = f64::from(keyboard.footprint(MIDI_LOWEST).0);
        let right = f64::from(keyboard.footprint(MIDI_HIGHEST).1);
        // Keys run left to right across the frame; the far end of the keys is at the
        // top, which in image coordinates is a smaller y.
        Homography::fit(&[
            ((0.0, 0.0), (left, f64::from(WHITE_KEY_LENGTH))),
            ((1920.0, 0.0), (right, f64::from(WHITE_KEY_LENGTH))),
            ((1920.0, 300.0), (right, 0.0)),
            ((0.0, 300.0), (left, 0.0)),
        ])
        .unwrap()
    }

    fn pixel_of(homography: &Homography, midi: u8, depth_mm: f64) -> Pixel {
        // Invert by search: the map is smooth and this is only a test helper.
        let keyboard = Keyboard::new();
        let want = (f64::from(keyboard.centre_x(midi)), depth_mm);
        let mut best = (0.0, 0.0);
        let mut best_error = f64::MAX;
        for x in 0..1921 {
            for y in [0, 75, 150, 225, 300] {
                let got = homography.to_keyboard((x as f64, y as f64));
                let error = (got.0 - want.0).abs() + (got.1 - want.1).abs();
                if error < best_error {
                    best_error = error;
                    best = (x as f64, y as f64);
                }
            }
        }
        best
    }

    #[test]
    fn a_homography_maps_the_corners_it_was_given() {
        let homography = overhead();
        let keyboard = Keyboard::new();
        let left = f64::from(keyboard.footprint(MIDI_LOWEST).0);
        let (x, y) = homography.to_keyboard((0.0, 300.0));
        assert!((x - left).abs() < 0.5, "left edge came out at {x}");
        assert!(y.abs() < 0.5, "the near edge came out at {y}");
    }

    #[test]
    fn a_perspective_view_still_measures_the_keyboard() {
        // A camera off to one side: the far edge of the keys is narrower in the image
        // than the near edge, which is what a homography is for and what an affine
        // map cannot do.
        let keyboard = Keyboard::new();
        let left = f64::from(keyboard.footprint(MIDI_LOWEST).0);
        let right = f64::from(keyboard.footprint(MIDI_HIGHEST).1);
        let homography = Homography::fit(&[
            ((260.0, 40.0), (left, f64::from(WHITE_KEY_LENGTH))),
            ((1700.0, 40.0), (right, f64::from(WHITE_KEY_LENGTH))),
            ((1880.0, 420.0), (right, 0.0)),
            ((60.0, 420.0), (left, 0.0)),
        ])
        .unwrap();
        // The middle of the far edge has to land in the middle of the keyboard.
        let (x, y) = homography.to_keyboard((980.0, 40.0));
        assert!(
            (x - (left + right) / 2.0).abs() < 1.0,
            "the centre came out at {x}"
        );
        assert!(
            (y - f64::from(WHITE_KEY_LENGTH)).abs() < 1.0,
            "the far edge came out at {y}"
        );
    }

    #[test]
    fn four_points_are_the_fewest_that_will_do() {
        let err = Homography::fit(&[((0.0, 0.0), (0.0, 0.0)); 3]).unwrap_err();
        assert!(err.to_string().contains("four points"), "{err}");
    }

    fn hand_at(homography: &Homography, hand: HandLabel, keys: [u8; 5]) -> SeenHand {
        let tips = keys.map(|midi| pixel_of(homography, midi, 40.0));
        SeenHand {
            hand,
            fingertips: tips,
            wrist: pixel_of(homography, keys[2], 0.0),
        }
    }

    #[test]
    fn it_credits_the_finger_that_was_over_the_key() {
        let homography = overhead();
        // A right hand in a five-finger position on middle C.
        let frame = Frame {
            time: 1.0,
            hands: vec![hand_at(&homography, HandLabel::Right, [60, 62, 64, 65, 67])],
        };
        let watched = Watched {
            source: "test".into(),
            offset: 0.0,
            frames: vec![frame],
        };
        let onsets = vec![Onset { midi: 64, time: 1.0 }];
        let piece = extract(&watched, &onsets, &homography, "test", "1");
        assert_eq!(piece.notes.len(), 1);
        assert_eq!(piece.notes[0].midi, 64);
        assert_eq!(piece.notes[0].finger, Some(3), "the middle finger was over E");
    }

    #[test]
    fn one_finger_cannot_play_two_keys_at_once() {
        let homography = overhead();
        let frame = Frame {
            time: 0.0,
            hands: vec![hand_at(&homography, HandLabel::Right, [60, 62, 64, 65, 67])],
        };
        let watched = Watched {
            source: "test".into(),
            offset: 0.0,
            frames: vec![frame],
        };
        let onsets = vec![
            Onset { midi: 60, time: 0.0 },
            Onset { midi: 64, time: 0.0 },
            Onset { midi: 67, time: 0.0 },
        ];
        let piece = extract(&watched, &onsets, &homography, "test", "1");
        assert_eq!(piece.notes.len(), 3);
        let mut fingers: Vec<u8> = piece.notes.iter().map(|n| n.finger.expect("video extraction always names a finger")).collect();
        fingers.sort_unstable();
        fingers.dedup();
        assert_eq!(fingers.len(), 3, "a finger was used twice");
    }

    #[test]
    fn a_hand_that_is_not_over_the_keys_plays_nothing() {
        let homography = overhead();
        // Fingertips well in front of the keyboard, as when a hand is resting in the
        // player's lap.
        let below = pixel_of(&homography, 60, 0.0);
        let off = (below.0, below.1 + 200.0);
        let frame = Frame {
            time: 0.0,
            hands: vec![SeenHand {
                hand: HandLabel::Right,
                fingertips: [off; 5],
                wrist: off,
            }],
        };
        let watched = Watched {
            source: "test".into(),
            offset: 0.0,
            frames: vec![frame],
        };
        let onsets = vec![Onset { midi: 60, time: 0.0 }];
        let piece = extract(&watched, &onsets, &homography, "test", "1");
        assert!(piece.notes.is_empty(), "{:?}", piece.notes);
    }

    #[test]
    fn fingers_of_one_hand_never_cross() {
        let mut chosen = vec![
            (67, Finger::Thumb, HandLabel::Right),
            (60, Finger::Middle, HandLabel::Right),
            (64, Finger::Little, HandLabel::Right),
        ];
        enforce_ordering(&mut chosen);
        chosen.sort_by_key(|c| c.0);
        let fingers: Vec<u8> = chosen.iter().map(|c| c.1.number()).collect();
        assert_eq!(fingers, vec![1, 3, 5], "right hand rises with pitch");

        let mut left = vec![
            (48, Finger::Thumb, HandLabel::Left),
            (55, Finger::Little, HandLabel::Left),
        ];
        enforce_ordering(&mut left);
        left.sort_by_key(|c| c.0);
        let fingers: Vec<u8> = left.iter().map(|c| c.1.number()).collect();
        assert_eq!(fingers, vec![5, 1], "left hand falls with pitch");
    }

    /// The video's clock and the score's clock rarely start together, and the offset
    /// has to actually move which frame is consulted.
    #[test]
    fn the_offset_lines_the_video_up_with_the_score() {
        let homography = overhead();
        let watched = Watched {
            source: "test".into(),
            offset: 2.0,
            frames: vec![
                Frame {
                    time: 0.0,
                    hands: vec![hand_at(&homography, HandLabel::Right, [72, 74, 76, 77, 79])],
                },
                Frame {
                    time: 1.0,
                    hands: vec![hand_at(&homography, HandLabel::Right, [60, 62, 64, 65, 67])],
                },
            ],
        };
        // A note at t = 3 in the score is at t = 1 in the video.
        let onsets = vec![Onset { midi: 65, time: 3.0 }];
        let piece = extract(&watched, &onsets, &homography, "test", "1");
        assert_eq!(piece.notes.len(), 1);
        assert_eq!(piece.notes[0].finger, Some(4), "the ring finger was over F");
    }
}
