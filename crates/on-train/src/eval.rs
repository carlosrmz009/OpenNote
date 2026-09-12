//! Measuring whether the fingering is any good.
//!
//! The measure is agreement with what pianists actually wrote, reported the way the
//! literature reports it so the numbers mean something outside this repository.
//! Nakamura, Saito & Yoshii (2020) define three rates, and all three are needed
//! because fingering has no single right answer:
//!
//! * **general** — agreement with each annotation separately, averaged. The strictest
//!   and the most honest; two people fingering the same piece score about 71% against
//!   each other, so that is the ceiling, not 100%.
//! * **highest** — for each piece, agreement with whichever annotation it matched
//!   best. Says: did it find *an* answer a pianist would recognise?
//! * **soft** — a note counts if any annotation used that finger. Generous, and the
//!   one that moves least, but it catches a model that is right about every note
//!   while disagreeing with each individual annotator somewhere.
//!
//! Reported together, they say different things: general going up with soft flat
//! means the model is settling onto one tradition; soft going up means it is finding
//! fingerings nobody used.

use std::collections::{BTreeMap, HashMap};

use on_fingering::{FingeringOptions, FingeringPrior};
use on_hand::Hand;
use on_score::{Note, NoteId, Score, TICKS_PER_QUARTER};

use crate::corpus::{by_piece, HandLabel, Piece};

/// How well a fingering agreed with the people who wrote one.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MatchRates {
    /// Agreement with each annotation separately, averaged.
    pub general: f32,
    /// Agreement with the best-matching annotation of each piece, averaged.
    pub highest: f32,
    /// Notes matching at least one annotation, over all notes.
    pub soft: f32,
    /// How many notes were compared.
    pub notes: usize,
    /// How many pieces they came from.
    pub pieces: usize,
}

impl std::fmt::Display for MatchRates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "general {:.1}%   highest {:.1}%   soft {:.1}%   ({} notes, {} pieces)",
            self.general * 100.0,
            self.highest * 100.0,
            self.soft * 100.0,
            self.notes,
            self.pieces
        )
    }
}

/// Finger every piece in a set and report how well it agreed.
pub fn evaluate(
    pieces: &[Piece],
    options: &FingeringOptions,
    prior: Option<&dyn FingeringPrior>,
) -> MatchRates {
    let grouped = by_piece(pieces);
    let mut general_total = 0.0;
    let mut general_count = 0usize;
    let mut highest_total = 0.0;
    let mut soft_matched = 0usize;
    let mut soft_notes = 0usize;
    let mut counted_pieces = 0usize;
    let mut notes_compared = 0usize;

    for annotations in grouped.values() {
        // All annotations of one piece are of the same notes, so the first one is
        // enough to reconstruct the score to be fingered.
        let Some(reference) = annotations.first() else {
            continue;
        };
        let (score, keys) = rebuild(reference);
        if keys.is_empty() {
            continue;
        }
        let solution = on_fingering::finger_score_with_prior(&score, options, prior);
        let chosen: HashMap<NoteId, u8> = solution
            .fingerings
            .iter()
            .map(|f| (f.note, f.finger.number()))
            .collect();

        let mut best = 0.0f32;
        // Which fingers any annotator used for each note, for the soft rate.
        let mut acceptable: BTreeMap<usize, Vec<u8>> = BTreeMap::new();

        for annotation in annotations {
            let mut agreed = 0usize;
            let mut compared = 0usize;
            for (index, note) in annotation.notes.iter().enumerate() {
                let Some(id) = keys.get(index) else { continue };
                let Some(ours) = chosen.get(id) else { continue };
                // Only where somebody wrote a finger down. The unannotated notes are
                // in the piece so the engine fingers the real music, but there is
                // nothing to agree or disagree with on them.
                let Some(theirs) = note.finger else { continue };
                compared += 1;
                acceptable.entry(index).or_default().push(theirs);
                if *ours == theirs {
                    agreed += 1;
                }
            }
            if compared == 0 {
                continue;
            }
            let rate = agreed as f32 / compared as f32;
            general_total += rate;
            general_count += 1;
            best = best.max(rate);
            notes_compared = notes_compared.max(compared);
        }
        if general_count == 0 {
            continue;
        }
        highest_total += best;
        counted_pieces += 1;

        for (index, fingers) in acceptable {
            let Some(id) = keys.get(index) else { continue };
            let Some(ours) = chosen.get(id) else { continue };
            soft_notes += 1;
            if fingers.contains(ours) {
                soft_matched += 1;
            }
        }
    }

    MatchRates {
        general: ratio(general_total, general_count),
        highest: ratio(highest_total, counted_pieces),
        soft: ratio(soft_matched as f32, soft_notes),
        notes: soft_notes,
        pieces: counted_pieces,
    }
}

/// A rate, or zero if there was nothing to average.
fn ratio(total: f32, count: usize) -> f32 {
    if count == 0 {
        0.0
    } else {
        total / count as f32
    }
}

