//! Standard scale fingerings.
//!
//! Scales are the most heavily codified thing in piano technique. Every method book
//! agrees on how each one is fingered, the agreement is centuries old, and a pianist
//! does not work it out — they already know it. An ergonomic model, meanwhile, finds
//! the two-octave C major scale nearly indifferent between putting the thumb on C
//! and F or on C and G, because biomechanically it very nearly is. What settles it
//! is the convention.
//!
//! So this module detects scale passages and supplies the published fingering for
//! them. The tables are transcribed from a standard scale chart rather than derived,
//! and the tests assert the properties that make them right — most importantly that
//! the thumb never lands on a black key, which is the constraint the whole chart
//! exists to satisfy.

use std::collections::HashMap;

use on_hand::{Finger, Hand};

/// Bonus for a note that takes the finger the standard fingering gives it.
///
/// This was 0.9 for a long time, on the reasoning that the major-scale conventions
/// settle something the ergonomic model finds nearly indifferent and so need only a
/// nudge. The `scalebench` example says otherwise: at 0.9 the engine still disagreed
/// with the books on Eb, Gb and Ab in the right hand, which are exactly the scales
/// where the model has an opinion of its own — the flat keys, where the thumb has to
/// be kept off the black notes and the model would rather not bother.
///
/// Measured the same way [`CHROMATIC_BONUS`] was: run it up until the answer stops
/// changing. Everything agrees from about 5.4, and nothing above that moves. This sits
/// above that with the same margin the chromatic bonus keeps, and still far below the
/// penalty for a shape the hand cannot make, so the convention can win an argument
/// without ever making the hand do something impossible.
pub const SCALE_BONUS: f32 = 7.5;

/// Bonus for a note of a chromatic run that takes the standard finger.
///
/// Much larger than [`SCALE_BONUS`], and for a reason. The major-scale conventions
/// settle something the ergonomic model finds nearly indifferent; the chromatic one has
/// to overrule it outright. Read note by note, a chromatic scale is most comfortably
/// played by alternating the thumb and the second finger — a shape that crosses nothing
/// over anything, that the model likes a great deal, and that no pianist uses, because
/// it cannot be taken at speed and leaves the hand nowhere to travel. The taught
/// fingering deliberately buys a thumb crossing under the third finger on every other
/// note, and the bonus has to be worth more than the crossing costs.
///
/// Measured: the model holds out until about 9, and anything above that gives the same
/// answer. This sits above that with room to spare, and well below the penalty for a
/// shape the hand cannot make, so it can make the convention win an argument but never
/// make the hand do something impossible.
pub const CHROMATIC_BONUS: f32 = 12.0;

/// How many consecutive stepwise notes it takes before a passage is a scale rather
/// than a few notes that happen to be adjacent.
///
/// Six, which is one more than a hand sitting still can reach without moving.
///
/// At five this caught C-D-E-F-G and gave its interior notes the taught 2-3-1 — a
/// thumb crossing on F — against the 1-2-3-4-5 every teacher writes for a five-finger
/// position. The pattern bonus is large enough to win that argument, so the fragment
/// has to be excluded rather than out-argued.
///
/// Six rather than the octave it is tempting to ask for. Measured on the test pieces:
/// every scale run in them is exactly six notes long, so at seven the taught patterns
/// stop reaching real music altogether — ten runs in one piece become none — while
/// six excludes the five-finger position just as well. The collision is with what one
/// still hand covers, and that is five notes, not eight.
pub const MIN_SCALE_RUN: usize = 6;

/// How much longer than the run's own median a gap has to be before it breaks the run.
///
/// Scale detection reads pitches in event order and nothing else, so without this a
/// run is five stepwise notes whether they occupy half a bar or eight bars either side
/// of a rest, a phrase ending and a change of texture. When that misfires it applies
/// the pattern bonus to every interior note, which is large enough to overrule the
/// whole ergonomic model — so a false positive is expensive and worth being strict
/// about.
///
/// Two and a half lets a run breathe — a scale is rarely metronomic, and the last note
/// of a group is often longer — while still breaking at a rest or a held note.
const RUN_GAP_FACTOR: f64 = 2.5;

/// Whether the step from the previous note to this one is long enough to end a run.
///
/// Judged against the median of the gaps already in the run, which is robust to the
/// one long note a scale often ends on in a way a mean is not. With fewer than two
/// gaps there is nothing to judge against and the run continues.
fn breaks_the_run(gaps: &[f64], next: f64) -> bool {
    if gaps.len() < 2 {
        return false;
    }
    let mut sorted: Vec<f64> = gaps.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    median > 0.0 && next > median * RUN_GAP_FACTOR
}

