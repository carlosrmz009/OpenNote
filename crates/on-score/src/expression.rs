use std::collections::HashMap;

use crate::{art, Score, SourceRef, Ticks, TICKS_PER_QUARTER};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mood {
    pub name: &'static str,
    pub dynamics: f32,
    pub rubato: f32,
    pub accent: f32,
    pub legato: f32,
    pub staccato: f32,
    pub detach: f32,
    pub tempo: f32,
    pub loudness: f32,
    pub lift: f32,
    pub weight: f32,
    pub softness: f32,
}

impl Mood {
    pub const CALM: Mood = Mood {
        name: "calm",
        dynamics: 0.6,
        rubato: 0.7,
        accent: 0.6,
        legato: 1.2,
        staccato: 1.1,
        detach: 0.0,
        tempo: 0.97,
        loudness: -8.0,
        lift: 0.7,
        weight: 0.8,
        softness: 1.3,
    };
    pub const WARM: Mood = Mood {
        name: "warm",
        dynamics: 1.0,
        rubato: 1.0,
        accent: 1.0,
        legato: 1.0,
        staccato: 1.0,
        detach: 0.0,
        tempo: 1.0,
        loudness: 0.0,
        lift: 1.0,
        weight: 1.0,
        softness: 1.0,
    };
    pub const PASSIONATE: Mood = Mood {
        name: "passionate",
        dynamics: 1.5,
        rubato: 1.4,
        accent: 1.4,
        legato: 1.1,
        staccato: 1.0,
        detach: 0.0,
        tempo: 1.0,
        loudness: 8.0,
        lift: 1.4,
        weight: 1.5,
        softness: 0.8,
    };
    pub const PLAYFUL: Mood = Mood {
        name: "playful",
        dynamics: 1.1,
        rubato: 0.8,
        accent: 1.2,
        legato: 0.8,
        staccato: 0.8,
        detach: 0.15,
        tempo: 1.03,
        loudness: 0.0,
        lift: 1.2,
        weight: 1.0,
        softness: 1.0,
    };

    pub const ALL: [Mood; 4] = [Mood::CALM, Mood::WARM, Mood::PASSIONATE, Mood::PLAYFUL];

    pub fn named(name: &str) -> Option<Mood> {
        Mood::ALL.into_iter().find(|m| m.name.eq_ignore_ascii_case(name))
    }
}

impl Default for Mood {
    fn default() -> Self {
        Mood::WARM
    }
}

const ARCH_VELOCITY: f32 = 8.0;
const MELODY_VELOCITY: f32 = 5.0;
const DOWNBEAT_VELOCITY: f32 = 4.0;
const BEAT_VELOCITY: f32 = 2.0;
const ACCENT_VELOCITY: f32 = 14.0;
const STRONG_ACCENT_VELOCITY: f32 = 22.0;
const HAIRPIN_VELOCITY: f32 = 12.0;
const SLUR_END_VELOCITY: f32 = 6.0;
const SLUR_END_LENGTH: f64 = 0.8;
const STACCATO_LENGTH: f64 = 0.45;
const STACCATISSIMO_LENGTH: f64 = 0.3;
const SHORTEST_SECONDS: f64 = 0.06;
const LEGATO_SECONDS: f64 = 0.025;
const REPEAT_GAP_SECONDS: f64 = 0.012;
const ARCH_TEMPO: f64 = 0.04;
const PHRASE_END_TEMPO: f64 = 0.12;
const FINAL_RITARDANDO: f64 = 0.25;
const FERMATA_TEMPO: f64 = 0.8;
const PHRASE_BARS: usize = 4;

