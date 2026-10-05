use crate::sampled::SampledPiano;
use crate::voice::Voice;

const MASTER_GAIN: f32 = 0.34;

const DISPATCH_BLOCK: usize = 32;

const MAX_VOICES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Note {
    pub midi: u8,
    pub start: f64,
    pub end: f64,
    pub velocity: u8,
}

struct Sounding {
    voice: Voice,
    end: f64,
    released: bool,
}

pub struct Synth {
    notes: Vec<Note>,
    sample_rate: f32,
    sampled: Option<Box<SampledPiano>>,
    voices: Vec<Sounding>,
    next_note: usize,
    held: Vec<(u8, f64)>,
    position: f64,
    speed: f64,
    playing: bool,
}

impl Synth {
    pub fn new(mut notes: Vec<Note>, sample_rate: f32) -> Self {
        notes.sort_by(|a, b| a.start.total_cmp(&b.start));
        Self {
            notes,
            sample_rate,
            sampled: None,
            voices: Vec::with_capacity(MAX_VOICES),
            held: Vec::new(),
            next_note: 0,
            position: 0.0,
            speed: 1.0,
            playing: false,
        }
    }

    pub fn with_soundfont(mut self, piano: SampledPiano) -> Self {
        self.sampled = Some(Box::new(piano));
        self
    }

    pub fn load(&mut self, mut notes: Vec<Note>) {
        notes.sort_by(|a, b| a.start.total_cmp(&b.start));
        self.notes = notes;
        self.voices.clear();
        self.next_note = 0;
        self.position = 0.0;
        if let Some(piano) = self.sampled.as_deref_mut() {
            piano.silence();
        }
        self.held.clear();
    }

    pub fn set_soundfont(&mut self, piano: Option<SampledPiano>) {
        if let Some(old) = self.sampled.as_deref_mut() {
            old.silence();
        }
        self.held.clear();
        self.voices.clear();
        self.sampled = piano.map(Box::new);
    }

    pub fn instrument(&self) -> Option<&str> {
        self.sampled.as_deref().map(SampledPiano::name)
    }

