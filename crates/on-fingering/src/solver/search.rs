//! The search for one hand: a second-order Viterbi over each chord's possible fingerings,
//! scored by the rules, the hand model and the trained model.

use super::*;

pub(crate) const SUBSTITUTION_COST: f32 = 800.0;

pub(crate) struct HandSolver<'a> {
    pub(crate) hand: Hand,
    pub(crate) rules: RuleScorer,
    pub(crate) biomech: BiomechModel,
    pub(crate) options: &'a FingeringOptions,
    pub(crate) prior: Option<&'a dyn FingeringPrior>,
    pub(crate) agreement: Option<&'a Agreement>,
    pub(crate) scales: std::collections::HashMap<usize, crate::scales::Taught>,
}

impl<'a> HandSolver<'a> {
    pub(crate) fn new(
        hand: Hand,
        options: &'a FingeringOptions,
        prior: Option<&'a dyn FingeringPrior>,
        agreement: Option<&'a Agreement>,
    ) -> Self {
        Self {
            hand,
            rules: RuleScorer::new(
                options.rule_set,
                options.rule_weights,
                options.span_model.table(),
                options.ruler,
            ),
            biomech: BiomechModel::new(options.profile.clone(), hand, options.biomech),
            options,
            prior,
            agreement,
            scales: std::collections::HashMap::new(),
        }
    }

    pub(crate) fn find_scales(&mut self, events: &[Event]) {
        let line: Vec<Option<u8>> =
            events.iter().map(|e| if e.len() == 1 { Some(e.notes[0]) } else { None }).collect();
        let onsets: Vec<f64> = events.iter().map(|e| e.onset_seconds).collect();
        self.scales = crate::scales::scale_fingerings(self.hand, &line, &onsets);
    }

    pub(crate) fn candidates(&self, event: &Event) -> Vec<Candidate> {
        let k = event.len();
        let mut out = Vec::new();
        if k == 0 || k > 5 {
            if k > 5 {
                out.push(Candidate { fingers: spread_fingers(self.hand, k) });
            }
            return out;
        }

        // Fingerings written into the score can ask for the impossible together, such as
        // a held note's finger on the wrong side of a struck one's. Rather than give up on
        // the search, let go of the held notes' fingerings first, then of all of them.
        let struck_only: Vec<Option<Finger>> =
            event.pinned.iter().zip(&event.struck).map(|(pin, struck)| pin.filter(|_| *struck)).collect();
        for pins in [event.pinned.clone(), struck_only, vec![None; k]] {
            let mut chosen = Vec::with_capacity(k);
            self.enumerate(event, &pins, 0, &mut chosen, &mut out);
            if !out.is_empty() {
                break;
            }
        }
        out
    }

    pub(crate) fn enumerate(
        &self,
        event: &Event,
        pins: &[Option<Finger>],
        index: usize,
        chosen: &mut Vec<Finger>,
        out: &mut Vec<Candidate>,
    ) {
        if index == event.len() {
            out.push(Candidate { fingers: chosen.clone() });
            return;
        }
        let remaining = event.len() - index - 1;
        for f in Finger::ALL {
            if let Some(required) = pins[index] {
                if f != required {
                    continue;
                }
            }
            if let Some(previous) = chosen.last() {
                let ordered = match self.hand {
                    Hand::Right => f.index() > previous.index(),
                    Hand::Left => f.index() < previous.index(),
                };
                if !ordered {
                    continue;
                }
            }
            let room = match self.hand {
                Hand::Right => 4 - f.index() >= remaining,
                Hand::Left => f.index() >= remaining,
            };
            if !room {
                continue;
            }
            chosen.push(f);
            self.enumerate(event, pins, index + 1, chosen, out);
            chosen.pop();
        }
    }

    pub(crate) fn grip(&self, event: &Event, candidate: &Candidate) -> Grip {
        Grip::new(event.notes.iter().copied().zip(candidate.fingers.iter().copied()).collect())
    }

