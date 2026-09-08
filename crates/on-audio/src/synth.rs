//! The instrument: a bank of strings, a playhead, and the notes to strike.
//!
//! One synthesiser serves both the live window and the offline video export. That is
//! deliberate — the export is not a second implementation that has to be kept in
//! agreement with the first, it is the same object rendered at a fixed rate instead
//! of a real-time one, so what is exported is what was heard.

use crate::sampled::SampledPiano;
use crate::voice::Voice;

/// How loud the mix is before the limiter, relative to one note at full velocity.
///
/// Set so an ordinary two-hand texture sits comfortably below the limiter and only a
/// dense fortissimo chord touches it.
const MASTER_GAIN: f32 = 0.34;

/// How many frames are rendered between checks for notes starting and stopping.
///
/// A SoundFont player works in blocks, and asking one for a single frame at a time is
/// wasteful. Thirty-two frames is two thirds of a millisecond — far below anything
/// anyone can hear an onset shift by, and it keeps both kinds of instrument on one
/// loop.
const DISPATCH_BLOCK: usize = 32;

/// The most notes that may sound at once.
///
/// Well above what a pianist can hold — ten fingers, plus everything still ringing
/// under them — but bounded, so a pathological file cannot stall the audio thread.
const MAX_VOICES: usize = 64;

/// One note for the synthesiser to play.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Note {
    /// MIDI pitch.
    pub midi: u8,
    /// When the hammer strikes, in seconds from the start of the piece.
    pub start: f64,
    /// When the key is released.
    pub end: f64,
    /// How hard, 1..=127.
    pub velocity: u8,
}

/// A note that is currently sounding.
struct Sounding {
    voice: Voice,
    end: f64,
    released: bool,
}

/// A piano, and a piece to play on it.
pub struct Synth {
    notes: Vec<Note>,
    sample_rate: f32,
    /// A recorded instrument, if one was supplied. Everything below it is the
    /// modelled one, which plays when there is not.
    sampled: Option<Box<SampledPiano>>,
    voices: Vec<Sounding>,
    /// Index of the next note to strike; everything before it has been dealt with.
    next_note: usize,
    /// Which keys a recorded instrument has down, and when to let them up. The
    /// modelled voices carry their own release time; a SoundFont player does not.
    held: Vec<(u8, f64)>,
    position: f64,
    speed: f64,
    playing: bool,
}

impl Synth {
    /// Load a piece.
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

    /// Play this piece on a recorded instrument instead of the modelled one.
    pub fn with_soundfont(mut self, piano: SampledPiano) -> Self {
        self.sampled = Some(Box::new(piano));
        self
    }

    /// Put a different piece on the same instrument.
    ///
    /// Everything about the performance is reset and the instrument is kept. Building a
    /// whole new synth instead loses the recorded instrument with it, and the piece
    /// starts playing on the modelled one — which is what happened every time a setting
    /// changed, because changing a setting reloads the piece. Reading the SoundFont
    /// again would not do either: the good ones run to a gigabyte, and this is called
    /// from the audio thread between buffers.
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

    /// Swap the recorded instrument, or drop back to the modelled one.
    ///
    /// Whatever was sounding is silenced first: the notes are held by the instrument
    /// that started them, and the one taking over has no idea they are down.
    pub fn set_soundfont(&mut self, piano: Option<SampledPiano>) {
        if let Some(old) = self.sampled.as_deref_mut() {
            old.silence();
        }
        self.held.clear();
        self.voices.clear();
        self.sampled = piano.map(Box::new);
    }

    /// What the recorded instrument calls itself, if there is one.
    pub fn instrument(&self) -> Option<&str> {
        self.sampled.as_deref().map(SampledPiano::name)
    }

    /// Where the playhead is, in seconds.
    pub fn position(&self) -> f64 {
        self.position
    }

    /// Whether the playhead is moving.
    pub fn playing(&self) -> bool {
        self.playing
    }

    /// How fast the playhead is moving, where 1.0 is the score's own tempo.
    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// Start or stop the playhead. Notes already ringing are left to ring: pausing a
    /// piano does not silence the strings.
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    /// Play faster or slower. The pitch does not change — this is a pianist playing
    /// at a different tempo, not a tape running at a different speed.
    pub fn set_speed(&mut self, speed: f64) {
        self.speed = speed.max(0.0);
    }

    /// Jump the playhead, silencing whatever was ringing.
    ///
    /// A negative position is allowed and means the piece has not started yet. The
    /// view opens on a moment of empty keyboard before the first note arrives, and
    /// the sound has to agree with it about where that moment is.
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

    /// Whether the piece has finished and everything has stopped ringing.
    ///
    /// A recorded instrument keeps its own voices and does not say how many are still
    /// sounding. Once every note has been let go of, its tail is short and bounded, so
    /// having played everything is close enough.
    pub fn silent(&self) -> bool {
        self.next_note >= self.notes.len()
            && self.held.is_empty()
            && (self.sampled.is_some() || self.voices.is_empty())
    }

    /// Fill a buffer with interleaved stereo samples.
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
                // A recorded instrument mixes and limits itself.
                Some(piano) => piano.render(block),
                None => {
                    for frame in block.chunks_exact_mut(2) {
                        let mut mixed = [0.0f32; 2];
                        for sounding in &mut self.voices {
                            sounding.voice.mix_sample(&mut mixed);
                        }
                        // A soft knee rather than a hard clip. A dense chord in a loud
                        // passage will exceed full scale; folding it over sounds like
                        // distortion, and clamping it sounds like worse distortion.
                        frame[0] = (mixed[0] * MASTER_GAIN).tanh();
                        frame[1] = (mixed[1] * MASTER_GAIN).tanh();
                    }
                    self.voices.retain(|s| !s.voice.finished());
                }
            }
            written += frames * 2;
        }
    }

    /// Strike and release whatever the playhead has just reached.
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

            // A repeated note takes its own string back: a real one cannot ring twice
            // at once, and letting it would build up into a drone.
            if let Some(existing) = self.voices.iter().position(|s| s.voice.midi == note.midi) {
                self.voices.swap_remove(existing);
            }
            if self.voices.len() >= MAX_VOICES {
                // Drop whatever is quietest and oldest — in practice something that
                // has nearly died anyway.
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
        // Reloading is what happens whenever a setting changes. It used to build a
        // whole new synth, which reset all of this correctly and also threw away the
        // recorded instrument, putting the piece back on the modelled piano. That part
        // cannot be checked here without shipping a SoundFont to test against; it is
        // instead guaranteed by `load` taking `&mut self` and never clearing `sampled`.
        // What is checked here is everything the old reconstruction did that still has
        // to happen.
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

    /// Half speed should take twice as long to get through the piece.
    #[test]
    fn speed_scales_the_playhead() {
        let mut synth = Synth::new(scale(), 44_100.0);
        synth.set_playing(true);
        synth.set_speed(0.5);
        let mut out = vec![0.0; 44_100 * 2];
        synth.render(&mut out);
        assert!((synth.position() - 0.5).abs() < 1e-3, "{}", synth.position());
    }

    /// Seeking past a note means it is not played when the playhead moves on.
    #[test]
    fn seeking_skips_the_notes_behind_it() {
        let mut synth = Synth::new(scale(), 44_100.0);
        synth.seek(1.9);
        synth.set_playing(true);
        let mut out = vec![0.0; 4_410 * 2];
        synth.render(&mut out);
        assert!(synth.silent(), "notes were left to play after seeking past them");
    }

    /// A repeated note must not stack up into a drone.
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
