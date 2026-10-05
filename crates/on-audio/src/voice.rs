const RENORMALIZE_EVERY: u32 = 2_048;

const MAX_PARTIALS: usize = 14;

const STRIKE_POINT: f32 = 0.125;

const ATTACK_SECONDS: f32 = 0.004;

const DAMPING_SECONDS: f32 = 0.13;

const HIGHEST_DAMPED_MIDI: u8 = 92;

#[derive(Debug, Clone, Copy)]
struct Partial {
    re: f32,
    im: f32,
    cos_step: f32,
    sin_step: f32,
    amplitude: f32,
    decay: f32,
}

impl Partial {
    #[inline]
    fn next_sample(&mut self) -> f32 {
        let re = self.re * self.cos_step - self.im * self.sin_step;
        let im = self.re * self.sin_step + self.im * self.cos_step;
        self.re = re;
        self.im = im;
        self.amplitude *= self.decay;
        im * self.amplitude
    }

    #[inline]
    fn renormalize(&mut self) {
        let magnitude = (self.re * self.re + self.im * self.im).sqrt();
        if magnitude > 1e-6 {
            self.re /= magnitude;
            self.im /= magnitude;
        }
    }
}

#[derive(Debug, Clone)]
pub struct Voice {
    partials: [Partial; MAX_PARTIALS],
    used: usize,
    pan: f32,
    gain: f32,
    target_gain: f32,
    attack_step: f32,
    countdown: u32,
    damping: bool,
    damping_factor: f32,
    pub midi: u8,
}

impl Voice {
    pub fn strike(midi: u8, velocity: u8, sample_rate: f32) -> Self {
        let fundamental = 440.0 * ((midi as f32 - 69.0) / 12.0).exp2();
        let loudness = (velocity as f32 / 127.0).clamp(0.02, 1.0);

        let inharmonicity = 4.0e-5 * ((midi as f32 - 21.0) / 22.0).exp2();

        let decay_time = 0.6 + 26.0 * (-(midi as f32 - 21.0) / 26.0).exp();

        let brightness = 0.75 + 0.85 * loudness;

        let silent = Partial {
            re: 1.0,
            im: 0.0,
            cos_step: 1.0,
            sin_step: 0.0,
            amplitude: 0.0,
            decay: 1.0,
        };
        let mut partials = [silent; MAX_PARTIALS];
        let mut used = 0;
        let mut total = 0.0;

        for index in 0..MAX_PARTIALS {
            let n = index as f32 + 1.0;
            let frequency = n * fundamental * (1.0 + inharmonicity * n * n).sqrt();
            if frequency > (sample_rate * 0.45).min(18_000.0) {
                break;
            }

            let comb = (n * std::f32::consts::PI * STRIKE_POINT).sin().abs();
            let amplitude = comb * brightness.powf(n.min(8.0) - 1.0) / n.powf(1.45);
            if amplitude < 1.0e-4 {
                continue;
            }

            let partial_decay_time = decay_time / n.powf(0.72);
            let step = std::f32::consts::TAU * frequency / sample_rate;
            partials[used] = Partial {
                re: 1.0,
                im: 0.0,
                cos_step: step.cos(),
                sin_step: step.sin(),
                amplitude,
                decay: (-6.907_755 / (partial_decay_time * sample_rate)).exp(),
            };
            total += amplitude;
            used += 1;
        }

        if total > 0.0 {
            for partial in partials.iter_mut().take(used) {
                partial.amplitude /= total;
            }
        }

        Self {
            partials,
            used,
            pan: ((midi as f32 - 60.0) / 48.0).clamp(-1.0, 1.0) * 0.55,
            gain: 0.0,
            target_gain: loudness.powf(1.6),
            attack_step: 1.0 / (ATTACK_SECONDS * sample_rate),
            countdown: RENORMALIZE_EVERY,
            damping: false,
            damping_factor: (-6.907_755 / (DAMPING_SECONDS * sample_rate)).exp(),
            midi,
        }
    }

    pub fn release(&mut self) {
        if self.midi <= HIGHEST_DAMPED_MIDI {
            self.damping = true;
        }
    }

    pub fn finished(&self) -> bool {
        self.target_gain <= 1.0e-5
            || self.partials[..self.used]
                .iter()
                .all(|p| p.amplitude < 1.0e-5)
    }

    #[inline]
    pub fn mix_sample(&mut self, frame: &mut [f32; 2]) {
        if self.gain < self.target_gain {
            self.gain = (self.gain + self.attack_step).min(self.target_gain);
        }
        if self.damping {
            self.target_gain *= self.damping_factor;
            self.gain = self.gain.min(self.target_gain);
        }

        self.countdown -= 1;
        let renormalize = self.countdown == 0;
        if renormalize {
            self.countdown = RENORMALIZE_EVERY;
        }

        let mut sample = 0.0;
        for partial in self.partials[..self.used].iter_mut() {
            sample += partial.next_sample();
            if renormalize {
                partial.renormalize();
            }
        }
        sample *= self.gain;

        let angle = (self.pan + 1.0) * std::f32::consts::FRAC_PI_4;
        frame[0] += sample * angle.cos();
        frame[1] += sample * angle.sin();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_partial_is_above_the_nyquist_frequency() {
        for midi in 21..=108u8 {
            let voice = Voice::strike(midi, 90, 44_100.0);
            for partial in &voice.partials[..voice.used] {
                let frequency =
                    partial.sin_step.atan2(partial.cos_step) / std::f32::consts::TAU * 44_100.0;
                assert!(
                    frequency > 0.0 && frequency < 22_050.0,
                    "midi {midi} has a partial at {frequency} Hz"
                );
            }
        }
    }

    #[test]
    fn low_notes_ring_longer_than_high_ones() {
        let sample_rate = 44_100.0;
        let ring = |midi: u8| {
            let mut voice = Voice::strike(midi, 100, sample_rate);
            let mut frame = [0.0; 2];
            let mut samples = 0;
            while !voice.finished() && samples < (sample_rate as usize * 40) {
                voice.mix_sample(&mut frame);
                samples += 1;
            }
            samples as f32 / sample_rate
        };
        let bottom = ring(21);
        let top = ring(96);
        assert!(bottom > 8.0, "the bottom A died after {bottom:.1}s");
        assert!(top < bottom / 3.0, "the treble rang for {top:.1}s");
    }

    #[test]
    fn the_damper_stops_a_note() {
        let sample_rate = 44_100.0;
        let mut voice = Voice::strike(48, 100, sample_rate);
        let mut frame = [0.0; 2];
        for _ in 0..(sample_rate as usize / 10) {
            voice.mix_sample(&mut frame);
        }
        voice.release();
        let mut samples = 0;
        while !voice.finished() && samples < sample_rate as usize {
            voice.mix_sample(&mut frame);
            samples += 1;
        }
        let seconds = samples as f32 / sample_rate;
        assert!(
            seconds < 1.0,
            "the damper took {seconds:.2}s to stop the note"
        );
    }

    #[test]
    fn the_top_notes_have_no_dampers() {
        let mut voice = Voice::strike(105, 100, 44_100.0);
        voice.release();
        assert!(!voice.damping);
    }
}
