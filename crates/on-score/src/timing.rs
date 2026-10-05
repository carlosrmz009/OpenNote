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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TempoMap {
    changes: Vec<TempoChange>,
}

impl Default for TempoMap {
    fn default() -> Self {
        Self {
            changes: vec![TempoChange {
                tick: 0,
                micros_per_quarter: DEFAULT_MICROS_PER_QUARTER,
            }],
        }
    }
}

impl TempoMap {
    pub fn constant_bpm(bpm: f64) -> Self {
        Self {
            changes: vec![TempoChange {
                tick: 0,
                micros_per_quarter: (60_000_000.0 / bpm.max(1.0)) as u32,
            }],
        }
    }

    pub fn insert(&mut self, tick: Ticks, micros_per_quarter: u32) {
        if micros_per_quarter == 0 {
            return;
        }
        let tick = tick.max(0);
        match self.changes.binary_search_by_key(&tick, |c| c.tick) {
            Ok(i) => self.changes[i].micros_per_quarter = micros_per_quarter,
            Err(i) => self.changes.insert(i, TempoChange { tick, micros_per_quarter }),
        }
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
        let ppq = crate::TICKS_PER_QUARTER as f64;
        let mut seconds = 0.0;
        for (i, change) in self.changes.iter().enumerate() {
            if change.tick >= tick {
                break;
            }
            let span_end = self
                .changes
                .get(i + 1)
                .map(|c| c.tick.min(tick))
                .unwrap_or(tick);
            let span = (span_end - change.tick) as f64;
            seconds += span / ppq * change.micros_per_quarter as f64 / 1e6;
        }
        seconds
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
