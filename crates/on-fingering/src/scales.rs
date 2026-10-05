use std::collections::HashMap;

use on_hand::{Finger, Hand};

pub const SCALE_BONUS: f32 = 7.5;

pub const CHROMATIC_BONUS: f32 = 12.0;

pub const MIN_SCALE_RUN: usize = 6;

const RUN_GAP_FACTOR: f64 = 2.5;

const RUN_GAP_CEILING_SECONDS: f64 = 2.0;

pub const MIN_CHROMATIC_RUN: usize = 5;

pub const ARPEGGIO_BONUS: f32 = SCALE_BONUS;

pub const MIN_ARPEGGIO_RUN: usize = 7;

fn split_on_gaps(start: usize, end: usize, onsets: &[f64]) -> Vec<(usize, usize)> {
    let gaps: Vec<f64> = (start + 1..end)
        .filter_map(|i| Some(onsets.get(i)? - onsets.get(i - 1)?))
        .collect();
    if gaps.len() != end - start - 1 || gaps.len() < 2 {
        return vec![(start, end)];
    }
    let mut sorted = gaps.clone();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    let limit = if median > 0.0 {
        (median * RUN_GAP_FACTOR).min(RUN_GAP_CEILING_SECONDS)
    } else {
        RUN_GAP_CEILING_SECONDS
    };

    let mut out = Vec::new();
    let mut from = start;
    for (k, gap) in gaps.iter().enumerate() {
        if *gap > limit {
            out.push((from, start + 1 + k));
            from = start + 1 + k;
        }
    }
    out.push((from, end));
    out
}

const MAJOR_DEGREES: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];

struct ScaleFingering {
    right: [u8; 7],
    left: [u8; 7],
}

const MAJOR_SCALES: [ScaleFingering; 12] = [
    ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] },
    ScaleFingering { right: [2, 3, 1, 2, 3, 4, 1], left: [3, 2, 1, 4, 3, 2, 1] },
    ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] },
    ScaleFingering { right: [3, 1, 2, 3, 4, 1, 2], left: [3, 2, 1, 4, 3, 2, 1] },
    ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] },
    ScaleFingering { right: [1, 2, 3, 4, 1, 2, 3], left: [1, 4, 3, 2, 1, 3, 2] },
    ScaleFingering { right: [2, 3, 4, 1, 2, 3, 1], left: [4, 3, 2, 1, 3, 2, 1] },
    ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] },
    ScaleFingering { right: [3, 4, 1, 2, 3, 1, 2], left: [3, 2, 1, 4, 3, 2, 1] },
    ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] },
    ScaleFingering { right: [4, 1, 2, 3, 1, 2, 3], left: [3, 2, 1, 4, 3, 2, 1] },
    ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 3, 2, 1, 4, 3, 2] },
];

const HARMONIC_MINOR_DEGREES: [u8; 7] = [0, 2, 3, 5, 7, 8, 11];

const HARMONIC_MINOR_SCALES: [Option<ScaleFingering>; 12] = [
    Some(ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] }),
    None,
    Some(ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] }),
    None,
    Some(ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] }),
    Some(ScaleFingering { right: [1, 2, 3, 4, 1, 2, 3], left: [1, 4, 3, 2, 1, 3, 2] }),
    None,
    Some(ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] }),
    None,
    Some(ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] }),
    None,
    Some(ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 3, 2, 1, 4, 3, 2] }),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Major,
    HarmonicMinor,
}

impl Mode {
    fn degrees(self) -> &'static [u8; 7] {
        match self {
            Mode::Major => &MAJOR_DEGREES,
            Mode::HarmonicMinor => &HARMONIC_MINOR_DEGREES,
        }
    }

    fn fingering(self, tonic: u8) -> Option<&'static ScaleFingering> {
        match self {
            Mode::Major => Some(&MAJOR_SCALES[tonic as usize % 12]),
            Mode::HarmonicMinor => HARMONIC_MINOR_SCALES[tonic as usize % 12].as_ref(),
        }
    }
}

fn is_black(pitch: u8) -> bool {
    matches!(pitch % 12, 1 | 3 | 6 | 8 | 10)
}