/// Semitone offsets of the seven degrees of a major scale.
const MAJOR_DEGREES: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];

/// The repeating part of the standard fingering for a major scale.
///
/// These are the *interior* patterns. The first and last notes of a scale get
/// adjusted in practice — a right hand starting B flat major begins on 2 and ends on
/// 4, though every B flat in between takes 4 — so the bonus is applied away from the
/// ends of a run and the ends are left to the rest of the cost function.
struct ScaleFingering {
    /// Finger number for each of the seven degrees, right hand.
    right: [u8; 7],
    /// Finger number for each of the seven degrees, left hand.
    left: [u8; 7],
}

/// Standard fingerings by tonic pitch class, C first.
///
/// The keys fall into a few families: every scale whose tonic takes the thumb shares
/// one pattern, and the flat keys share another, because the thumb has to avoid the
/// black keys and there are only so many ways to arrange that.
const MAJOR_SCALES: [ScaleFingering; 12] = [
    // C
    ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] },
    // D flat
    ScaleFingering { right: [2, 3, 1, 2, 3, 4, 1], left: [3, 2, 1, 4, 3, 2, 1] },
    // D
    ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] },
    // E flat
    ScaleFingering { right: [3, 1, 2, 3, 4, 1, 2], left: [3, 2, 1, 4, 3, 2, 1] },
    // E
    ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] },
    // F
    ScaleFingering { right: [1, 2, 3, 4, 1, 2, 3], left: [1, 4, 3, 2, 1, 3, 2] },
    // F sharp
    ScaleFingering { right: [2, 3, 4, 1, 2, 3, 1], left: [4, 3, 2, 1, 3, 2, 1] },
    // G
    ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] },
    // A flat
    ScaleFingering { right: [3, 4, 1, 2, 3, 1, 2], left: [3, 2, 1, 4, 3, 2, 1] },
    // A
    ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 4, 3, 2, 1, 3, 2] },
    // B flat
    ScaleFingering { right: [4, 1, 2, 3, 1, 2, 3], left: [3, 2, 1, 4, 3, 2, 1] },
    // B
    ScaleFingering { right: [1, 2, 3, 1, 2, 3, 4], left: [1, 3, 2, 1, 4, 3, 2] },
];

/// Whether a pitch is a black key.
fn is_black(pitch: u8) -> bool {
    matches!(pitch % 12, 1 | 3 | 6 | 8 | 10)
}

/// The finger the standard chromatic fingering gives a note.
///
/// Unlike the major scales this is a rule rather than a chart, and every source states
/// it the same way: the third finger takes every black key, and the thumb takes every
/// white one — except where two white keys sit side by side, which happens only at E-F
/// and at B-C, where one of the pair has to give way to the second finger. Which one
/// depends on the hand, and the two are mirror images: the right hand puts the second
/// finger on the upper note of the pair, F and C, and the left hand on the lower, E and
/// B. In each case the second finger sits on the far side from where the thumb has to
/// travel next.
///
/// Because it is a rule about key colour rather than about scale degree, it reads the
/// same going up as coming down, which is why a chromatic scale has one fingering and
/// not two.
fn chromatic_finger(hand: Hand, pitch: u8) -> u8 {
    if is_black(pitch) {
        return 3;
    }
    let paired = match hand {
        Hand::Right => [0, 5],  // C and F
        Hand::Left => [4, 11],  // E and B
    };
    if paired.contains(&(pitch % 12)) {
        2
    } else {
        1
    }
}

/// Find the chromatic runs in a sequence of notes.
///
/// Every step a semitone, all in one direction. Five of those in a row cannot fit any
/// major scale, so this never competes with [`find_scale_runs`] for the same notes.
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
        let mut gaps: Vec<f64> = Vec::new();
        while end < pitches.len() {
            let Some(pitch) = pitches[end] else { break };
            let step = pitch as i32 - previous as i32;
            if step.abs() != 1 || (direction != 0 && step.signum() != direction) {
                break;
            }
            let gap = onsets.get(end).zip(onsets.get(end - 1)).map(|(a, b)| a - b);
            if let Some(gap) = gap {
                if breaks_the_run(&gaps, gap) {
                    break;
                }
                gaps.push(gap);
            }
            direction = step.signum();
            previous = pitch;
            end += 1;
        }
        let length = end - index;
        if length >= MIN_SCALE_RUN {
            runs.push((index, end));
        }
        index = if length > 1 { end } else { index + 1 };
    }
    runs
}

