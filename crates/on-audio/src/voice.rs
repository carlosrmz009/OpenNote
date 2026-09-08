//! One struck piano string.
//!
//! Not a sample library and not a filter sweep over a sawtooth: a small additive
//! model of what a piano string actually does when a hammer hits it. That is worth
//! doing because the four things that make a piano recognisable are all cheap to
//! state directly.
//!
//! * **Inharmonicity.** A real string has stiffness, so its partials sit not at
//!   integer multiples of the fundamental but at `n·f₀·√(1 + Bn²)`. This is why a
//!   piano sounds like a piano rather than an organ, and why its octaves are tuned
//!   stretched. Fletcher & Rossing, *The Physics of Musical Instruments*, ch. 12.
//! * **Strike point.** The hammer hits about an eighth of the way along the string,
//!   which suppresses the eighth partial and its multiples — the classic `|sin(nπα)|`
//!   comb. Move the strike point and the tone changes character completely.
//! * **Partial-dependent decay.** High partials die away far faster than low ones, so
//!   a note starts bright and mellows as it rings. A fixed spectrum sounds synthetic
//!   however good it is at the attack.
//! * **The damper.** Lifting a finger drops a felt damper onto the string; the sound
//!   stops in a fraction of a second rather than at the end of its natural decay.
//!
//! Each partial runs as a rotating unit vector rather than as `sin(phase)`. A
//! rotation is four multiplies where a sine is a library call, which keeps a large
//! chord affordable even in an unoptimised build; the magnitude drifts slowly, so it
//! is renormalised every so often.

/// How many samples between renormalisations of a partial's rotating vector.
///
/// The rotation is a multiply by a unit complex number, and rounding makes the
/// magnitude wander over tens of thousands of samples. Renormalising costs a square
/// root — too much per sample, nothing at all per few thousand.
const RENORMALIZE_EVERY: u32 = 2_048;

/// The most partials any one string is given.
const MAX_PARTIALS: usize = 14;

/// Where along the string the hammer strikes, as a fraction of its length.
const STRIKE_POINT: f32 = 0.125;

/// How long the attack ramp lasts, in seconds. Short, but not instant: a step would
/// click.
const ATTACK_SECONDS: f32 = 0.004;

/// How long a damped string takes to fall silent once the key is released.
const DAMPING_SECONDS: f32 = 0.13;

/// The highest note that has a damper at all.
///
/// The top notes of a piano have none: their strings are so short that they stop on
/// their own, and the felt would do more harm than good. Holding one of those keys
/// down or letting it go sounds the same, which is a real and audible detail.
const HIGHEST_DAMPED_MIDI: u8 = 92;

/// One partial of one string.
#[derive(Debug, Clone, Copy)]
struct Partial {
    /// The rotating vector; its imaginary part is the sample.
    re: f32,
    im: f32,
    /// The rotation applied each sample, as a unit complex number.
    cos_step: f32,
    sin_step: f32,
    /// Current amplitude, and what it is multiplied by each sample.
    amplitude: f32,
    decay: f32,
}

impl Partial {
    /// Advance one sample and return this partial's contribution.
    #[inline]
    fn next_sample(&mut self) -> f32 {
        let re = self.re * self.cos_step - self.im * self.sin_step;
        let im = self.re * self.sin_step + self.im * self.cos_step;
        self.re = re;
        self.im = im;
        self.amplitude *= self.decay;
        im * self.amplitude
    }

    /// Pull the rotating vector back onto the unit circle.
    #[inline]
    fn renormalize(&mut self) {
        let magnitude = (self.re * self.re + self.im * self.im).sqrt();
        if magnitude > 1e-6 {
            self.re /= magnitude;
            self.im /= magnitude;
        }
    }
}

/// A single sounding note.
#[derive(Debug, Clone)]
pub struct Voice {
    partials: [Partial; MAX_PARTIALS],
    used: usize,
    /// Where the note sits in the stereo image, -1 left to 1 right.
    pan: f32,
    /// Ramped in over the attack, so the note does not begin with a click.
    gain: f32,
    target_gain: f32,
    attack_step: f32,
    /// Samples until the next renormalisation.
    countdown: u32,
    /// Set when the key is released and the note has a damper.
    damping: bool,
    damping_factor: f32,
    /// Which key this is, so a repeated note can take its own voice back.
    pub midi: u8,
}