/// Rebuild a score from an annotation, without its fingering.
///
/// The corpus keeps onsets in seconds, which is what the solver's tempo-aware terms
/// want; the tick positions are only there because a [`Score`] has them. Returns the
/// score and, aligned with the annotation's own note order, the ids they were given,
/// so the solver's answer can be compared note for note.
/// Rebuild a playable score from a corpus entry.
///
/// How long each note sounds matters as much as when it starts. What a hand is still
/// holding is a constraint on what it can reach for next, so a piece rebuilt with every
/// note the same length is not the piece that was fingered, and measuring against it
/// measures the wrong thing. Entries written before lengths were recorded fall back to
/// an eighth note, which is what this used to assume for everything.
fn rebuild(piece: &Piece) -> (Score, Vec<NoteId>) {
    // A corpus entry keeps its timings in seconds, and a score keeps them in ticks
    // against a tempo. `finalise` recomputes the seconds from the ticks, so the two have
    // to agree or the piece comes out at the wrong speed — and how much time there is
    // between two notes is part of what makes a fingering reachable. One quarter note
    // per second makes the conversion below exact.
    let mut score = Score { tempo: on_score::TempoMap::constant_bpm(60.0), ..Default::default() };
    let mut keys = Vec::with_capacity(piece.notes.len());
    for note in &piece.notes {
        let id = NoteId(score.notes.len() as u32);
        keys.push(id);
        score.notes.push(Note {
            id,
            midi: note.midi,
            // Rounded rather than truncated: a note written at exactly one beat lands
            // a tick early otherwise, and a piece this size accumulates enough of that
            // to move an onset by a couple of milliseconds — which is plenty to turn
            // over a choice the cost function finds nearly even.
            onset: (note.onset * f64::from(TICKS_PER_QUARTER)).round() as i64,
            duration: (note.duration * f64::from(TICKS_PER_QUARTER)).round() as i64,
            onset_seconds: note.onset,
            duration_seconds: note.duration,
            staff: Some(match note.hand {
                HandLabel::Right => 1,
                HandLabel::Left => 2,
            }),
            voice: Some(1),
            hand: Some(Hand::from(note.hand)),
            grace: false,
            chord: false,
            velocity: 80,
            tie: Default::default(),
            given_finger: None,
            // Nothing to write a fingering back onto: this score was rebuilt from a
            // corpus entry, not read from a document.
            source: on_score::SourceRef::Midi { track: 0, event: 0 },
        });
    }
    score.finalise();
    (score, keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::FingeredNote;

    fn piece(name: &str, annotator: &str, fingers: &[(u8, u8)]) -> Piece {
        Piece {
            piece: name.into(),
            annotator: annotator.into(),
            source: "test".into(),
            notes: fingers
                .iter()
                .enumerate()
                .map(|(i, (midi, finger))| FingeredNote {
                    duration: crate::corpus::ASSUMED_DURATION,
                    midi: *midi,
                    onset: i as f64 * 0.25,
                    hand: HandLabel::Right,
                    finger: Some(*finger),
                })
                .collect(),
        }
    }

    /// A C major scale is what the solver already gets right, so agreement with the
    /// textbook fingering should be total.
    #[test]
    fn a_textbook_scale_agrees_completely() {
        let scale = piece(
            "c-major",
            "1",
            &[
                (60, 1),
                (62, 2),
                (64, 3),
                (65, 1),
                (67, 2),
                (69, 3),
                (71, 4),
                (72, 5),
            ],
        );
        let rates = evaluate(&[scale], &FingeringOptions::default(), None);
        assert_eq!(rates.pieces, 1);
        assert!(rates.general > 0.99, "{rates}");
        assert!((rates.soft - rates.general).abs() < 1e-6, "{rates}");
    }

    /// With two annotators who disagree, the soft rate has to be at least the
    /// highest, and the highest at least the general.
    #[test]
    fn the_three_rates_are_ordered() {
        let notes: Vec<(u8, u8)> = vec![(60, 1), (62, 2), (64, 3), (65, 1), (67, 2)];
        let other: Vec<(u8, u8)> = vec![(60, 1), (62, 3), (64, 4), (65, 1), (67, 2)];
        let pieces = vec![
            piece("c-major", "1", &notes),
            piece("c-major", "2", &other),
        ];
        let rates = evaluate(&pieces, &FingeringOptions::default(), None);
        assert!(rates.soft >= rates.highest - 1e-6, "{rates}");
        assert!(rates.highest >= rates.general - 1e-6, "{rates}");
        assert_eq!(rates.pieces, 1);
    }

    /// Nothing to measure is not a crash.
    #[test]
    fn an_empty_set_reports_zero() {
        let rates = evaluate(&[], &FingeringOptions::default(), None);
        assert_eq!(rates, MatchRates::default());
    }
}
