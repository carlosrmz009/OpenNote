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
//! * **recombined** — agreement with the best sequence that can be stitched together
//!   out of the annotations, charging for each stitch. See [`recombined`].
//!
//! Reported together, they say different things: general going up with soft flat
//! means the model is settling onto one tradition; soft going up means it is finding
//! fingerings nobody used. Recombined going up while soft stays put means the fingering
//! is becoming more *coherent* — the same choices in the same order somebody made —
//! rather than merely more often locally defensible.

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
    /// Agreement with the cheapest recombination of the annotations.
    pub recombined: f32,
    /// How many notes were compared.
    pub notes: usize,
    /// How many pieces they came from.
    pub pieces: usize,
}

impl std::fmt::Display for MatchRates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "general {:.1}%   highest {:.1}%   soft {:.1}%   recombined {:.1}%   \
             ({} notes, {} pieces)",
            self.general * 100.0,
            self.highest * 100.0,
            self.soft * 100.0,
            self.recombined * 100.0,
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
    let mut recombined_total = 0.0;
    let mut recombined_pieces = 0usize;
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
        // The same thing kept per annotator rather than pooled, which is what the
        // recombination needs: it has to know whose choice each finger was.
        let mut truths: Vec<Vec<Option<u8>>> = Vec::new();

        for annotation in annotations {
            let mut agreed = 0usize;
            let mut compared = 0usize;
            let mut row = vec![None; annotation.notes.len()];
            for (index, note) in annotation.notes.iter().enumerate() {
                let Some(id) = keys.get(index) else { continue };
                let Some(ours) = chosen.get(id) else { continue };
                // Only where somebody wrote a finger down. The unannotated notes are
                // in the piece so the engine fingers the real music, but there is
                // nothing to agree or disagree with on them.
                let Some(theirs) = note.finger else { continue };
                compared += 1;
                acceptable.entry(index).or_default().push(theirs);
                row[index] = Some(theirs);
                if *ours == theirs {
                    agreed += 1;
                }
            }
            if compared == 0 {
                continue;
            }
            truths.push(row);
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

        // The recombination needs every annotator to have an opinion at every note it
        // walks through: following one of them across a note they left blank is not
        // defined. So it runs on the notes they all filled in, which for a fully
        // annotated corpus is all of them, and for a partial one is the part where the
        // question can be asked at all.
        if let Some(first) = truths.first() {
            let shared: Vec<usize> = (0..first.len())
                .filter(|i| truths.iter().all(|row| row[*i].is_some()))
                .filter(|i| keys.get(*i).and_then(|id| chosen.get(id)).is_some())
                .collect();
            if !shared.is_empty() {
                let rows: Vec<Vec<u8>> = truths
                    .iter()
                    .map(|row| shared.iter().map(|i| row[*i].expect("filtered")).collect())
                    .collect();
                let mine: Vec<u8> = shared
                    .iter()
                    .map(|i| chosen[&keys[*i]])
                    .collect();
                recombined_total += recombined(&rows, &mine);
                recombined_pieces += 1;
            }
        }

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
        recombined: ratio(recombined_total, recombined_pieces),
        notes: soft_notes,
        pieces: counted_pieces,
    }
}