/// A stretch of notes moving by step in one direction, all fitting one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaleRun {
    /// Index of the first note of the run.
    pub start: usize,
    /// Number of notes in it.
    pub length: usize,
    /// Tonic pitch class of the key it fits.
    pub tonic: u8,
}

impl ScaleRun {
    /// One past the last note of the run.
    pub fn end(&self) -> usize {
        self.start + self.length
    }
}

/// Whether a pitch belongs to a major scale.
fn in_major(pitch: u8, tonic: u8) -> bool {
    MAJOR_DEGREES.contains(&((pitch + 12 - tonic % 12) % 12))
}

/// Which degree of the scale a pitch is, if it is in it.
fn degree_of(pitch: u8, tonic: u8) -> Option<usize> {
    let offset = (pitch + 12 - tonic % 12) % 12;
    MAJOR_DEGREES.iter().position(|d| *d == offset)
}

/// How far round the circle of fifths a key sits from C, counting flats and sharps
/// alike. Used to prefer the plainest key that fits a passage.
fn accidentals(tonic: u8) -> u8 {
    const COUNT: [u8; 12] = [0, 5, 2, 3, 4, 1, 6, 1, 4, 3, 2, 5];
    COUNT[tonic as usize]
}

/// Find the key a run of notes fits.
///
/// A run of seven different notes pins the key exactly; a shorter one may fit
/// several, so the tie goes to the key with fewest accidentals, and then to one the
/// run actually touches the tonic of.
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

/// Find the scale runs in a sequence of notes.
///
/// A run moves in one direction by steps of a semitone or a tone and fits one major
/// scale. Chromatic stretches and arpeggios are deliberately not scales: they have
/// their own conventions, and guessing at them would be worse than leaving them to
/// the ergonomic model.
/// Whether a run announces a tonic of its own, and it is not the one its notes imply.
///
/// [`key_of`] can only read pitch content, and pitch content cannot tell a minor scale
/// from its relative major: A natural minor and C major are the same seven notes. Asked
/// about a run of white keys it answers C, every time.
///
/// A run that begins and ends on the same note is telling you what key it is in, and it
/// is worth more than the content. Two octaves from A to A is an A scale whatever its
/// notes happen to spell, and the C major pattern read off by chromatic degree gives it
/// finger 3 on the top A where every method book gives 5 — a fingering nobody plays.
///
/// The minor and modal patterns are not in [`MAJOR_SCALES`], so there is nothing right
/// to substitute. Declining to answer leaves the passage to the rules and the hand
/// model, which is an honest fingering rather than a confidently wrong one.
///
/// Deliberately narrow: a *fragment* that neither begins nor ends on the same note says
/// nothing about its key, and goes on taking the pattern of the key its notes imply,
/// which is the right reading of a passage in that key.
fn anchored_elsewhere(notes: &[u8], tonic: u8) -> bool {
    match (notes.first(), notes.last()) {
        (Some(first), Some(last)) => first % 12 == last % 12 && first % 12 != tonic,
        _ => false,
    }
}

pub fn find_scale_runs(pitches: &[Option<u8>], onsets: &[f64]) -> Vec<ScaleRun> {
    let mut runs = Vec::new();
    let mut index = 0;
    while index < pitches.len() {
        if pitches[index].is_none() {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        let mut direction = 0i32;
        let mut previous = pitches[index].unwrap();
        let mut gaps: Vec<f64> = Vec::new();
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
            // A scale is stepwise in time as well as in pitch. Without this, a motif
            // and its answer eight bars later are one run.
            let gap = onsets.get(end).zip(onsets.get(end - 1)).map(|(a, b)| a - b);
            if let Some(gap) = gap {
                if breaks_the_run(&gaps, gap) {
                    break;
                }
                gaps.push(gap);
            }
            previous = pitch;
            end += 1;
        }

        let length = end - index;
        if length >= MIN_SCALE_RUN {
            let notes: Vec<u8> = pitches[index..end].iter().flatten().copied().collect();
            if let Some(tonic) = key_of(&notes) {
                if anchored_elsewhere(&notes, tonic) {
                    // A scale in some other key than the one its notes suggest. Say
                    // nothing rather than say the wrong thing.
                } else {
                    runs.push(ScaleRun { start: index, length, tonic });
                }
            }
        }
        index = if length > 1 { end } else { index + 1 };
    }
    runs
}

/// What the published fingering gives one note, and what it is worth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Taught {
    /// The finger the convention puts there.
    pub finger: Finger,
    /// What taking it is worth, as a bonus against the rest of the cost function.
    pub bonus: f32,
}

