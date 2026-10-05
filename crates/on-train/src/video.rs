use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use on_hand::keyboard::{is_black, Keyboard, BLACK_KEY_FRONT_Y, MIDI_HIGHEST, MIDI_LOWEST, WHITE_KEY_LENGTH};
use on_hand::{Finger, Hand};
use serde::{Deserialize, Serialize};

use crate::corpus::{FingeredNote, HandLabel, Piece};

const MAX_OFFSET_KEYS: f32 = 2.0;

const MAX_OVERHANG_MM: f32 = 3.0;

const BLACK_KEY_SLACK_MM: f32 = 15.0;

const ONSET_WINDOW_SECONDS: f64 = 0.05;

pub type Pixel = (f64, f64);

pub type Millimetres = (f64, f64);

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Homography {
    pub m: [[f64; 3]; 3],
}

impl Homography {
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
        let inverse_to = invert(&norm_to)?;

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
        let m = multiply(&multiply(&inverse_to, &normalised), &norm_from);
        Ok(Self { m: rescale(m) })
    }

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

    pub fn to_keyboard(&self, pixel: Pixel) -> Millimetres {
        apply(&self.m, pixel)
    }

    pub fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("{} is not a calibration", path.display()))
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Corners {
    pub corners: [Pixel; 4],
    pub lowest_white: u8,
    pub highest_white: u8,
}

impl Corners {
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

    pub fn fit(&self) -> Result<Homography> {
        Homography::from_keyboard_corners(self.corners, self.lowest_white, self.highest_white)
    }
}