/// Agreement with the best sequence stitched together out of several annotations.
///
/// Nakamura, Saito & Yoshii (2020, sec. 6.2 and Appendix A). The other three rates
/// judge each note on its own, and fingering does not work that way: there are usually
/// several defensible fingers for a note, but choosing one commits the hand to what
/// follows. A model can match some annotator at every single note and still be
/// incoherent, by taking its choices from a different pianist each time.
///
/// So the estimate is compared against a *recombined* reference: a sequence that may
/// follow one annotation for a while and then switch to another, but only where the
/// two agree at the switching point, and paying a cost for each switch. Formally, with
/// `z[n]` the annotation being followed at note `n`:
///
/// * switching costs `C_rec` where the two annotations agree at that note, and is
///   forbidden where they do not — an abrupt change no pianist made is not evidence of
///   anything;
/// * disagreeing with the recombined reference costs `C_sub` per note.
///
/// The cheapest total is the error, and `M_rec = (N - E_rec) / N`. Both costs are 1,
/// as in the paper: a recombination is charged the same as a local mismatch, so the
/// measure never prefers an implausible stitch to simply being wrong once. Finding the
/// cheapest is a Viterbi over which annotation is being followed.
///
/// `truths` is one row per annotation, all the same length, and `ours` is the
/// estimate. Notes nobody fingered are the caller's to strip: following an annotation
/// through a note it does not fill in is not defined.
///
/// The score can be negative in principle — an estimate that forces a switch at every
/// note costs more than the notes it gets right — so it is clamped at zero, which is
/// what a rate means.
pub fn recombined(truths: &[Vec<u8>], ours: &[u8]) -> f32 {
    let notes = ours.len();
    if truths.is_empty() || notes == 0 {
        return 0.0;
    }
    debug_assert!(truths.iter().all(|t| t.len() == notes));

    const RECOMBINATION: f32 = 1.0;
    const SUBSTITUTION: f32 = 1.0;

    // Cost so far of following each annotation, having just placed note `n`.
    let mut cost: Vec<f32> = truths
        .iter()
        .map(|t| if t[0] == ours[0] { 0.0 } else { SUBSTITUTION })
        .collect();

    for n in 1..notes {
        let previous = cost.clone();
        for (g, truth) in truths.iter().enumerate() {
            // Staying with the same annotation is free; moving to it from another is
            // allowed only where that other one agrees here, and costs a recombination.
            let mut best = previous[g];
            for (h, other) in truths.iter().enumerate() {
                if h != g && other[n] == truth[n] {
                    best = best.min(previous[h] + RECOMBINATION);
                }
            }
            cost[g] = best + if truth[n] == ours[n] { 0.0 } else { SUBSTITUTION };
        }
    }

    let error = cost.iter().copied().fold(f32::INFINITY, f32::min);
    ((notes as f32 - error) / notes as f32).max(0.0)
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


    /// Matching one annotation exactly costs nothing: no switches, no substitutions.
    #[test]
    fn following_one_annotation_all_the_way_is_free() {
        let truths = vec![vec![1, 2, 3, 1, 2], vec![1, 3, 4, 1, 2]];
        assert!((recombined(&truths, &[1, 2, 3, 1, 2]) - 1.0).abs() < 1e-6);
        assert!((recombined(&truths, &[1, 3, 4, 1, 2]) - 1.0).abs() < 1e-6);
    }

    /// The point of the measure, and what separates it from the soft rate.
    ///
    /// Two estimates can match *some* annotator at every single note and still be
    /// quite different things. One changes its mind where the two pianists happen to
    /// agree, which is a fingering a third pianist could have arrived at and played.
    /// The other changes its mind where they disagree, repeatedly, which is not a
    /// fingering at all but a shuffle of two incompatible plans — the hand would have
    /// to be in two places. The soft rate scores both a perfect 100%. This one does
    /// not, and that is the whole reason Nakamura et al. added it.
    #[test]
    fn a_coherent_stitch_beats_a_shuffle() {
        // The two annotators agree only at note 2, so that is the only place a
        // fingering can cross from one to the other.
        let a = vec![1, 2, 5, 3, 3];
        let b = vec![4, 4, 5, 2, 2];
        let truths = vec![a.clone(), b.clone()];

        // Follows the first pianist, then the second, crossing where they agree.
        let coherent = vec![1, 2, 5, 2, 2];
        // Takes whichever finger it likes at each note, crossing four times.
        let shuffled = vec![4, 2, 5, 3, 2];

        // Every note of both matches one of the two, so the soft rate is 100% for
        // each and cannot tell them apart.
        for note in 0..5 {
            assert!(coherent[note] == a[note] || coherent[note] == b[note]);
            assert!(shuffled[note] == a[note] || shuffled[note] == b[note]);
        }

        let stitched = recombined(&truths, &coherent);
        let shuffle = recombined(&truths, &shuffled);
        assert!(
            stitched > shuffle,
            "one crossing scored {stitched}, four scored {shuffle}"
        );
        // One legal crossing and nothing else wrong, out of five notes.
        assert!((stitched - 0.8).abs() < 1e-6, "{stitched}");
    }

    /// A recombination costs the same as being wrong once, so the measure never
    /// prefers an elaborate stitch to a single mistake. This is the paper's choice of
    /// C_rec = C_sub = 1, and it is what stops the reference bending to fit anything.
    #[test]
    fn a_stitch_costs_what_a_mistake_costs() {
        let truths = vec![vec![1, 2, 3, 2, 1], vec![1, 4, 3, 5, 1]];
        // Five notes, one note wrong and no switching needed.
        let one_error = recombined(&truths, &[1, 2, 3, 2, 5]);
        // Five notes, all matching an annotator, but needing one switch at note 2.
        let one_switch = recombined(&truths, &[1, 2, 3, 5, 1]);
        assert!((one_error - one_switch).abs() < 1e-6, "{one_error} vs {one_switch}");
        assert!((one_error - 0.8).abs() < 1e-6, "{one_error}");
    }

    /// With one annotation there is nothing to recombine, so it is the plain match
    /// rate.
    #[test]
    fn one_annotation_gives_the_plain_match_rate() {
        let truths = vec![vec![1, 2, 3, 4]];
        assert!((recombined(&truths, &[1, 2, 3, 4]) - 1.0).abs() < 1e-6);
        assert!((recombined(&truths, &[1, 2, 3, 1]) - 0.75).abs() < 1e-6);
        assert!((recombined(&truths, &[5, 5, 5, 5]) - 0.0).abs() < 1e-6);
    }

    /// Nothing to measure is not a division by zero or a negative rate.
    #[test]
    fn recombining_nothing_is_zero() {
        assert_eq!(recombined(&[], &[1, 2]), 0.0);
        assert_eq!(recombined(&[vec![]], &[]), 0.0);
    }

    /// The recombined rate sits between the highest and the soft ones: it can do at
    /// least as well as staying with the best single annotation, and no better than
    /// taking whatever finger anybody used, since it pays for the changes.
    #[test]
    fn the_recombined_rate_sits_between_the_highest_and_the_soft() {
        let notes: Vec<(u8, u8)> = vec![(60, 1), (62, 2), (64, 3), (65, 1), (67, 2)];
        let other: Vec<(u8, u8)> = vec![(60, 1), (62, 3), (64, 4), (65, 1), (67, 2)];
        let pieces = vec![
            piece("c-major", "1", &notes),
            piece("c-major", "2", &other),
        ];
        let rates = evaluate(&pieces, &FingeringOptions::default(), None);
        assert!(rates.recombined >= rates.highest - 1e-6, "{rates}");
        assert!(rates.soft >= rates.recombined - 1e-6, "{rates}");
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