    pub(crate) fn outer(&self, event: &Event, candidate: &Candidate) -> (Placement, Placement) {
        let last = event.len() - 1;
        (
            Placement::new(event.notes[0], candidate.fingers[0]),
            Placement::new(event.notes[last], candidate.fingers[last]),
        )
    }

    pub(crate) fn baseline_strain(&self, event: &Event, candidates: &[Candidate]) -> f32 {
        candidates
            .iter()
            .map(|c| self.biomech.grip_outcome(&self.grip(event, c)).strain)
            .fold(f32::INFINITY, f32::min)
    }

    pub(crate) fn event_cost(
        &self,
        index: usize,
        event: &Event,
        candidate: &Candidate,
        baseline: f32,
    ) -> f32 {
        self.event_breakdown(index, event, candidate, baseline).total()
    }

    pub(crate) fn event_breakdown(
        &self,
        index: usize,
        event: &Event,
        candidate: &Candidate,
        baseline: f32,
    ) -> CostBreakdown {
        let mut rules = 0.0;
        let mut agreed = 0.0;
        for i in 0..event.len() {
            let here = Placement::new(event.notes[i], candidate.fingers[i]);
            if let Some(votes) = self.agreement {
                agreed -= votes.bonus(event.ids[i], candidate.fingers[i])
                    + votes.passage(event.ids[i], candidate.fingers[i]);
            }
            rules += self
                .rules
                .score_with(
                    &Trigram { hand: self.hand, prev: None, current: here, next: None },
                    PER_NOTE_RULES,
                )
                .total();
            if i > 0 {
                let below = Placement::new(event.notes[i - 1], candidate.fingers[i - 1]);
                rules += self
                    .rules
                    .score_with(
                        &Trigram { hand: self.hand, prev: Some(below), current: here, next: None },
                        WITHIN_CHORD_RULES,
                    )
                    .total();
            }
        }
        let mut pattern = crate::patterns::chord_bonus(self.hand, &event.notes, &candidate.fingers);
        if let Some(taught) = self.scales.get(&index) {
            if candidate.fingers.first() == Some(&taught.finger) {
                pattern += taught.bonus;
            }
        }
        let pattern = self.options.pattern_scale * pattern;

        let outcome = self.biomech.grip_outcome(&self.grip(event, candidate));
        let mut posture = self.biomech.weights().posture * (outcome.strain - baseline).max(0.0);
        if !outcome.reachable {
            posture += crate::biomech::UNREACHABLE_PENALTY + outcome.shortfall_mm;
        }
        let learned = self.prior.map_or(0.0, |model| {
            model
                .chord_cost(self.hand, &event.notes, &candidate.fingers)
                .clamp(-crate::learned::LIMIT, crate::learned::LIMIT)
        });
        CostBreakdown {
            rules: self.options.rule_scale * rules,
            posture,
            pattern: -pattern,
            motion: 0.0,
            agreement: agreed,
            prior: 0.0,
            learned,
        }
    }

    pub(crate) fn window_cost(
        &self,
        prev: Option<(&Event, &Candidate)>,
        current: (&Event, &Candidate),
        next: Option<(&Event, &Candidate)>,
    ) -> f32 {
        let (low, high) = self.outer(current.0, current.1);
        let prev_outer = prev.map(|(e, c)| self.outer(e, c));
        let next_outer = next.map(|(e, c)| self.outer(e, c));

        let shifting = current.0.len() > 1 && prev.is_some_and(|(event, _)| event.len() > 1);

        let mut total = 0.0;
        for (side, current) in [(0usize, low), (1usize, high)] {
            let t = Trigram {
                hand: self.hand,
                prev: prev_outer.map(|o| if side == 0 { o.0 } else { o.1 }),
                current,
                next: next_outer.map(|o| if side == 0 { o.0 } else { o.1 }),
            };
            let held = t.prev.is_some_and(|p| p.finger == t.current.finger);
            let skip = if shifting && held { SHIFTED_VOICE_RULES } else { PER_NOTE_RULES };
            total += 0.5 * self.rules.score_melodic(&t, skip).total();
        }
        self.options.rule_scale * total
    }