    pub fn position(&self) -> f64 {
        self.position
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    pub fn set_speed(&mut self, speed: f64) {
        self.speed = speed.max(0.0);
    }

    pub fn seek(&mut self, seconds: f64) {
        self.position = seconds;
        self.voices.clear();
        self.held.clear();
        if let Some(piano) = &mut self.sampled {
            piano.silence();
        }
        self.next_note = self
            .notes
            .partition_point(|note| note.start < self.position);
    }

    pub fn silent(&self) -> bool {
        self.next_note >= self.notes.len()
            && self.held.is_empty()
            && (self.sampled.is_some() || self.voices.is_empty())
    }

    pub fn render(&mut self, out: &mut [f32]) {
        let step = self.speed / f64::from(self.sample_rate);
        let mut written = 0;
        while written < out.len() {
            let frames = ((out.len() - written) / 2).min(DISPATCH_BLOCK);
            if frames == 0 {
                break;
            }
            let block = &mut out[written..written + frames * 2];
            if self.playing {
                self.position += step * frames as f64;
                self.dispatch();
            }
            match &mut self.sampled {
                Some(piano) => piano.render(block),
                None => {
                    for frame in block.chunks_exact_mut(2) {
                        let mut mixed = [0.0f32; 2];
                        for sounding in &mut self.voices {
                            sounding.voice.mix_sample(&mut mixed);
                        }
                        frame[0] = (mixed[0] * MASTER_GAIN).tanh();
                        frame[1] = (mixed[1] * MASTER_GAIN).tanh();
                    }
                    self.voices.retain(|s| !s.voice.finished());
                }
            }
            written += frames * 2;
        }
    }

    fn dispatch(&mut self) {
        while let Some(note) = self.notes.get(self.next_note) {
            if note.start > self.position {
                break;
            }
            self.next_note += 1;

            if let Some(piano) = &mut self.sampled {
                piano.strike(note.midi, note.velocity);
                self.held.retain(|(midi, _)| *midi != note.midi);
                self.held.push((note.midi, note.end));
                continue;
            }

            if let Some(existing) = self.voices.iter().position(|s| s.voice.midi == note.midi) {
                self.voices.swap_remove(existing);
            }
            if self.voices.len() >= MAX_VOICES {
                self.voices.remove(0);
            }
            self.voices.push(Sounding {
                voice: Voice::strike(note.midi, note.velocity, self.sample_rate),
                end: note.end,
                released: false,
            });
        }

        if let Some(piano) = &mut self.sampled {
            let now = self.position;
            self.held.retain(|(midi, end)| {
                if *end <= now {
                    piano.release(*midi);
                    return false;
                }
                true
            });
            return;
        }

        for sounding in &mut self.voices {
            if !sounding.released && sounding.end <= self.position {
                sounding.released = true;
                sounding.voice.release();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_another_piece_starts_it_from_the_beginning() {
        let mut synth = Synth::new(scale(), 44_100.0);
        synth.set_speed(1.5);
        synth.set_playing(true);
        synth.seek(1.0);
        assert!(synth.next_note > 0, "the seek should have consumed some notes");

        synth.load(scale());
        assert_eq!(synth.next_note, 0, "the new piece starts from its beginning");
        assert_eq!(synth.position, 0.0);
        assert!(synth.voices.is_empty(), "the old piece's voices are not still ringing");
        assert_eq!(synth.speed(), 1.5, "the transport is left alone");
        assert!(synth.playing());
    }

    fn scale() -> Vec<Note> {
        (0..8)
            .map(|i| Note {
                midi: 60 + i as u8,
                start: i as f64 * 0.25,
                end: i as f64 * 0.25 + 0.24,
                velocity: 90,
            })
            .collect()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |a, s| a.max(s.abs()))
    }

    #[test]
    fn a_paused_synth_makes_no_new_sound() {
        let mut synth = Synth::new(scale(), 44_100.0);
        let mut out = vec![0.0; 4_410 * 2];
        synth.render(&mut out);
        assert_eq!(peak(&out), 0.0);
        assert_eq!(synth.position(), 0.0);
    }

    #[test]
    fn playing_produces_sound_and_moves_the_playhead() {
        let mut synth = Synth::new(scale(), 44_100.0);
        synth.set_playing(true);
        let mut out = vec![0.0; 44_100 * 2];
        synth.render(&mut out);
        assert!((synth.position() - 1.0).abs() < 1e-3, "{}", synth.position());
        let level = peak(&out);
        assert!(level > 0.05, "the scale came out at {level}");
        assert!(level <= 1.0, "the limiter let {level} through");
    }

    #[test]
    fn speed_scales_the_playhead() {
        let mut synth = Synth::new(scale(), 44_100.0);
        synth.set_playing(true);
        synth.set_speed(0.5);
        let mut out = vec![0.0; 44_100 * 2];
        synth.render(&mut out);
        assert!((synth.position() - 0.5).abs() < 1e-3, "{}", synth.position());
    }

    #[test]
    fn seeking_skips_the_notes_behind_it() {
        let mut synth = Synth::new(scale(), 44_100.0);
        synth.seek(1.9);
        synth.set_playing(true);
        let mut out = vec![0.0; 4_410 * 2];
        synth.render(&mut out);
        assert!(synth.silent(), "notes were left to play after seeking past them");
    }

    #[test]
    fn a_repeated_note_takes_its_own_string_back() {
        let notes = (0..10)
            .map(|i| Note {
                midi: 60,
                start: i as f64 * 0.05,
                end: i as f64 * 0.05 + 2.0,
                velocity: 110,
            })
            .collect();
        let mut synth = Synth::new(notes, 44_100.0);
        synth.set_playing(true);
        let mut out = vec![0.0; 44_100 * 2];
        synth.render(&mut out);
        assert_eq!(synth.voices.len(), 1);
    }
}