pub fn perform(score: &Score, mood: &Mood, amount: f32) -> Score {
    let mut out = score.clone();
    if amount <= 0.0 || score.notes.is_empty() {
        return out;
    }
    let human = sounds_human(score);
    let k_dyn = amount * mood.dynamics * if human { 0.3 } else { 1.0 };
    let k_time = if human { 0.0 } else { f64::from(amount * mood.rubato) };
    let phrases = phrases(score);
    let place = |tick: Ticks| -> (usize, f64) {
        let i = phrases.partition_point(|(start, _)| *start <= tick).saturating_sub(1);
        let (start, end) = phrases[i];
        (i, ((tick - start) as f64 / (end - start).max(1) as f64).clamp(0.0, 1.0))
    };
    let slur_ends: HashMap<Ticks, ()> = score.marks.slurs.iter().map(|(_, end)| (*end, ())).collect();
    let in_slur = |tick: Ticks| score.marks.slurs.iter().any(|(a, b)| *a <= tick && tick < *b);
    let downbeat = |tick: Ticks| {
        if score.marks.measures.is_empty() {
            tick % (4 * TICKS_PER_QUARTER as Ticks) == 0
        } else {
            score.marks.measures.binary_search(&tick).is_ok()
        }
    };
    let tops: HashMap<(Ticks, u8), u8> = {
        let mut tops = HashMap::new();
        for n in &score.notes {
            let side = n.hand.map_or(u8::from(n.midi >= 60), |h| h as u8);
            let top = tops.entry((n.onset, side)).or_insert(n.midi);
            *top = (*top).max(n.midi);
        }
        tops
    };
    let crowded: HashMap<(Ticks, u8), usize> = {
        let mut count = HashMap::new();
        for n in &score.notes {
            let side = n.hand.map_or(u8::from(n.midi >= 60), |h| h as u8);
            *count.entry((n.onset, side)).or_insert(0) += 1;
        }
        count
    };

    for (index, note) in out.notes.iter_mut().enumerate() {
        let original = &score.notes[index];
        let flags = score.marks.articulation(original.source);
        let (_, u) = place(original.onset);
        let mut delta = ARCH_VELOCITY * k_dyn * ((std::f64::consts::PI * u.powf(0.8)).sin() as f32 - 0.5);
        let side = original.hand.map_or(u8::from(original.midi >= 60), |h| h as u8);
        if crowded[&(original.onset, side)] > 1 && tops[&(original.onset, side)] == original.midi {
            delta += MELODY_VELOCITY * k_dyn;
        }
        if downbeat(original.onset) {
            delta += DOWNBEAT_VELOCITY * k_dyn;
        } else if original.onset % TICKS_PER_QUARTER as Ticks == 0 {
            delta += BEAT_VELOCITY * k_dyn;
        }
        if flags & art::STRONG_ACCENT != 0 {
            delta += STRONG_ACCENT_VELOCITY * amount * mood.accent;
        } else if flags & art::ACCENT != 0 {
            delta += ACCENT_VELOCITY * amount * mood.accent;
        }
        delta += hairpin(score, original.onset, original.velocity) * amount * mood.dynamics;
        if slur_ends.contains_key(&original.onset) {
            delta -= SLUR_END_VELOCITY * k_dyn;
        }
        delta += mood.loudness * amount;
        note.velocity = (f32::from(original.velocity) + delta).round().clamp(1.0, 127.0) as u8;
    }

    out.tempo = shaped_tempo(score, mood, amount, k_time, &phrases);
    out.recompute_seconds();

    let mut next_on_key: HashMap<u8, Ticks> = HashMap::new();
    let mut next_same: Vec<Option<Ticks>> = vec![None; out.notes.len()];
    for i in (0..out.notes.len()).rev() {
        next_same[i] = next_on_key.insert(out.notes[i].midi, out.notes[i].onset);
    }
    let seconds_to_ticks = |at: Ticks, seconds: f64| -> Ticks {
        out.tempo.tick_at(out.tempo.seconds_at(at) + seconds) - at
    };
    let lengths: Vec<Ticks> = out
        .notes
        .iter()
        .enumerate()
        .map(|(i, note)| {
            let flags = score.marks.articulation(note.source);
            let mut length = note.duration as f64;
            if flags & art::STACCATISSIMO != 0 {
                length *= 1.0 - (1.0 - STACCATISSIMO_LENGTH) * f64::from(amount.min(1.0)) * f64::from(mood.staccato.recip());
            } else if flags & art::STACCATO != 0 && flags & art::TENUTO == 0 {
                length *= 1.0 - (1.0 - STACCATO_LENGTH) * f64::from(amount.min(1.0)) * f64::from(mood.staccato.recip());
            } else if slur_ends.contains_key(&note.onset) {
                length *= 1.0 - (1.0 - SLUR_END_LENGTH) * f64::from(amount.min(1.0));
            } else if in_slur(note.onset) {
                length += seconds_to_ticks(note.offset(), LEGATO_SECONDS * f64::from(amount * mood.legato)) as f64;
            } else if flags & art::TENUTO == 0 && mood.detach > 0.0 {
                length *= 1.0 - f64::from(mood.detach * amount.min(1.0));
            }
            let shortest = seconds_to_ticks(note.onset, SHORTEST_SECONDS).min(note.duration) as f64;
            let mut length = length.max(shortest).max(1.0) as Ticks;
            if let Some(next) = next_same[i] {
                let room = next - note.onset - seconds_to_ticks(note.onset, REPEAT_GAP_SECONDS).max(1);
                length = length.min(room.max(1)).max(1);
            }
            length
        })
        .collect();
    for (note, length) in out.notes.iter_mut().zip(lengths) {
        note.duration = length;
    }
    out.recompute_seconds();
    out
}