    pub(crate) fn prior_cost(
        &self,
        prev2: Option<(&Event, &Candidate)>,
        prev: Option<(&Event, &Candidate)>,
        current: (&Event, &Candidate),
    ) -> f32 {
        let Some(prior) = self.prior else { return 0.0 };
        let learned = match (prev2, prev) {
            (Some(a), Some(b)) => prior
                .trigram_cost(
                    self.hand,
                    (&a.0.notes, &a.1.fingers),
                    (&b.0.notes, &b.1.fingers),
                    (&current.0.notes, &current.1.fingers),
                )
                .clamp(-crate::learned::LIMIT, crate::learned::LIMIT),
            _ => 0.0,
        };
        if self.options.prior_scale == 0.0 {
            return learned;
        }
        let top = |pair: Option<(&Event, &Candidate)>| pair.map(|(e, c)| self.outer(e, c).1);
        let (_, here) = self.outer(current.0, current.1);
        let logp = prior.log_probability(self.hand, top(prev2), top(prev), here);
        learned - self.options.prior_scale * logp
    }

    pub(crate) fn move_cost(&self, from: (&Event, &Candidate), to: (&Event, &Candidate)) -> f32 {
        let seconds = to.0.onset_seconds - from.0.onset_seconds;
        let travel =
            self.biomech.transition_cost(&self.grip(from.0, from.1), &self.grip(to.0, to.1), seconds);
        let learned = self.prior.map_or(0.0, |model| {
            model
                .step_cost(self.hand, (&from.0.notes, &from.1.fingers), (&to.0.notes, &to.1.fingers), seconds)
                .clamp(-crate::learned::LIMIT, crate::learned::LIMIT)
        });
        travel + SUBSTITUTION_COST * self.substitutions(from, to) as f32 + learned
    }

    pub(crate) fn substitutions(&self, from: (&Event, &Candidate), to: (&Event, &Candidate)) -> usize {
        to.0.ids
            .iter()
            .enumerate()
            .filter(|(slot, id)| {
                !to.0.struck[*slot]
                    && from
                        .0
                        .ids
                        .iter()
                        .position(|earlier| earlier == *id)
                        .is_some_and(|was| from.1.fingers[was] != to.1.fingers[*slot])
            })
            .count()
    }