fn chromatic_finger(hand: Hand, pitch: u8) -> u8 {
    if is_black(pitch) {
        return 3;
    }
    let paired = match hand {
        Hand::Right => [0, 5],
        Hand::Left => [4, 11],
    };
    if paired.contains(&(pitch % 12)) {
        2
    } else {
        1
    }
}

pub fn find_chromatic_runs(pitches: &[Option<u8>], onsets: &[f64]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut index = 0;
    while index < pitches.len() {
        let Some(first) = pitches[index] else {
            index += 1;
            continue;
        };
        let mut end = index + 1;
        let mut direction = 0i32;
        let mut previous = first;
        while end < pitches.len() {
            let Some(pitch) = pitches[end] else { break };
            let step = pitch as i32 - previous as i32;
            if step.abs() != 1 || (direction != 0 && step.signum() != direction) {
                break;
            }
            direction = step.signum();
            previous = pitch;
            end += 1;
        }
        for (from, to) in split_on_gaps(index, end, onsets) {
            if to - from >= MIN_CHROMATIC_RUN {
                runs.push((from, to));
            }
        }
        index = if end - index > 1 { end } else { index + 1 };
    }
    runs
}

pub fn find_arpeggio_runs(pitches: &[Option<u8>], onsets: &[f64]) -> Vec<(usize, usize, u8, u8)> {
    let mut runs = Vec::new();
    let mut index = 0;
    while index + 1 < pitches.len() {
        let (Some(root), Some(next)) = (pitches[index], pitches[index + 1]) else {
            index += 1;
            continue;
        };
        let step = next as i32 - root as i32;
        let third = match step {
            4 | 3 => step as u8,
            -5 => {
                match pitches.get(index + 2).copied().flatten().map(|p| next as i32 - p as i32) {
                    Some(t @ 3) | Some(t @ 4) => 7 - t as u8,
                    _ => {
                        index += 1;
                        continue;
                    }
                }
            }
            _ => {
                index += 1;
                continue;
            }
        };
        let direction = step.signum();
        let chord = [0, third, 7];
        let mut end = index + 1;
        let mut previous = root;
        while end < pitches.len() {
            let Some(pitch) = pitches[end] else { break };
            let step = pitch as i32 - previous as i32;
            let class = (pitch as i32 - root as i32).rem_euclid(12) as u8;
            if !(3..=5).contains(&step.abs()) || step.signum() != direction || !chord.contains(&class) {
                break;
            }
            previous = pitch;
            end += 1;
        }
        while end > index && (pitches[end - 1].unwrap_or(root) as i32 - root as i32).rem_euclid(12) != 0 {
            end -= 1;
        }
        for (from, to) in split_on_gaps(index, end, onsets) {
            let ends_on_roots = [from, to - 1].iter().all(|&i| pitches[i].is_some_and(|p| (p + 12 - root % 12) % 12 == 0));
            if to - from >= MIN_ARPEGGIO_RUN && ends_on_roots {
                runs.push((from, to, root % 12, third));
            }
        }
        index = if end > index + 1 { end - 1 } else { index + 1 };
    }
    runs
}