fn hairpin(score: &Score, tick: Ticks, velocity: u8) -> f32 {
    for (start, end, way) in &score.marks.hairpins {
        if tick < *start || tick > *end {
            continue;
        }
        let through = (tick - start) as f32 / (end - start).max(1) as f32;
        let after = score.marks.dynamics.iter().find(|(at, _)| *at >= *end).map(|(_, level)| *level);
        let reach = match after {
            Some(level) if (i32::from(level) - i32::from(velocity)).signum() == i32::from(*way) => {
                f32::from(level) - f32::from(velocity)
            }
            _ => HAIRPIN_VELOCITY * f32::from(*way),
        };
        return reach * through;
    }
    0.0
}

pub fn phrases(score: &Score) -> Vec<(Ticks, Ticks)> {
    let mut onsets: Vec<Ticks> = score.notes.iter().map(|n| n.onset).collect();
    onsets.sort_unstable();
    onsets.dedup();
    let end = score.notes.iter().map(|n| n.offset()).max().unwrap_or(0).max(1);
    if onsets.is_empty() {
        return vec![(0, end)];
    }
    let mut gaps: Vec<Ticks> = onsets.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_unstable();
    let median = gaps.get(gaps.len() / 2).copied().unwrap_or(TICKS_PER_QUARTER as Ticks).max(1);

    let mut cuts: Vec<Ticks> = Vec::new();
    let mut sounding_until = Ticks::MIN;
    let mut by_onset = score.notes.iter().collect::<Vec<_>>();
    by_onset.sort_by_key(|n| n.onset);
    for note in &by_onset {
        if sounding_until != Ticks::MIN && note.onset - sounding_until >= median / 2 {
            cuts.push(note.onset);
        }
        sounding_until = sounding_until.max(note.offset());
        if score.marks.articulation(note.source) & art::FERMATA != 0 {
            if let Some(after) = onsets.iter().find(|t| **t >= note.offset()) {
                cuts.push(*after);
            }
        }
    }
    for (_, slur_end) in &score.marks.slurs {
        if let Some(after) = onsets.iter().find(|t| **t > *slur_end) {
            cuts.push(*after);
        }
    }
    cuts.push(onsets[0]);
    cuts.sort_unstable();
    cuts.dedup();

    let bar = 4 * TICKS_PER_QUARTER as Ticks;
    let longest = PHRASE_BARS as Ticks * bar;
    let mut out = Vec::new();
    for (i, start) in cuts.iter().enumerate() {
        let stop = cuts.get(i + 1).copied().unwrap_or(end);
        let mut from = *start;
        while stop - from > 2 * longest {
            let mut cut = from + longest;
            if let Some(m) = score.marks.measures.iter().find(|m| **m >= cut) {
                cut = *m;
            }
            if cut >= stop {
                break;
            }
            out.push((from, cut));
            from = cut;
        }
        out.push((from, stop));
    }
    out
}

fn shaped_tempo(score: &Score, mood: &Mood, amount: f32, k_time: f64, phrases: &[(Ticks, Ticks)]) -> crate::TempoMap {
    if k_time <= 0.0 {
        return score.tempo.clone();
    }
    let global = 1.0 + (1.0 / f64::from(mood.tempo) - 1.0) * f64::from(amount.min(1.0));
    let step = (TICKS_PER_QUARTER / 4) as Ticks;
    let end = phrases.last().map_or(0, |(_, e)| *e);
    let fermatas: Vec<(Ticks, Ticks)> = score
        .notes
        .iter()
        .filter(|n| score.marks.articulation(n.source) & art::FERMATA != 0)
        .map(|n| (n.onset, n.offset()))
        .collect();
    let mut tempo = crate::TempoMap::default();
    let mut tick = 0;
    let last = phrases.len().saturating_sub(1);
    while tick <= end {
        let i = phrases.partition_point(|(s, _)| *s <= tick).saturating_sub(1);
        let (s, e) = phrases[i];
        let u = ((tick - s) as f64 / (e - s).max(1) as f64).clamp(0.0, 1.0);
        let mut factor = global * (1.0 - ARCH_TEMPO * k_time * ((std::f64::consts::PI * u).sin() - 0.5));
        let ending = ((u - 0.85) / 0.15).clamp(0.0, 1.0);
        factor *= 1.0 + PHRASE_END_TEMPO * k_time * ending * ending * (3.0 - 2.0 * ending);
        if i == last {
            factor *= 1.0 + FINAL_RITARDANDO * k_time * u * u;
        }
        if fermatas.iter().any(|(a, b)| *a <= tick && tick < *b) {
            factor *= 1.0 + FERMATA_TEMPO * f64::from(amount.min(1.0));
        }
        let base = f64::from(score.tempo.micros_per_quarter_at(tick));
        tempo.insert(tick, (base * factor).round() as u32);
        tick += step;
    }
    tempo
}