    pub(crate) fn solve(&self, events: &[Event]) -> Solution {
        let all: Vec<Vec<Candidate>> = events.iter().map(|e| self.candidates(e)).collect();
        let baselines: Vec<f32> = events.iter().zip(&all).map(|(e, c)| self.baseline_strain(e, c)).collect();

        let n = events.len();
        if n == 1 {
            return self.single_event(&events[0], &all[0], baselines[0]);
        }

        let mut best: Vec<Vec<f32>> = Vec::with_capacity(n);
        let mut from: Vec<Vec<usize>> = Vec::with_capacity(n);

        let (c0, c1) = (all[0].len(), all[1].len());
        let mut row = vec![f32::INFINITY; c0 * c1];
        for a in 0..c0 {
            let pa = (&events[0], &all[0][a]);
            let base = self.event_cost(0, &events[0], &all[0][a], baselines[0]);
            for b in 0..c1 {
                let pb = (&events[1], &all[1][b]);
                let mut total = base;
                total += self.event_cost(1, &events[1], &all[1][b], baselines[1]);
                total += self.move_cost(pa, pb);
                total += self.window_cost(None, pa, Some(pb));
                total += self.prior_cost(None, None, pa);
                total += self.prior_cost(None, Some(pa), pb);
                row[a * c1 + b] = total;
            }
        }
        best.push(vec![]);
        best.push(row);
        from.push(vec![]);
        from.push(vec![0; c0 * c1]);

        for i in 2..n {
            let (ca, cb, cc) = (all[i - 2].len(), all[i - 1].len(), all[i].len());
            let mut row = vec![f32::INFINITY; cb * cc];
            let mut back = vec![0usize; cb * cc];
            for b in 0..cb {
                let pb = (&events[i - 1], &all[i - 1][b]);
                for c in 0..cc {
                    let pc = (&events[i], &all[i][c]);
                    let step =
                        self.event_cost(i, &events[i], &all[i][c], baselines[i]) + self.move_cost(pb, pc);
                    for a in 0..ca {
                        let previous = best[i - 1][a * cb + b];
                        if !previous.is_finite() {
                            continue;
                        }
                        let pa = (&events[i - 2], &all[i - 2][a]);
                        let total = previous
                            + step
                            + self.window_cost(Some(pa), pb, Some(pc))
                            + self.prior_cost(Some(pa), Some(pb), pc);
                        let slot = b * cc + c;
                        if total < row[slot] {
                            row[slot] = total;
                            back[slot] = a;
                        }
                    }
                }
            }
            best.push(row);
            from.push(back);
        }

        let (cb, cc) = (all[n - 2].len(), all[n - 1].len());
        let mut best_slot = 0;
        let mut best_cost = f32::INFINITY;
        for b in 0..cb {
            for c in 0..cc {
                let slot = b * cc + c;
                let base = best[n - 1][slot];
                if !base.is_finite() {
                    continue;
                }
                let pb = (&events[n - 2], &all[n - 2][b]);
                let pc = (&events[n - 1], &all[n - 1][c]);
                let total = base + self.window_cost(Some(pb), pc, None);
                if total < best_cost {
                    best_cost = total;
                    best_slot = slot;
                }
            }
        }

        let mut chosen = vec![0usize; n];
        let mut b = best_slot / cc;
        let mut c = best_slot % cc;
        chosen[n - 1] = c;
        chosen[n - 2] = b;
        for i in (2..n).rev() {
            let a = from[i][b * all[i].len() + c];
            chosen[i - 2] = a;
            c = b;
            b = a;
        }

        self.assemble(events, &all, &baselines, &chosen, best_cost)
    }

    pub(crate) fn single_event(&self, event: &Event, candidates: &[Candidate], baseline: f32) -> Solution {
        let mut best = 0;
        let mut best_cost = f32::INFINITY;
        for (i, candidate) in candidates.iter().enumerate() {
            let cost = self.event_cost(0, event, candidate, baseline)
                + self.window_cost(None, (event, candidate), None);
            if cost < best_cost {
                best_cost = cost;
                best = i;
            }
        }
        let events = std::slice::from_ref(event);
        let all = vec![candidates.to_vec()];
        self.assemble(events, &all, &[baseline], &[best], best_cost)
    }

    pub(crate) fn neighbourhood(
        &self,
        events: &[Event],
        all: &[Vec<Candidate>],
        baselines: &[f32],
        chosen: &[usize],
        index: usize,
        here: &Candidate,
    ) -> f32 {
        let at = |j: isize| on_path(events, all, chosen, index, here, j);
        let i = index as isize;
        let centre = (&events[index], here);

        let mut total = self.event_cost(index, &events[index], here, baselines[index]);
        if let Some(prev) = at(i - 1) {
            total += self.move_cost(prev, centre);
        }
        if let Some(next) = at(i + 1) {
            total += self.move_cost(centre, next);
        }
        for j in [i - 1, i, i + 1] {
            if let Some(middle) = at(j) {
                total += self.window_cost(at(j - 1), middle, at(j + 1));
            }
        }
        if events.len() >= 2 {
            for j in [i, i + 1, i + 2] {
                if let Some(current) = at(j) {
                    total += self.prior_cost(at(j - 2), at(j - 1), current);
                }
            }
        }
        total
    }