fn arpeggio_finger(hand: Hand, class: u8) -> Option<u8> {
    let (root, third, fifth) = match hand {
        Hand::Right => (1, 2, 3),
        Hand::Left => (1, 4, 2),
    };
    match class {
        0 => Some(root),
        3 | 4 => Some(third),
        7 => Some(fifth),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleRun {
    pub start: usize,
    pub length: usize,
    pub tonic: u8,
    pub mode: Mode,
}

impl ScaleRun {
    pub fn end(&self) -> usize {
        self.start + self.length
    }
}

fn in_major(pitch: u8, tonic: u8) -> bool {
    MAJOR_DEGREES.contains(&((pitch + 12 - tonic % 12) % 12))
}

fn degree_of(pitch: u8, tonic: u8, mode: Mode) -> Option<usize> {
    let offset = (pitch + 12 - tonic % 12) % 12;
    mode.degrees().iter().position(|d| *d == offset)
}

fn accidentals(tonic: u8) -> u8 {
    const COUNT: [u8; 12] = [0, 5, 2, 3, 4, 1, 6, 1, 4, 3, 2, 5];
    COUNT[tonic as usize]
}

pub fn key_of(pitches: &[u8]) -> Option<u8> {
    let mut best: Option<(u8, (u8, bool))> = None;
    for tonic in 0..12u8 {
        if !pitches.iter().all(|p| in_major(*p, tonic)) {
            continue;
        }
        let touches_tonic = pitches.iter().any(|p| p % 12 == tonic);
        let score = (accidentals(tonic), !touches_tonic);
        if best.as_ref().is_none_or(|(_, existing)| score < *existing) {
            best = Some((tonic, score));
        }
    }
    best.map(|(tonic, _)| tonic)
}

const AUGMENTED_SECOND: i32 = 3;

fn harmonic_minor_of(pitches: &[u8]) -> Option<u8> {
    let mut tonic = None;
    for pair in pitches.windows(2) {
        let step = pair[1] as i32 - pair[0] as i32;
        if step.abs() != AUGMENTED_SECOND {
            continue;
        }
        let named = (pair[0].min(pair[1]) + 4) % 12;
        if tonic.is_some_and(|t| t != named) {
            return None;
        }
        tonic = Some(named);
    }
    let tonic = tonic?;
    pitches
        .iter()
        .all(|p| HARMONIC_MINOR_DEGREES.contains(&((p + 12 - tonic) % 12)))
        .then_some(tonic)
}

fn read_key(notes: &[u8]) -> Option<(u8, Mode)> {
    let leaps = notes
        .windows(2)
        .any(|pair| (pair[1] as i32 - pair[0] as i32).abs() == AUGMENTED_SECOND);
    if leaps {
        harmonic_minor_of(notes).map(|tonic| (tonic, Mode::HarmonicMinor))
    } else {
        key_of(notes).map(|tonic| (tonic, Mode::Major))
    }
}

fn anchored_elsewhere(notes: &[u8], tonic: u8) -> bool {
    match (notes.first(), notes.last()) {
        (Some(first), Some(last)) => first % 12 == last % 12 && first % 12 != tonic,
        _ => false,
    }
}

fn stepwise_stretches(pitches: &[Option<u8>]) -> Vec<(usize, usize, i32)> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < pitches.len() {
        let Some(mut previous) = pitches[index] else {
            index += 1;
            continue;
        };
        let mut end = index + 1;
        let mut direction = 0i32;
        while end < pitches.len() {
            let Some(pitch) = pitches[end] else { break };
            let step = pitch as i32 - previous as i32;
            if !(1..=2).contains(&step.abs()) {
                break;
            }
            if direction == 0 {
                direction = step.signum();
            } else if step.signum() != direction {
                break;
            }
            previous = pitch;
            end += 1;
        }
        out.push((index, end, direction));
        index = if end - index > 1 { end } else { index + 1 };
    }
    out
}

fn join_augmented_seconds(
    pitches: &[Option<u8>],
    stretches: &[(usize, usize, i32)],
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < stretches.len() {
        let (start, mut end, mut direction) = stretches[i];
        let mut j = i + 1;
        while let Some(&(next_start, next_end, next_direction)) = stretches.get(j) {
            let (Some(last), Some(first)) = (pitches[end - 1], pitches[next_start]) else {
                break;
            };
            let step = first as i32 - last as i32;
            let joins = next_start == end
                && step.abs() == AUGMENTED_SECOND
                && (direction == 0 || step.signum() == direction)
                && (next_direction == 0 || step.signum() == next_direction);
            if !joins {
                break;
            }
            let notes: Vec<u8> = pitches[start..next_end].iter().flatten().copied().collect();
            if harmonic_minor_of(&notes).is_none() {
                break;
            }
            end = next_end;
            direction = step.signum();
            j += 1;
        }
        out.push((start, end));
        i = j;
    }
    out
}

pub fn find_scale_runs(pitches: &[Option<u8>], onsets: &[f64]) -> Vec<ScaleRun> {
    let mut runs = Vec::new();
    let stretches = stepwise_stretches(pitches);
    for (start, end) in join_augmented_seconds(pitches, &stretches) {
        for (from, to) in split_on_gaps(start, end, onsets) {
            let length = to - from;
            if length < MIN_SCALE_RUN {
                continue;
            }
            let notes: Vec<u8> = pitches[from..to].iter().flatten().copied().collect();
            if let Some((tonic, mode)) = read_key(&notes) {
                if anchored_elsewhere(&notes, tonic) {
                } else {
                    runs.push(ScaleRun { start: from, length, tonic, mode });
                }
            }
        }
    }
    runs
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Taught {
    pub finger: Finger,
    pub bonus: f32,
}

fn teach_turns(hand: Hand, pitches: &[Option<u8>], runs: &[ScaleRun], out: &mut HashMap<usize, Taught>) {
    for pair in runs.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let turn = a.end() - 1;
        if b.start != a.end() || a.tonic != b.tonic || a.mode != b.mode || turn == 0 {
            continue;
        }
        let (Some(first), Some(top)) = (pitches[a.start], pitches[turn]) else { continue };
        let Some(before) = out.get(&(turn - 1)).map(|t| t.finger.number()) else { continue };
        let towards_little = (top > first) == (hand == Hand::Right);
        let finger = if towards_little { before + 1 } else { before.wrapping_sub(1) };
        if let Some(finger) = Finger::from_number(finger) {
            out.insert(turn, Taught { finger, bonus: SCALE_BONUS });
        }
        let (Some(table), Some(back)) = (b.mode.fingering(b.tonic), pitches[b.start]) else { continue };
        let pattern = if hand == Hand::Right { &table.right } else { &table.left };
        if let Some(finger) = degree_of(back, b.tonic, b.mode).and_then(|d| Finger::from_number(pattern[d])) {
            out.entry(b.start).or_insert(Taught { finger, bonus: SCALE_BONUS });
        }
    }
}

pub fn scale_fingerings(
    hand: Hand,
    pitches: &[Option<u8>],
    onsets: &[f64],
) -> HashMap<usize, Taught> {
    let mut out = HashMap::new();
    let runs = find_scale_runs(pitches, onsets);
    for run in &runs {
        let Some(table) = run.mode.fingering(run.tonic) else {
            continue;
        };
        let pattern = match hand {
            Hand::Right => &table.right,
            Hand::Left => &table.left,
        };
        for index in (run.start + 1)..(run.end() - 1) {
            let Some(pitch) = pitches[index] else { continue };
            let Some(degree) = degree_of(pitch, run.tonic, run.mode) else {
                continue;
            };
            if let Some(finger) = Finger::from_number(pattern[degree]) {
                out.insert(index, Taught { finger, bonus: SCALE_BONUS });
            }
        }
    }
    teach_turns(hand, pitches, &runs, &mut out);
    for (start, end, root, third) in find_arpeggio_runs(pitches, onsets) {
        if [0, third, 7].iter().any(|&step| is_black(root + step)) {
            continue;
        }
        for index in (start + 1)..(end - 1) {
            let Some(pitch) = pitches[index] else { continue };
            let class = (pitch + 12 - root) % 12;
            if let Some(finger) = arpeggio_finger(hand, class).and_then(Finger::from_number) {
                out.insert(index, Taught { finger, bonus: ARPEGGIO_BONUS });
            }
        }
    }
    for (start, end) in find_chromatic_runs(pitches, onsets) {
        for index in (start + 1)..(end - 1) {
            let Some(pitch) = pitches[index] else { continue };
            if let Some(finger) = Finger::from_number(chromatic_finger(hand, pitch)) {
                out.insert(index, Taught { finger, bonus: CHROMATIC_BONUS });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(pitches: &[u8]) -> Vec<Option<u8>> {
        pitches.iter().map(|p| Some(*p)).collect()
    }

    fn even(n: usize) -> Vec<f64> {
        (0..n).map(|i| i as f64 * 0.25).collect()
    }

    fn scale(tonic: u8, octaves: usize) -> Vec<Option<u8>> {
        let mut out = Vec::new();
        for octave in 0..octaves {
            for degree in MAJOR_DEGREES {
                out.push(Some(tonic + degree + 12 * octave as u8));
            }
        }
        out.push(Some(tonic + 12 * octaves as u8));
        out
    }

    #[test]
    fn a_scale_that_turns_back_is_taught_at_its_turn() {
        let mut notes = scale(65, 2);
        let down: Vec<Option<u8>> = notes.iter().rev().skip(1).copied().collect();
        notes.extend(down);
        let taught = scale_fingerings(Hand::Right, &notes, &even(notes.len()));
        assert_eq!(taught.get(&14).map(|t| t.finger.number()), Some(4), "the top F");
        assert_eq!(taught.get(&15).map(|t| t.finger.number()), Some(3), "the E straight after");
        let mut notes = scale(60, 2);
        let down: Vec<Option<u8>> = notes.iter().rev().skip(1).copied().collect();
        notes.extend(down);
        let taught = scale_fingerings(Hand::Right, &notes, &even(notes.len()));
        assert_eq!(taught.get(&14).map(|t| t.finger.number()), Some(5), "the top C");
    }

    #[test]
    fn a_scale_is_recognised_and_its_key_identified() {
        let notes = scale(60, 2);
        let runs = find_scale_runs(&notes, &even(notes.len()));
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].length, 15);
        assert_eq!(runs[0].tonic, 0, "should be C major");
    }

    #[test]
    fn keys_with_accidentals_are_identified_too() {
        for tonic in 0..12u8 {
            let notes = scale(60 + tonic, 1);
            let runs = find_scale_runs(&notes, &even(notes.len()));
            assert_eq!(runs.len(), 1, "scale on {tonic} not found");
            assert_eq!(runs[0].tonic, tonic, "scale on {tonic} misidentified");
        }
    }

    #[test]
    fn a_two_octave_arpeggio_is_found_up_and_down_but_an_inversion_is_not() {
        let up = seq(&[48, 52, 55, 60, 64, 67, 72]);
        assert_eq!(find_arpeggio_runs(&up, &even(7)), vec![(0, 7, 0, 4)]);
        let down = seq(&[81, 76, 72, 69, 64, 60, 57]);
        assert_eq!(find_arpeggio_runs(&down, &even(7)), vec![(0, 7, 9, 3)], "A minor, coming down");
        let inversion = seq(&[52, 55, 60, 64, 67, 72, 76]);
        assert!(find_arpeggio_runs(&inversion, &even(7)).is_empty(), "starting on the third is another fingering");
        let one_octave = seq(&[48, 52, 55, 60]);
        assert!(find_arpeggio_runs(&one_octave, &even(4)).is_empty(), "books differ within one octave");
    }

    #[test]
    fn a_few_adjacent_notes_are_not_a_scale() {
        assert!(find_scale_runs(&seq(&[60, 62, 64, 65]), &even(16)).is_empty());
    }

    #[test]
    fn an_arpeggio_is_not_a_scale() {
        assert!(find_scale_runs(&seq(&[60, 64, 67, 72, 76, 79]), &even(16)).is_empty());
    }

    #[test]
    fn a_chromatic_run_is_not_a_major_scale() {
        assert!(find_scale_runs(&seq(&[60, 61, 62, 63, 64, 65, 66]), &even(16)).is_empty());
    }

    #[test]
    fn a_run_that_turns_round_is_two_runs_not_one() {
        let up = [60u8, 62, 64, 65, 67, 69, 71, 72, 74, 76, 77, 79, 81, 83, 84];
        let mut both: Vec<u8> = up.to_vec();
        both.extend(up.iter().rev().skip(1).copied());
        let up_then_down = seq(&both);
        let runs = find_scale_runs(&up_then_down, &even(up_then_down.len()));
        assert_eq!(runs.len(), 2, "{runs:?}");
    }

    #[test]
    fn the_right_hand_gets_the_published_c_major_fingering() {
        let notes = scale(60, 2);
        let map = scale_fingerings(Hand::Right, &notes, &even(notes.len()));
        let fingers: Vec<u8> = (1..notes.len() - 1).map(|i| map[&i].finger.number()).collect();
        assert_eq!(fingers, vec![2, 3, 1, 2, 3, 4, 1, 2, 3, 1, 2, 3, 4]);
    }

    #[test]
    fn the_left_hand_gets_the_published_c_major_fingering() {
        let notes = scale(60, 2);
        let map = scale_fingerings(Hand::Left, &notes, &even(notes.len()));
        let fingers: Vec<u8> = (1..notes.len() - 1).map(|i| map[&i].finger.number()).collect();
        assert_eq!(fingers, vec![4, 3, 2, 1, 3, 2, 1, 4, 3, 2, 1, 3, 2]);
    }

    fn every_table() -> Vec<(Mode, u8, &'static ScaleFingering)> {
        [Mode::Major, Mode::HarmonicMinor]
            .into_iter()
            .flat_map(|mode| (0..12u8).map(move |tonic| (mode, tonic)))
            .filter_map(|(mode, tonic)| Some((mode, tonic, mode.fingering(tonic)?)))
            .collect()
    }

    #[test]
    fn the_thumb_never_lands_on_a_black_key_in_any_tabulated_scale() {
        for (mode, tonic, table) in every_table() {
            for (pattern, hand) in [(&table.right, Hand::Right), (&table.left, Hand::Left)] {
                for (degree, finger) in pattern.iter().enumerate() {
                    if *finger != 1 {
                        continue;
                    }
                    let pitch = (tonic + mode.degrees()[degree]) % 12;
                    assert!(
                        !on_hand::keyboard::is_black(60 + pitch),
                        "{hand:?} puts the thumb on a black key in the {mode:?} scale on {tonic}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_pattern_passes_the_thumb_twice_an_octave_and_saves_finger_five() {
        for (_, tonic, table) in every_table() {
            for pattern in [&table.right, &table.left] {
                assert!(pattern.iter().all(|f| (1..=5).contains(f)));
                assert!(
                    !pattern.contains(&5),
                    "the scale on {tonic} uses finger 5 mid-run; that belongs to the ends"
                );
                assert_eq!(
                    pattern.iter().filter(|f| **f == 1).count(),
                    2,
                    "the scale on {tonic} should pass the thumb twice per octave"
                );
            }
        }
    }

    #[test]
    fn f_major_crosses_after_the_fourth_finger_in_the_right_hand() {
        assert_eq!(MAJOR_SCALES[5].right, [1, 2, 3, 4, 1, 2, 3]);
    }

    fn harmonic_minor_up_and_down(tonic: u8, octaves: usize) -> Vec<Option<u8>> {
        let mut up = Vec::new();
        for octave in 0..octaves {
            for degree in HARMONIC_MINOR_DEGREES {
                up.push(tonic + degree + 12 * octave as u8);
            }
        }
        up.push(tonic + 12 * octaves as u8);
        let mut both = up.clone();
        both.extend(up.iter().rev().skip(1));
        seq(&both)
    }

    #[test]
    fn a_harmonic_minor_scale_is_one_run_in_its_own_key() {
        let notes = harmonic_minor_up_and_down(64, 2);
        let runs = find_scale_runs(&notes, &even(notes.len()));
        assert_eq!(runs.len(), 2, "one run up and one down: {runs:?}");
        assert_eq!((runs[0].start, runs[0].length), (0, 15), "{runs:?}");
        assert_eq!(runs[1].end(), notes.len(), "{runs:?}");
        for run in &runs {
            assert_eq!((run.tonic, run.mode), (4, Mode::HarmonicMinor), "{runs:?}");
        }
    }

    #[test]
    fn e_harmonic_minor_takes_the_e_major_pattern_both_ways() {
        let notes = harmonic_minor_up_and_down(64, 2);
        let onsets = even(notes.len());
        for (hand, up) in [
            (Hand::Right, [1u8, 2, 3, 1, 2, 3, 4, 1, 2, 3, 1, 2, 3, 4, 5]),
            (Hand::Left, [5u8, 4, 3, 2, 1, 3, 2, 1, 4, 3, 2, 1, 3, 2, 1]),
        ] {
            let mut wanted = up.to_vec();
            wanted.extend(up.iter().rev().skip(1));
            let map = scale_fingerings(hand, &notes, &onsets);
            assert!(!map.is_empty(), "{hand:?} was taught nothing");
            for (index, taught) in &map {
                assert_eq!(
                    taught.finger.number(),
                    wanted[*index],
                    "{hand:?} at {index}, pitch {:?}",
                    notes[*index]
                );
            }
        }
    }

    #[test]
    fn a_black_key_harmonic_minor_is_not_taught_a_major_fragment() {
        let notes = harmonic_minor_up_and_down(61, 2);
        let runs = find_scale_runs(&notes, &even(notes.len()));
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert!(runs.iter().all(|r| r.tonic == 1 && r.mode == Mode::HarmonicMinor), "{runs:?}");
        assert!(scale_fingerings(Hand::Right, &notes, &even(notes.len())).is_empty());
    }

    #[test]
    fn a_minor_third_that_is_not_an_augmented_second_still_ends_a_run() {
        let notes = seq(&[60, 62, 64, 67, 69, 71, 72, 74, 76]);
        let runs = find_scale_runs(&notes, &even(notes.len()));
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!((runs[0].start, runs[0].mode), (3, Mode::Major), "{runs:?}");
    }

    #[test]
    fn the_chromatic_scale_is_three_on_the_black_keys() {
        let line: Vec<Option<u8>> = (60..=72).map(Some).collect();
        let right = scale_fingerings(Hand::Right, &line, &even(line.len()));
        let wanted = [1u8, 3, 1, 3, 1, 2, 3, 1, 3, 1, 3, 1, 2];
        for index in 1..line.len() - 1 {
            assert_eq!(
                right.get(&index).map(|t| t.finger.number()),
                Some(wanted[index]),
                "right hand at {index}"
            );
        }
        let left = scale_fingerings(Hand::Left, &line, &even(line.len()));
        let wanted = [1u8, 3, 1, 3, 2, 1, 3, 1, 3, 1, 3, 2, 1];
        for index in 1..line.len() - 1 {
            assert_eq!(
                left.get(&index).map(|t| t.finger.number()),
                Some(wanted[index]),
                "left hand at {index}"
            );
        }
    }

    #[test]
    fn a_rest_in_the_middle_ends_the_run() {
        let notes = seq(&[60, 62, 64, 65, 67, 69, 71, 72, 74, 76]);

        assert_eq!(find_scale_runs(&notes, &even(notes.len())).len(), 1);

        let mut broken = even(notes.len());
        for onset in broken.iter_mut().skip(5) {
            *onset += 4.0;
        }
        assert!(
            find_scale_runs(&notes, &broken).is_empty(),
            "a stepwise motif either side of a rest is not a scale"
        );
    }

    #[test]
    fn a_five_finger_position_is_not_a_scale() {
        for start in [60u8, 62, 64, 65, 67] {
            let five = seq(&[start, start + 2, start + 4, start + 5, start + 7]);
            assert!(
                find_scale_runs(&five, &even(five.len())).is_empty(),
                "five notes under a still hand should not be a scale run"
            );
            assert!(
                scale_fingerings(Hand::Right, &five, &even(five.len())).is_empty(),
                "and so should attract no taught fingering"
            );
        }
    }

    #[test]
    fn the_clock_is_read_over_the_whole_run_not_the_prefix() {
        let notes = seq(&[60, 62, 64, 65, 67, 69, 71]);
        assert!(
            find_scale_runs(&notes, &[0.0, 4.0, 8.0, 8.25, 8.5, 8.75, 9.0]).is_empty(),
            "a run cannot begin with two four-second steps"
        );

        assert!(
            find_scale_runs(&notes, &[0.0, 3.0, 5.5, 7.5, 9.0, 10.0, 10.5]).is_empty(),
            "stepwise and evenly slow is a series of events, not a scale"
        );

        let long = seq(&[60, 62, 64, 65, 67, 69, 71, 72, 74, 76, 77, 79, 81, 83, 84]);
        let mut onsets = vec![0.0];
        let mut gap = 0.2;
        for _ in 1..long.len() {
            onsets.push(onsets.last().unwrap() + gap);
            gap *= 1.2;
        }
        let runs = find_scale_runs(&long, &onsets);
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert!(
            runs[0].length >= long.len() - 1,
            "a ritardando should cost at most the last note, kept {} of {}",
            runs[0].length,
            long.len()
        );
    }

    #[test]
    fn a_chromatic_fragment_is_shorter_than_a_scale_fragment() {
        let five: Vec<Option<u8>> = (60..65).map(Some).collect();
        let onsets = even(five.len());
        assert_eq!(find_chromatic_runs(&five, &onsets).len(), 1);
        assert!(
            !scale_fingerings(Hand::Right, &five, &onsets).is_empty(),
            "a five-note chromatic fragment should take the taught fingering"
        );
        assert!(MIN_CHROMATIC_RUN < MIN_SCALE_RUN);
    }
}