impl Voice {
    /// Strike a string.
    ///
    /// `velocity` is the MIDI 1..=127 the score carries, and it changes the tone as
    /// well as the level: a harder blow gives a brighter note, because the hammer
    /// compresses more and couples more energy into the high partials. A synthesiser
    /// that only changes the volume sounds like a volume knob rather than a pianist.
    pub fn strike(midi: u8, velocity: u8, sample_rate: f32) -> Self {
        let fundamental = 440.0 * ((midi as f32 - 69.0) / 12.0).exp2();
        let loudness = (velocity as f32 / 127.0).clamp(0.02, 1.0);

        // Stiffness rises steeply toward the treble, where the strings are short and
        // thick for their length. Bass strings are wound precisely so that it does not.
        let inharmonicity = 4.0e-5 * ((midi as f32 - 21.0) / 22.0).exp2();

        // How long the fundamental takes to fall by 60 dB. A bottom A rings for the
        // better part of half a minute; the top of the keyboard is gone in a second.
        let decay_time = 0.6 + 26.0 * (-(midi as f32 - 21.0) / 26.0).exp();

        // A harder strike puts more energy into the upper partials.
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
            // Nothing above the Nyquist frequency, and nothing above hearing.
            if frequency > (sample_rate * 0.45).min(18_000.0) {
                break;
            }

            // Rolls off with partial number, faster than a plucked string because the
            // hammer is soft; the strike point notches out every eighth partial.
            let comb = (n * std::f32::consts::PI * STRIKE_POINT).sin().abs();
            let amplitude = comb * brightness.powf(n.min(8.0) - 1.0) / n.powf(1.45);
            if amplitude < 1.0e-4 {
                continue;
            }

            // Upper partials shed energy faster: the string loses more to internal
            // friction and to the bridge the faster it moves.
            let partial_decay_time = decay_time / n.powf(0.72);
            let step = std::f32::consts::TAU * frequency / sample_rate;
            partials[used] = Partial {
                re: 1.0,
                im: 0.0,
                cos_step: step.cos(),
                sin_step: step.sin(),
                amplitude,
                // Amplitude falls by 60 dB — a factor of a thousand — over the decay
                // time, which is what "decay time" conventionally means.
                decay: (-6.907_755 / (partial_decay_time * sample_rate)).exp(),
            };
            total += amplitude;
            used += 1;
        }

        // Normalize, so a bass note and a treble note struck equally hard arrive at
        // about the same loudness whatever their partial count worked out to be.
        if total > 0.0 {
            for partial in partials.iter_mut().take(used) {
                partial.amplitude /= total;
            }
        }

        Self {
            partials,
            used,
            // Spread across the stereo image the way the instrument sounds from the
            // bench: bass to the left, treble to the right, gently.
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

    /// Let the key up: the damper falls, unless this note has none.
    pub fn release(&mut self) {
        if self.midi <= HIGHEST_DAMPED_MIDI {
            self.damping = true;
        }
    }

    /// Whether this voice has faded to nothing, so its slot can be reused.
    pub fn finished(&self) -> bool {
        self.target_gain <= 1.0e-5
            || self.partials[..self.used]
                .iter()
                .all(|p| p.amplitude < 1.0e-5)
    }

    /// Add this voice's next sample into a stereo frame.
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

        // Equal-power panning, so moving across the image does not change loudness.
        let angle = (self.pan + 1.0) * std::f32::consts::FRAC_PI_4;
        frame[0] += sample * angle.cos();
        frame[1] += sample * angle.sin();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every partial has to be below the Nyquist frequency, or it aliases down into
    /// the audible range as a tone that is not in the music.
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

    /// A bass note should ring far longer than a treble one.
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

    /// Releasing a key has to stop the note, and quickly.
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

    /// The very top of the keyboard has no dampers, so releasing changes nothing.
    #[test]
    fn the_top_notes_have_no_dampers() {
        let mut voice = Voice::strike(105, 100, 44_100.0);
        voice.release();
        assert!(!voice.damping);
    }
}