fn sounds_human(score: &Score) -> bool {
    if !score.notes.iter().all(|n| matches!(n.source, SourceRef::Midi { .. })) || score.notes.len() < 16 {
        return false;
    }
    let mean = score.notes.iter().map(|n| f64::from(n.velocity)).sum::<f64>() / score.notes.len() as f64;
    let spread = (score.notes.iter().map(|n| (f64::from(n.velocity) - mean).powi(2)).sum::<f64>() / score.notes.len() as f64).sqrt();
    let grid = (TICKS_PER_QUARTER / 8) as Ticks;
    let slack = (grid / 10).max(1);
    let off_grid = score
        .notes
        .iter()
        .filter(|n| {
            let r = n.onset.rem_euclid(grid);
            r > slack && grid - r > slack
        })
        .count();
    spread > 8.0 && off_grid as f64 > 0.3 * score.notes.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Note, NoteId, TieState};

    fn note(i: u32, midi: u8, onset: Ticks, duration: Ticks) -> Note {
        Note {
            id: NoteId(i),
            midi,
            onset,
            duration,
            onset_seconds: 0.0,
            duration_seconds: 0.0,
            staff: None,
            voice: None,
            hand: None,
            tie: TieState::default(),
            grace: false,
            chord: false,
            velocity: 72,
            given_finger: None,
            source: SourceRef::MusicXml { part: 0, measure: 0, element: i as usize },
        }
    }

    fn melody() -> Score {
        let q = TICKS_PER_QUARTER as Ticks;
        let pitches = [60u8, 62, 64, 65, 67, 69, 71, 72, 71, 69, 67, 65, 64, 62, 60, 60];
        let mut score = Score {
            notes: pitches.iter().enumerate().map(|(i, m)| note(i as u32, *m, i as Ticks * q, q)).collect(),
            ..Default::default()
        };
        score.marks.measures = (0..4).map(|b| b * 4 * q).collect();
        score.marks.slurs = vec![(0, 7 * q)];
        score.marks.articulations.insert(score.notes[9].source, art::STACCATO);
        score.marks.articulations.insert(score.notes[11].source, art::ACCENT);
        score.finalise();
        score
    }

    #[test]
    fn no_expression_changes_nothing() {
        let score = melody();
        let same = perform(&score, &Mood::WARM, 0.0);
        assert_eq!(same.notes, score.notes);
        assert_eq!(same.tempo, score.tempo);
    }

    #[test]
    fn marks_are_played_as_written() {
        let score = melody();
        let played = perform(&score, &Mood::WARM, 1.0);
        assert!(played.notes[9].duration < score.notes[9].duration / 2 + 1, "staccato is short");
        assert!(played.notes[11].velocity > played.notes[10].velocity + 8, "the accent stands out");
        assert!(played.notes[7].duration < score.notes[7].duration, "a slur ends lifted");
        assert!(played.notes[3].offset() > played.notes[4].onset, "a slurred line overlaps");
    }

    #[test]
    fn the_piece_breathes_and_slows_at_the_end() {
        let score = melody();
        let played = perform(&score, &Mood::WARM, 1.0);
        let last = score.notes.len() - 1;
        let gap = |s: &Score| s.notes[last].onset_seconds - s.notes[last - 1].onset_seconds;
        assert!(gap(&played) > gap(&score) * 1.05, "{} vs {}", gap(&played), gap(&score));
        assert_eq!(perform(&score, &Mood::WARM, 1.0).notes, played.notes, "the same every time");
    }

    #[test]
    fn moods_differ_in_loudness_and_pace() {
        let score = melody();
        let calm = perform(&score, &Mood::CALM, 1.0);
        let passionate = perform(&score, &Mood::PASSIONATE, 1.0);
        let loudness = |s: &Score| s.notes.iter().map(|n| u32::from(n.velocity)).sum::<u32>();
        assert!(loudness(&passionate) > loudness(&calm));
        assert!(calm.notes.last().unwrap().onset_seconds > score.notes.last().unwrap().onset_seconds);
    }

    #[test]
    fn a_human_performance_keeps_its_own_timing() {
        let mut score = melody();
        for (i, n) in score.notes.iter_mut().enumerate() {
            n.source = SourceRef::Midi { track: 0, event: i };
            n.onset += (i as Ticks * 7) % 23;
            n.velocity = 50 + ((i * 37) % 50) as u8;
        }
        score.marks = Default::default();
        score.finalise();
        let played = perform(&score, &Mood::PASSIONATE, 1.0);
        let onsets = |s: &Score| s.notes.iter().map(|n| n.onset_seconds).collect::<Vec<_>>();
        assert_eq!(onsets(&played), onsets(&score));
    }
}