fn apply(m: &[[f64; 3]; 3], p: (f64, f64)) -> (f64, f64) {
    let w = m[2][0] * p.0 + m[2][1] * p.1 + m[2][2];
    let w = if w.abs() < 1e-12 { 1e-12 } else { w };
    (
        (m[0][0] * p.0 + m[0][1] * p.1 + m[0][2]) / w,
        (m[1][0] * p.0 + m[1][1] * p.1 + m[1][2]) / w,
    )
}

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
            out[j][i] = (m[a][c] * m[b][d] - m[a][d] * m[b][c]) / det;
        }
    }
    Ok(out)
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeenHand {
    pub hand: HandLabel,
    pub fingertips: [Pixel; 5],
    pub wrist: Pixel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub time: f64,
    pub hands: Vec<SeenHand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watched {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub offset: f64,
    pub frames: Vec<Frame>,
}

impl Watched {
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Onset {
    pub midi: u8,
    pub time: f64,
    pub duration: f64,
}

pub fn extract(
    watched: &Watched,
    onsets: &[Onset],
    homography: &Homography,
    piece: &str,
    annotator: &str,
) -> Piece {
    let keyboard = Keyboard::new();
    let mut events: Vec<(f64, Vec<u8>)> = Vec::new();
    let mut held: Vec<Vec<f64>> = Vec::new();
    let mut sorted: Vec<Onset> = onsets.to_vec();
    sorted.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.midi.cmp(&b.midi)));
    for onset in sorted {
        match events.last_mut() {
            Some((time, keys)) if onset.time - *time < ONSET_WINDOW_SECONDS => {
                keys.push(onset.midi);
                held.last_mut().expect("one list of holds per event").push(onset.duration);
            }
            _ => {
                events.push((onset.time, vec![onset.midi]));
                held.push(vec![onset.duration]);
            }
        }
    }

    let mut notes = Vec::new();
    for ((time, keys), durations) in events.iter().zip(&held) {
        let mut frames = frames_around(watched, *time);
        if frames.is_empty() {
            frames.extend(nearest_frame(watched, *time));
        }
        if frames.is_empty() {
            continue;
        }
        for (midi, finger, hand, confidence) in vote(&keyboard, keys, &frames, homography) {
            notes.push(FingeredNote {
                midi,
                onset: *time,
                duration: keys
                    .iter()
                    .position(|&k| k == midi)
                    .map_or(crate::corpus::ASSUMED_DURATION, |i| durations[i]),
                hand,
                finger: Some(finger.number()),
                confidence: Some(confidence),
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

fn nearest_frame(watched: &Watched, time: f64) -> Option<&Frame> {
    let wanted = time - watched.offset;
    watched
        .frames
        .iter()
        .min_by(|a, b| (a.time - wanted).abs().total_cmp(&(b.time - wanted).abs()))
        .filter(|frame| (frame.time - wanted).abs() < 0.25)
}

struct Tip {
    hand: HandLabel,
    finger: Finger,
    x: f64,
    y: f64,
}

fn assign(
    keyboard: &Keyboard,
    keys: &[u8],
    frame: &Frame,
    homography: &Homography,
) -> Vec<(u8, Finger, HandLabel, f32)> {
    let tips = fingertips(frame, homography);
    let reach = f64::from(on_hand::keyboard::WHITE_KEY_WIDTH) * f64::from(MAX_OFFSET_KEYS);

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
            if is_black(*midi) && tip.y < f64::from(BLACK_KEY_FRONT_Y - BLACK_KEY_SLACK_MM) {
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
    let mut trust: Vec<f32> = Vec::new();
    for &(offset, k, t) in &pairs {
        if taken_key[k] || taken_tip[t] {
            continue;
        }
        taken_key[k] = true;
        taken_tip[t] = true;
        let runner_up = pairs
            .iter()
            .filter(|(_, other_key, other_tip)| *other_key == k && *other_tip != t)
            .map(|(distance, _, _)| *distance)
            .fold(None, |nearest: Option<f64>, d| Some(nearest.map_or(d, |n| n.min(d))));
        chosen.push((keys[k], tips[t].finger, tips[t].hand));
        trust.push(reading_confidence(offset, runner_up, reach));
    }

    settle(chosen, trust)
}

fn settle(
    mut chosen: Vec<(u8, Finger, HandLabel)>,
    mut trust: Vec<f32>,
) -> Vec<(u8, Finger, HandLabel, f32)> {
    let seen: Vec<Finger> = chosen.iter().map(|c| c.1).collect();
    enforce_ordering(&mut chosen);
    for ((now, before), confidence) in chosen.iter().zip(&seen).zip(&mut trust) {
        if now.1 != *before {
            *confidence *= CORRECTED;
        }
    }
    chosen
        .into_iter()
        .zip(trust)
        .map(|((midi, finger, hand), confidence)| (midi, finger, hand, confidence))
        .collect()
}

const BEFORE_ONSET: f64 = 0.05;
const AFTER_ONSET: f64 = 0.15;

fn frames_around(watched: &Watched, time: f64) -> Vec<&Frame> {
    let at = time - watched.offset;
    watched
        .frames
        .iter()
        .filter(|frame| frame.time >= at - BEFORE_ONSET && frame.time <= at + AFTER_ONSET)
        .collect()
}

fn vote(
    keyboard: &Keyboard,
    keys: &[u8],
    frames: &[&Frame],
    homography: &Homography,
) -> Vec<(u8, Finger, HandLabel, f32)> {
    use std::collections::BTreeMap;

    let mut tally: BTreeMap<u8, BTreeMap<u8, (f32, HandLabel)>> = BTreeMap::new();
    for frame in frames {
        for (midi, finger, hand, confidence) in assign(keyboard, keys, frame, homography) {
            let entry = tally
                .entry(midi)
                .or_default()
                .entry(finger.number())
                .or_insert((0.0, hand));
            entry.0 += confidence;
        }
    }

    let frames = frames.len().max(1) as f32;
    let mut chosen = Vec::new();
    let mut trust = Vec::new();
    for (midi, fingers) in tally {
        let Some((number, (weight, hand))) = fingers
            .into_iter()
            .max_by(|a, b| a.1 .0.total_cmp(&b.1 .0).then(b.0.cmp(&a.0)))
        else {
            continue;
        };
        let Some(finger) = Finger::from_number(number) else { continue };
        chosen.push((midi, finger, hand));
        trust.push((weight / frames).clamp(0.0, 1.0));
    }
    settle(chosen, trust)
}

const CORRECTED: f32 = 0.5;

fn reading_confidence(offset: f64, runner_up: Option<f64>, reach: f64) -> f32 {
    let key = f64::from(on_hand::keyboard::WHITE_KEY_WIDTH);
    let near = (1.0 - offset / reach).clamp(0.0, 1.0);
    let clear = runner_up.map_or(1.0, |other| ((other - offset) / key).clamp(0.0, 1.0));
    near.min(clear) as f32
}

fn fingertips(frame: &Frame, homography: &Homography) -> Vec<Tip> {
    let mut hands: Vec<&SeenHand> = frame.hands.iter().collect();
    if hands.len() == 2 {
        let thumb_left = |seen: &SeenHand| {
            homography.to_keyboard(seen.fingertips[0]).0
                < homography.to_keyboard(seen.fingertips[4]).0
        };
        let left_first = match (thumb_left(hands[0]), thumb_left(hands[1])) {
            (false, true) => true,
            (true, false) => false,
            _ => {
                homography.to_keyboard(hands[0].wrist).0
                    <= homography.to_keyboard(hands[1].wrist).0
            }
        };
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

fn enforce_ordering(chosen: &mut [(u8, Finger, HandLabel)]) {
    for hand in [HandLabel::Left, HandLabel::Right] {
        let mut slots: Vec<usize> = (0..chosen.len()).filter(|i| chosen[*i].2 == hand).collect();
        if slots.len() < 2 {
            continue;
        }
        slots.sort_by_key(|i| chosen[*i].0);
        let mut fingers: Vec<Finger> = slots.iter().map(|i| chosen[*i].1).collect();
        fingers.sort_by_key(|f| f.number());
        if hand == HandLabel::Left {
            fingers.reverse();
        }
        for (slot, finger) in slots.into_iter().zip(fingers) {
            chosen[slot].1 = finger;
        }
    }
}

pub fn coverage(piece: &Piece, onsets: &[Onset]) -> (usize, usize) {
    (piece.notes.len(), onsets.len())
}

pub fn onsets_of(path: &Path) -> Result<Vec<Onset>> {
    let document = on_score::Document::open(path)?;
    let score = document.score();
    let mut onsets: Vec<Onset> = score
        .notes
        .iter()
        .filter(|note| !note.tie.is_continuation())
        .map(|note| Onset {
            midi: note.midi,
            time: note.onset_seconds,
            duration: (score.release_seconds(note.id) - note.onset_seconds).max(0.0),
        })
        .collect();
    onsets.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.midi.cmp(&b.midi)));
    Ok(onsets)
}

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

pub fn hand_of(label: HandLabel) -> Hand {
    Hand::from(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overhead() -> Homography {
        let keyboard = Keyboard::new();
        let left = f64::from(keyboard.footprint(MIDI_LOWEST).0);
        let right = f64::from(keyboard.footprint(MIDI_HIGHEST).1);
        Homography::fit(&[
            ((0.0, 0.0), (left, f64::from(WHITE_KEY_LENGTH))),
            ((1920.0, 0.0), (right, f64::from(WHITE_KEY_LENGTH))),
            ((1920.0, 300.0), (right, 0.0)),
            ((0.0, 300.0), (left, 0.0)),
        ])
        .unwrap()
    }

    fn pixel_of(homography: &Homography, midi: u8, depth_mm: f64) -> Pixel {
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
    fn a_reading_is_believed_by_how_close_and_how_clear_it_was() {
        let key = f64::from(on_hand::keyboard::WHITE_KEY_WIDTH);
        let reach = key * f64::from(MAX_OFFSET_KEYS);

        assert_eq!(reading_confidence(0.0, None, reach), 1.0, "dead over the key, alone");
        assert!(reading_confidence(reach, None, reach) < 0.01, "at the edge of the tolerance");
        assert!(reading_confidence(0.0, Some(0.0), reach) < 0.01, "tied with another finger");
        assert_eq!(reading_confidence(0.0, Some(key), reach), 1.0, "a clear key's width ahead");

        let close_but_contested = reading_confidence(0.0, Some(key * 0.1), reach);
        assert!(close_but_contested < 0.15, "{close_but_contested}");
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
        let frame = Frame {
            time: 1.0,
            hands: vec![hand_at(&homography, HandLabel::Right, [60, 62, 64, 65, 67])],
        };
        let watched = Watched {
            source: "test".into(),
            offset: 0.0,
            frames: vec![frame],
        };
        let onsets = vec![Onset { midi: 64, time: 1.0, duration: 0.25 }];
        let piece = extract(&watched, &onsets, &homography, "test", "1");
        assert_eq!(piece.notes.len(), 1);
        assert_eq!(piece.notes[0].midi, 64);
        assert_eq!(piece.notes[0].finger, Some(3), "the middle finger was over E");
    }

    #[test]
    fn a_note_keeps_how_long_it_was_held() {
        let homography = overhead();
        let watched = Watched {
            source: "test".into(),
            offset: 0.0,
            frames: vec![Frame { time: 1.0, hands: vec![hand_at(&homography, HandLabel::Right, [60, 62, 64, 65, 67])] }],
        };
        let onsets = vec![
            Onset { midi: 60, time: 1.0, duration: 2.0 },
            Onset { midi: 67, time: 1.0, duration: 0.1 },
        ];
        let piece = extract(&watched, &onsets, &homography, "test", "1");
        let held: Vec<(u8, f64)> = piece.notes.iter().map(|n| (n.midi, n.duration)).collect();
        assert_eq!(held, vec![(60, 2.0), (67, 0.1)]);
    }

    fn pixel_at(homography: &Homography, want: Millimetres) -> Pixel {
        let mut best = ((0.0, 0.0), f64::MAX);
        for x in 0..1921 {
            for y in 0..301 {
                let got = homography.to_keyboard((f64::from(x), f64::from(y)));
                let error = (got.0 - want.0).abs() + (got.1 - want.1).abs();
                if error < best.1 {
                    best = ((f64::from(x), f64::from(y)), error);
                }
            }
        }
        best.0
    }

    #[test]
    fn a_fingertip_in_front_of_a_black_key_is_not_pressing_it() {
        let homography = overhead();
        let keyboard = Keyboard::new();
        let across = f64::from(keyboard.centre_x(61));
        let mut hand = hand_at(&homography, HandLabel::Right, [60, 62, 64, 65, 67]);
        hand.fingertips[0] = pixel_at(&homography, (across, 15.0));
        hand.fingertips[1] = pixel_at(&homography, (across + 5.0, 100.0));
        let watched = Watched {
            source: "test".into(),
            offset: 0.0,
            frames: vec![Frame { time: 1.0, hands: vec![hand] }],
        };
        let piece = extract(&watched, &[Onset { midi: 61, time: 1.0, duration: 0.25 }], &homography, "test", "1");
        assert_eq!(piece.notes[0].finger, Some(2), "the thumb was credited from in front");
    }

    #[test]
    fn arms_crossed_over_are_still_told_apart() {
        let homography = overhead();
        let frame = Frame {
            time: 1.0,
            hands: vec![
                hand_at(&homography, HandLabel::Left, [48, 50, 52, 53, 55]),
                hand_at(&homography, HandLabel::Right, [79, 77, 76, 74, 72]),
            ],
        };
        let watched = Watched {
            source: "test".into(),
            offset: 0.0,
            frames: vec![frame],
        };
        let onsets = vec![Onset { midi: 52, time: 1.0, duration: 0.25 }, Onset { midi: 76, time: 1.0, duration: 0.25 }];
        let piece = extract(&watched, &onsets, &homography, "test", "1");
        let bass = piece.notes.iter().find(|n| n.midi == 52).expect("the bass note was read");
        let treble = piece.notes.iter().find(|n| n.midi == 76).expect("the treble note was read");
        assert_eq!((bass.hand, bass.finger), (HandLabel::Right, Some(3)));
        assert_eq!((treble.hand, treble.finger), (HandLabel::Left, Some(3)));
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
            Onset { midi: 60, time: 0.0, duration: 0.25 },
            Onset { midi: 64, time: 0.0, duration: 0.25 },
            Onset { midi: 67, time: 0.0, duration: 0.25 },
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
        let onsets = vec![Onset { midi: 60, time: 0.0, duration: 0.25 }];
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

    #[test]
    fn a_frame_the_tracker_got_wrong_is_outvoted() {
        let homography = overhead();
        let step = 1.0 / 15.0;
        let good = || hand_at(&homography, HandLabel::Right, [60, 62, 64, 65, 67]);
        let glitched = hand_at(&homography, HandLabel::Right, [62, 64, 65, 67, 69]);
        let watched = Watched {
            source: "test".into(),
            offset: 0.0,
            frames: vec![
                Frame { time: 1.0, hands: vec![good()] },
                Frame { time: 1.0 + step, hands: vec![glitched] },
                Frame { time: 1.0 + 2.0 * step, hands: vec![good()] },
            ],
        };
        let onsets = vec![Onset { midi: 64, time: 1.0, duration: 0.25 }];

        let piece = extract(&watched, &onsets, &homography, "test", "1");
        assert_eq!(piece.notes.len(), 1);
        assert_eq!(piece.notes[0].finger, Some(3), "the two good frames should win");
        let confidence = piece.notes[0].confidence.expect("a machine reading carries one");
        assert!((0.6..0.7).contains(&confidence), "two of three frames agreed: {confidence}");

        let alone = Watched {
            frames: vec![Frame { time: 1.0, hands: vec![good()] }],
            ..watched.clone()
        };
        let unanimous = Watched {
            frames: vec![
                Frame { time: 1.0, hands: vec![good()] },
                Frame { time: 1.0 + step, hands: vec![good()] },
                Frame { time: 1.0 + 2.0 * step, hands: vec![good()] },
            ],
            ..watched
        };
        let one = extract(&alone, &onsets, &homography, "test", "1").notes[0].confidence.unwrap();
        let three =
            extract(&unanimous, &onsets, &homography, "test", "1").notes[0].confidence.unwrap();
        assert!((one - three).abs() < 1e-5, "three agreeing frames gave {three}, one gave {one}");
        assert!(one > 0.95, "a clean reading should be well believed: {one}");
        assert!(confidence < one, "a split vote has to be believed less than a clean one");
    }

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
        let onsets = vec![Onset { midi: 65, time: 3.0, duration: 0.25 }];
        let piece = extract(&watched, &onsets, &homography, "test", "1");
        assert_eq!(piece.notes.len(), 1);
        assert_eq!(piece.notes[0].finger, Some(4), "the ring finger was over F");
    }
}