    pub(crate) fn assemble(
        &self,
        events: &[Event],
        all: &[Vec<Candidate>],
        baselines: &[f32],
        chosen: &[usize],
        cost: f32,
    ) -> Solution {
        let mut fingerings = Vec::new();
        let mut explanations = Vec::new();
        let mut path = Vec::with_capacity(events.len());

        for (i, event) in events.iter().enumerate() {
            let candidate = &all[i][chosen[i]];
            path.push(Step {
                hand: self.hand,
                notes: event.notes.clone(),
                fingers: candidate.fingers.clone(),
                onset_seconds: event.onset_seconds,
            });
            let grip = self.grip(event, candidate);
            let outcome = self.biomech.grip_outcome(&grip);

            let prev = i.checked_sub(1).map(|j| (&events[j], &all[j][chosen[j]]));
            let next = events.get(i + 1).map(|e| (e, &all[i + 1][chosen[i + 1]]));
            let (low, high) = self.outer(event, candidate);
            let window = Trigram {
                hand: self.hand,
                prev: prev.map(|(e, c)| self.outer(e, c).1),
                current: high,
                next: next.map(|(e, c)| self.outer(e, c).1),
            };
            let shifting = event.len() > 1 && prev.is_some_and(|(e, _)| e.len() > 1);
            let held = window.prev.is_some_and(|p| p.finger == window.current.finger);
            let skip = if shifting && held { SHIFTED_VOICE_RULES } else { PER_NOTE_RULES };
            let fired = self.rules.score_melodic(&window, skip).fired();
            let _ = low;

            let mut breakdown = self.event_breakdown(i, event, candidate, baselines[i]);
            breakdown.rules += self.window_cost(prev, (event, candidate), next);
            breakdown.motion = prev.map(|p| self.move_cost(p, (event, candidate))).unwrap_or(0.0);
            if events.len() >= 2 {
                let before = |back: usize| i.checked_sub(back).map(|j| (&events[j], &all[j][chosen[j]]));
                breakdown.prior = self.prior_cost(before(2), before(1), (event, candidate));
            }

            let local = |c: &Candidate| self.neighbourhood(events, all, baselines, chosen, i, c);
            let chosen_cost = local(candidate);
            let mut margin = f32::INFINITY;
            for (j, other) in all[i].iter().enumerate() {
                if j == chosen[i] {
                    continue;
                }
                margin = margin.min(local(other) - chosen_cost);
            }
            if !margin.is_finite() {
                margin = 0.0;
            }

            for (slot, id) in event.ids.iter().enumerate() {
                if !event.struck[slot] {
                    continue;
                }
                let finger = candidate.fingers[slot];
                fingerings.push(Fingering::new(*id, finger));
                explanations.push(NoteExplanation {
                    note: *id,
                    finger,
                    rules: fired.clone(),
                    posture_strain: outcome.strain,
                    reachable: outcome.reachable,
                    margin,
                    cost: breakdown,
                });
            }
        }

        Solution { fingerings, cost, explanations, agreement: None, path }
    }
}

pub(crate) fn on_path<'a>(
    events: &'a [Event],
    all: &'a [Vec<Candidate>],
    chosen: &[usize],
    index: usize,
    here: &'a Candidate,
    j: isize,
) -> Option<(&'a Event, &'a Candidate)> {
    let j = usize::try_from(j).ok().filter(|j| *j < events.len())?;
    Some((&events[j], if j == index { here } else { &all[j][chosen[j]] }))
}

pub(crate) fn spread_fingers(hand: Hand, k: usize) -> Vec<Finger> {
    (0..k)
        .map(|i| {
            let slot = (i * 4) / (k - 1).max(1);
            let slot = slot.min(4);
            match hand {
                Hand::Right => Finger::from_index(slot),
                Hand::Left => Finger::from_index(4 - slot),
            }
        })
        .collect()
}