/// The finger the standard fingering gives each note of each scale run, keyed by
/// position in the input sequence.
pub fn scale_fingerings(
    hand: Hand,
    pitches: &[Option<u8>],
    onsets: &[f64],
) -> HashMap<usize, Taught> {
    let mut out = HashMap::new();
    for run in find_scale_runs(pitches, onsets) {
        let table = &MAJOR_SCALES[run.tonic as usize];
        let pattern = match hand {
            Hand::Right => &table.right,
            Hand::Left => &table.left,
        };
        for index in (run.start + 1)..(run.end() - 1) {
            let Some(pitch) = pitches[index] else { continue };
            let Some(degree) = degree_of(pitch, run.tonic) else {
                continue;
            };
            if let Some(finger) = Finger::from_number(pattern[degree]) {
                out.insert(index, Taught { finger, bonus: SCALE_BONUS });
            }
        }
    }
    // After the major scales, since a chromatic reading is the more specific one: it
    // takes every step as a semitone, which no run of a major scale does.
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

    /// Evenly spaced onsets, one per note: a scale played in time, which is the case
    /// the gap rule must never break.
    fn even(n: usize) -> Vec<f64> {
        (0..n).map(|i| i as f64 * 0.25).collect()
    }

    /// An ascending major scale from a tonic, spanning whole octaves.
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
        // Two octaves each way, so both halves clear `MIN_SCALE_RUN` on their own and
        // the test is about the turn rather than about the length.
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

    #[test]
    fn the_thumb_never_lands_on_a_black_key_in_any_major_scale() {
        // This is the constraint the whole chart exists to satisfy, so it is worth
        // asserting against every key rather than trusting the transcription.
        for tonic in 0..12u8 {
            let table = &MAJOR_SCALES[tonic as usize];
            for (pattern, hand) in [(&table.right, Hand::Right), (&table.left, Hand::Left)] {
                for (degree, finger) in pattern.iter().enumerate() {
                    if *finger != 1 {
                        continue;
                    }
                    let pitch = (tonic + MAJOR_DEGREES[degree]) % 12;
                    assert!(
                        !on_hand::keyboard::is_black(60 + pitch),
                        "{hand:?} puts the thumb on a black key in the scale on {tonic}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_pattern_passes_the_thumb_twice_an_octave_and_saves_finger_five() {
        for tonic in 0..12u8 {
            let table = &MAJOR_SCALES[tonic as usize];
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
        // The one well-known exception among the scales whose tonic is a white key.
        assert_eq!(MAJOR_SCALES[5].right, [1, 2, 3, 4, 1, 2, 3]);
    }

    #[test]
    fn the_chromatic_scale_is_three_on_the_black_keys() {
        let line: Vec<Option<u8>> = (60..=72).map(Some).collect();
        let right = scale_fingerings(Hand::Right, &line, &even(line.len()));
        // 1-3-1-3-1-2-3-1-3-1-3-1-2, less the two ends the search settles itself.
        let wanted = [1u8, 3, 1, 3, 1, 2, 3, 1, 3, 1, 3, 1, 2];
        for index in 1..line.len() - 1 {
            assert_eq!(
                right.get(&index).map(|t| t.finger.number()),
                Some(wanted[index]),
                "right hand at {index}"
            );
        }
        // The left hand is the mirror: its second finger takes E and B, not C and F.
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
        // Scale detection reads pitches in event order. Without a clock, a motif and
        // its answer a few bars later are one run, and every interior note of the
        // invention collects a bonus large enough to overrule the ergonomic model.
        // Ten notes, so a break at the midpoint leaves five either side and neither
        // half is long enough to be a scale on its own.
        let notes = seq(&[60, 62, 64, 65, 67, 69, 71, 72, 74, 76]);

        // Played in time: one run, as before.
        assert_eq!(find_scale_runs(&notes, &even(notes.len())).len(), 1);

        // The same pitches with a rest in the middle are two fragments.
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
        // C-D-E-F-G is what a hand sitting still plays 1-2-3-4-5, and it is what every
        // teacher writes. Read as a scale run it collects the taught 2-3-1 instead — a
        // thumb crossing on F — and the bonus is large enough to win that argument, so
        // the fragment has to be excluded rather than out-argued.
        //
        // This is the boundary `SCALE_BONUS` is calibrated against: the constant can be
        // raised to fix a flat key without quietly breaking the five-finger position,
        // because the position is no longer a run at all.
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
}
