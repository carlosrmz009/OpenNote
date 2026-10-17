//! The tempo map: converts between ticks and seconds across tempo changes.

use crate::Ticks;

pub const DEFAULT_MICROS_PER_QUARTER: u32 = 500_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TempoChange {
    pub tick: Ticks,
    pub micros_per_quarter: u32,
}

impl TempoChange {
    pub fn bpm(&self) -> f64 {
        60_000_000.0 / self.micros_per_quarter as f64
    }
}

#[derive(Debug, Clone)]
pub struct TempoMap {
    changes: Vec<TempoChange>,
    /// Seconds from the start to each change, summed in order, so that a tick can be
    /// converted with a binary search rather than a walk through every change.
    starts: Vec<f64>,
}

/// Two maps are equal when their changes are; the start times follow from those.
impl PartialEq for TempoMap {
    fn eq(&self, other: &Self) -> bool {
        self.changes == other.changes
    }
}

impl Eq for TempoMap {}

impl Default for TempoMap {
    fn default() -> Self {
        Self::from_changes(vec![TempoChange { tick: 0, micros_per_quarter: DEFAULT_MICROS_PER_QUARTER }])
    }
}

impl TempoMap {
    pub fn constant_bpm(bpm: f64) -> Self {
        Self::from_changes(vec![TempoChange {
            tick: 0,
            micros_per_quarter: (60_000_000.0 / bpm.max(1.0)) as u32,
        }])
    }

    fn from_changes(changes: Vec<TempoChange>) -> Self {
        let mut map = Self { changes, starts: Vec::new() };
        map.restart_from(0);
        map
    }

    fn restart_from(&mut self, from: usize) {
        let ppq = crate::TICKS_PER_QUARTER as f64;
        self.starts.truncate(from);
        for i in from..self.changes.len() {
            let start = match i.checked_sub(1) {
                None => 0.0,
                Some(before) => {
                    let change = self.changes[before];
                    let span = (self.changes[i].tick - change.tick) as f64;
                    self.starts[before] + span / ppq * change.micros_per_quarter as f64 / 1e6
                }
            };
            self.starts.push(start);
        }
    }

    pub fn insert(&mut self, tick: Ticks, micros_per_quarter: u32) {
        if micros_per_quarter == 0 {
            return;
        }
        let tick = tick.max(0);
        let at = match self.changes.binary_search_by_key(&tick, |c| c.tick) {
            Ok(i) => {
                self.changes[i].micros_per_quarter = micros_per_quarter;
                i
            }
            Err(i) => {
                self.changes.insert(i, TempoChange { tick, micros_per_quarter });
                i
            }
        };
        self.restart_from(at);
    }

    pub fn changes(&self) -> &[TempoChange] {
        &self.changes
    }

    pub fn micros_per_quarter_at(&self, tick: Ticks) -> u32 {
        let i = match self.changes.binary_search_by_key(&tick, |c| c.tick) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        self.changes[i].micros_per_quarter
    }

    pub fn bpm_at(&self, tick: Ticks) -> f64 {
        60_000_000.0 / self.micros_per_quarter_at(tick) as f64
    }

    pub fn seconds_at(&self, tick: Ticks) -> f64 {
        if tick <= 0 {
            return 0.0;
        }
        // The last change before the tick; a change exactly at it starts after it.
        let i = self.changes.partition_point(|c| c.tick < tick).saturating_sub(1);
        let change = self.changes[i];
        let span = (tick - change.tick).max(0) as f64;
        self.starts[i] + span / crate::TICKS_PER_QUARTER as f64 * change.micros_per_quarter as f64 / 1e6
    }

    pub fn tick_at(&self, seconds: f64) -> Ticks {
        if seconds <= 0.0 {
            return 0;
        }
        let ppq = crate::TICKS_PER_QUARTER as f64;
        let mut elapsed = 0.0;
        for (i, change) in self.changes.iter().enumerate() {
            let rate = change.micros_per_quarter as f64 / 1e6 / ppq;
            let next = self.changes.get(i + 1).map(|c| c.tick);
            let span_seconds = match next {
                Some(n) => (n - change.tick) as f64 * rate,
                None => f64::INFINITY,
            };
            if elapsed + span_seconds > seconds {
                return change.tick + ((seconds - elapsed) / rate) as Ticks;
            }
            elapsed += span_seconds;
        }
        self.changes.last().map(|c| c.tick).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The walk through every change that `seconds_at` replaced.
    fn walked(map: &TempoMap, tick: Ticks) -> f64 {
        if tick <= 0 {
            return 0.0;
        }
        let ppq = crate::TICKS_PER_QUARTER as f64;
        let changes = map.changes();
        let mut seconds = 0.0;
        for (i, change) in changes.iter().enumerate() {
            if change.tick >= tick {
                break;
            }
            let end = changes.get(i + 1).map(|c| c.tick.min(tick)).unwrap_or(tick);
            seconds += (end - change.tick) as f64 / ppq * change.micros_per_quarter as f64 / 1e6;
        }
        seconds
    }

    #[test]
    fn many_changes_give_exactly_the_seconds_the_walk_did() {
        let mut map = TempoMap::default();
        let mut state = 7u64;
        for _ in 0..400 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let tick = ((state >> 33) % 200_000) as Ticks;
            map.insert(tick, 300_000 + ((state >> 20) % 600_000) as u32);
        }
        for tick in (-5..210_000).step_by(997) {
            assert_eq!(map.seconds_at(tick), walked(&map, tick), "at tick {tick}");
        }
    }
    use crate::TICKS_PER_QUARTER as PPQ;

    #[test]
    fn a_quarter_note_at_120_bpm_lasts_half_a_second() {
        let t = TempoMap::default();
        assert!((t.seconds_at(PPQ as Ticks) - 0.5).abs() < 1e-9);
        assert!((t.bpm_at(0) - 120.0).abs() < 1e-9);
    }

    #[test]
    fn tempo_changes_accumulate_piecewise() {
        let mut t = TempoMap::constant_bpm(60.0);
        t.insert(4 * PPQ as Ticks, 250_000);
        assert!((t.seconds_at(4 * PPQ as Ticks) - 4.0).abs() < 1e-9);
        assert!((t.seconds_at(8 * PPQ as Ticks) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn seconds_and_ticks_round_trip() {
        let mut t = TempoMap::constant_bpm(72.0);
        t.insert(3 * PPQ as Ticks, 300_000);
        t.insert(9 * PPQ as Ticks, 800_000);
        for tick in [0, 100, PPQ as Ticks, 5 * PPQ as Ticks, 20 * PPQ as Ticks] {
            let back = t.tick_at(t.seconds_at(tick));
            assert!((back - tick).abs() <= 1, "{tick} came back as {back}");
        }
    }

    #[test]
    fn changes_stay_sorted_however_they_arrive() {
        let mut t = TempoMap::default();
        t.insert(2000, 400_000);
        t.insert(500, 600_000);
        t.insert(1000, 300_000);
        let ticks: Vec<_> = t.changes().iter().map(|c| c.tick).collect();
        assert_eq!(ticks, vec![0, 500, 1000, 2000]);
        assert_eq!(t.micros_per_quarter_at(1500), 300_000);
        assert_eq!(t.micros_per_quarter_at(0), DEFAULT_MICROS_PER_QUARTER);
    }
}
