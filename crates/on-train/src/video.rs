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
//! assignment against the notes that are sounding — read from every frame in a short
//! window around each onset and put to a vote, so one frame the tracker got wrong is
//! outvoted by the frames around it rather than written straight into the corpus. Their
//! published pipeline was aimed at driving a robot
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
use on_hand::keyboard::{is_black, Keyboard, BLACK_KEY_FRONT_Y, MIDI_HIGHEST, MIDI_LOWEST, WHITE_KEY_LENGTH};
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

/// How far in front of a black key's front edge a fingertip may be and still be
/// credited with pressing it, in millimetres.
///
/// Measured on three PianoVAM performances, where the piano recorded every key: when a
/// black key went down, the fingertip over it was a median 76 to 91 mm into the
/// keyboard, and 95% of the time more than 61 mm — past the black keys' front edge at
/// 55. The slack is for the calibration and the tracking, not for the hand.
const BLACK_KEY_SLACK_MM: f32 = 15.0;

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
        let mut frames = frames_around(watched, *time);
        if frames.is_empty() {
            // Nothing in the window — a dropped stretch of video, or a frame rate low
            // enough to straddle it. The nearest frame is still worth something.
            frames.extend(nearest_frame(watched, *time));
        }
        if frames.is_empty() {
            continue;
        }
        for (midi, finger, hand, confidence) in vote(&keyboard, keys, &frames, homography) {
            notes.push(FingeredNote {
                midi,
                onset: *time,
                // Watching a recording finds when a key goes down and not when it comes
                // back up, so there is nothing better to say than the default.
                duration: crate::corpus::ASSUMED_DURATION,
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
) -> Vec<(u8, Finger, HandLabel, f32)> {
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
            // A black key is pressed on its own length, which starts well back from the
            // front of the keyboard: a fingertip out in front of it is on a white key,
            // however nearly it lines up across.
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
        // The nearest *other* fingertip to this key, whether or not it ended up playing
        // something else: if it was nearly as close, which one went down is uncertain
        // however it was then resolved.
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

/// Put a chord's fingers in the order a hand can hold them, and believe any finger that
/// had to be moved to get there less.
///
/// The tracker can report two fingers of one hand crossed, which a hand at a keyboard
/// does not do, and [`enforce_ordering`] puts them right. A finger that ends up where it
/// is because of that was assumed rather than observed, so it is believed half as much.
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

/// How far before a note's onset frames are read from, in seconds.
const BEFORE_ONSET: f64 = 0.05;
/// And how far after.
///
/// More after than before, on purpose. Before a key goes down the finger is still on its
/// way and may be over the neighbouring key; once it is down it stays there for as long
/// as the note is held, so the frames just after the onset are the ones that show which
/// finger it was.
const AFTER_ONSET: f64 = 0.15;

/// Every frame close enough to a note's onset to say which finger struck it.
fn frames_around(watched: &Watched, time: f64) -> Vec<&Frame> {
    let at = time - watched.offset;
    watched
        .frames
        .iter()
        .filter(|frame| frame.time >= at - BEFORE_ONSET && frame.time <= at + AFTER_ONSET)
        .collect()
}

/// Read a chord from several frames and let them vote.
///
/// One frame is one chance for the tracker to be wrong — a fingertip that jumps to the
/// next knuckle for a single frame, a hand briefly lost behind the other — and reading a
/// chord off one frame turns every such glitch straight into a wrong label. Reading it
/// off each frame around the onset and taking the finger that most of them agree on,
/// weighted by how sure each was, outvotes a single bad frame instead.
///
/// The confidence of the answer is the weight the winning finger collected divided by
/// the number of frames, so it falls when the frames disagree, when some of them did not
/// see the key at all, and when the ones that agreed were unsure — three kinds of doubt,
/// in one number, without having to decide how to trade them against each other. From a
/// single frame it is exactly that frame's own confidence.
fn vote(
    keyboard: &Keyboard,
    keys: &[u8],
    frames: &[&Frame],
    homography: &Homography,
) -> Vec<(u8, Finger, HandLabel, f32)> {
    use std::collections::BTreeMap;

    // Per key: per finger, the weight it collected and the hand it was seen in.
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
        // Most weight wins; a tie goes to the lower finger number, so the answer does not
        // depend on the order the frames happened to be read in.
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
    // Each frame's reading was already in order, but the winners for different keys can
    // come from different frames, and two of those can cross.
    settle(chosen, trust)
}

/// How far a finger that had to be put back in order is believed, against one that was
/// seen where it was.
const CORRECTED: f32 = 0.5;

/// How far to believe a finger read off one frame, from 0 to 1.
///
/// Two things, and the weaker of them decides. How close the fingertip was to the centre
/// of its key, against how far one may be and still count: a tip right over a key is a
/// strong reading, and one at the edge of the tolerance is a guess that happened to be
/// the nearest. And how clearly it won: if another fingertip was nearly as close, which
/// one went down is close to a coin toss, however near the winner was.
///
/// Not calibrated, and it does not claim to be — it has nothing yet to be calibrated
/// against. It orders readings from most to least believable, which is what a floor on
/// it needs, and the floor is set by watching whether the benchmark improves as it rises.
fn reading_confidence(offset: f64, runner_up: Option<f64>, reach: f64) -> f32 {
    let key = f64::from(on_hand::keyboard::WHITE_KEY_WIDTH);
    let near = (1.0 - offset / reach).clamp(0.0, 1.0);
    let clear = runner_up.map_or(1.0, |other| ((other - offset) / key).clamp(0.0, 1.0));
    near.min(clear) as f32
}

/// Every fingertip the frame shows, on the keyboard.
fn fingertips(frame: &Frame, homography: &Homography) -> Vec<Tip> {
    let mut hands: Vec<&SeenHand> = frame.hands.iter().collect();
    // Two hands: which is which, whatever the detector said, since handedness is the
    // least reliable thing it reports. The hands' shapes say it first: palm down, a
    // right hand has its thumb to the left of its little finger and a left hand the
    // other way round, wherever it is — so arms crossed over are still read right.
    // Only when the shapes do not settle it is the hand further left taken as the left.
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

    /// A reading is believed by how close the fingertip was and how clearly it won, and
    /// the weaker of the two decides. Right over a key with nothing else near is as sure
    /// as a video gets; at the edge of what counts, or tied with another fingertip, it is
    /// a guess however good the other half looks.
    #[test]
    fn a_reading_is_believed_by_how_close_and_how_clear_it_was() {
        let key = f64::from(on_hand::keyboard::WHITE_KEY_WIDTH);
        let reach = key * f64::from(MAX_OFFSET_KEYS);

        assert_eq!(reading_confidence(0.0, None, reach), 1.0, "dead over the key, alone");
        assert!(reading_confidence(reach, None, reach) < 0.01, "at the edge of the tolerance");
        assert!(reading_confidence(0.0, Some(0.0), reach) < 0.01, "tied with another finger");
        assert_eq!(reading_confidence(0.0, Some(key), reach), 1.0, "a clear key's width ahead");

        // And the weaker half decides, rather than the two averaging out.
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

    /// The pixel that lands at a point on the keyboard, by search.
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
        // The thumb lies exactly in line with C#4 but out on the white keys in front of
        // it; the index is a little to the side, on the black key itself. Only the index
        // can have played it.
        let mut hand = hand_at(&homography, HandLabel::Right, [60, 62, 64, 65, 67]);
        hand.fingertips[0] = pixel_at(&homography, (across, 15.0));
        hand.fingertips[1] = pixel_at(&homography, (across + 5.0, 100.0));
        let watched = Watched {
            source: "test".into(),
            offset: 0.0,
            frames: vec![Frame { time: 1.0, hands: vec![hand] }],
        };
        let piece = extract(&watched, &[Onset { midi: 61, time: 1.0 }], &homography, "test", "1");
        assert_eq!(piece.notes[0].finger, Some(2), "the thumb was credited from in front");
    }

    #[test]
    fn arms_crossed_over_are_still_told_apart() {
        let homography = overhead();
        // The right hand has crossed down into the bass and the left up into the
        // treble, and the detector, as it will, has called both wrong.
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
        let onsets = vec![Onset { midi: 52, time: 1.0 }, Onset { midi: 76, time: 1.0 }];
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
    /// One frame the tracker got wrong is outvoted by the frames either side of it,
    /// rather than written straight into the corpus as a wrong finger.
    ///
    /// Three frames of a right hand in a five-finger position on middle C, the middle one
    /// glitched a key to the right so the index finger reads as being over E. Read off
    /// that one frame alone, E would be labelled 2. Voted, the two good frames win, and
    /// the disagreement shows up as a confidence of about two thirds rather than as a
    /// wrong answer stated with certainty.
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
        let onsets = vec![Onset { midi: 64, time: 1.0 }];

        let piece = extract(&watched, &onsets, &homography, "test", "1");
        assert_eq!(piece.notes.len(), 1);
        assert_eq!(piece.notes[0].finger, Some(3), "the two good frames should win");
        let confidence = piece.notes[0].confidence.expect("a machine reading carries one");
        assert!((0.6..0.7).contains(&confidence), "two of three frames agreed: {confidence}");

        // And when every frame agrees, voting believes exactly what one frame would —
        // agreement costs nothing, only disagreement does.
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
        // A note at t = 3 in the score is at t = 1 in the video.
        let onsets = vec![Onset { midi: 65, time: 3.0 }];
        let piece = extract(&watched, &onsets, &homography, "test", "1");
        assert_eq!(piece.notes.len(), 1);
        assert_eq!(piece.notes[0].finger, Some(4), "the ring finger was over F");
    }
}
