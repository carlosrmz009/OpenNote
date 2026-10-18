//! The scene: the plugin that draws the keyboard, notes, effects and hands, the tuning
//! constants for how they look, and the shared resources.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use on_fingering::biomech::BiomechWeights;
use on_hand::keyboard::{is_black, BLACK_KEY_HEIGHT, KEY_DIP, WHITE_KEY_LENGTH};
use on_hand::{Hand, HandProfile};
use tracing::warn;

use crate::layout::Layout;
use crate::skin;
use crate::timeline::{HandAnimator, Timeline};

mod app;
mod camera;
mod fx;
mod materials;
mod scene;
mod transport;

pub use app::*;
pub(crate) use camera::*;
pub(crate) use fx::*;
pub(crate) use materials::*;
pub(crate) use scene::*;
pub use transport::*;

const CAMERA_HEIGHT_MM: f32 = 600.0;

const CAMERA_TILT_DEG: f32 = 25.0;

const BLOOM_HAZE: f32 = 0.22;

const VIGNETTE: f32 = 0.40;

const BLOOM_INTENSITY: f32 = 0.30;

const VIEW_MARGIN: f32 = 1.06;

const LANE_Z: f32 = BLACK_KEY_HEIGHT + 4.0;

const NOTE_Z: f32 = -40.0;

const CASE_Z: f32 = -20.0;

const OCTAVE_LINE_Z: f32 = NOTE_Z - 1.0;

pub const LEAD_IN: f64 = -1.0;

pub(crate) const KEYBOARD_Y_MM: f32 = -WHITE_KEY_LENGTH;

/// Blue for the left hand and yellow for the right. They stay apart for most kinds of colour
/// blindness and against the black stage.
pub(crate) const HAND_COLOURS: [Color; 2] = [Color::srgb(0.10, 0.35, 1.0), Color::srgb(0.96, 0.83, 0.05)];

/// Orange and sky blue from Okabe and Ito's palette, which stay apart for the common kinds
/// of colour blindness, for those who want a warmer pair than the default.
pub(crate) const CLEAR_HAND_COLOURS: [Color; 2] =
    [Color::srgb(0.90, 0.45, 0.0), Color::srgb(0.34, 0.71, 0.91)];

#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct NoteColours(pub [Color; 2]);

impl Default for NoteColours {
    fn default() -> Self {
        Self(HAND_COLOURS)
    }
}

const MIN_LABELLED_HEIGHT_MM: f32 = 12.0;

const MAX_LABELS: usize = 128;

const NOTE_WIDTH: f32 = 0.78;

const OCTAVE_LINE_WIDTH_MM: f32 = 7.0;

const OCTAVE_LINE_GLOW: f32 = 0.055;

const BACK_GLOW_DEPTH_MM: f32 = 17.0;

const BACK_GLOW_GLOW: f32 = 0.30;

const HIT_LINE_GLOW: f32 = 4.4;

const FLASH_SIZE_MM: f32 = 58.0;

const FLASH_HEIGHT_MM: f32 = 5.0;

const FLASH_GLOW: f32 = 9.5;

const BEAM_WIDTH_MM: f32 = 30.0;

const BEAM_HEIGHT_MM: f32 = 150.0;

const BEAM_GLOW: f32 = 0.9;

/// How brightly a struck key's glow lights what is near it: this many lux at
/// `NOTE_LIGHT_REACH_MM` from the key. The scene is in millimetres while Bevy's point
/// lights fall off with the square of distance, so the power is worked out from that.
const NOTE_LIGHT_LUX: f32 = 30_000.0;

const NOTE_LIGHT_REACH_MM: f32 = 40.0;

const NOTE_LIGHT_LUMENS: f32 =
    NOTE_LIGHT_LUX * 4.0 * std::f32::consts::PI * NOTE_LIGHT_REACH_MM * NOTE_LIGHT_REACH_MM;

const NOTE_LIGHT_RANGE_MM: f32 = 175.0;

const NOTE_LIGHT_HEIGHT_MM: f32 = 26.0;

const MAX_NOTE_LIGHTS: usize = 10;

const KEY_GLOW: f32 = 5.2;

use on_hand::keyboard::BLACK_KEY_DIP;

const BLACK_KEY_GLOW: f32 = 1.45;

const SPARK_GLOW: f32 = 3.4;

const SPARK_LIFE: f32 = 0.60;

const SPARK_TAIL_SECONDS: f32 = 0.10;

const SPARK_EVAPORATE_SHAPE: f32 = 0.75;

const SPARK_EMIT_SECONDS: f32 = 0.26;

const SPARK_SUSTAIN_SHARE: f32 = 0.34;

const SPARKS_PER_NOTE: usize = 600;

const MAX_SPARKS: usize = 24_000;

const SPARK_RISE_MM: f32 = 255.0;

const SPARK_GRAVITY_MM: f32 = 45.0;

const SPARK_WANDER_KEYS: f32 = 0.50;

const SPARK_WANDER_SHAPE: f32 = 0.8;

const SPARK_WANDER_AT_BIRTH: f32 = 0.26;

const SPARK_FILAMENT: u32 = 13;

const SPARK_DRIFT_KEYS: f32 = 0.40;

const SPARK_CURL_KEYS: f32 = 0.14;

const SPARK_CURL_RATE: f32 = 2.6;

const SPARK_SIZE_MM: std::ops::Range<f32> = 0.9..3.2;

const SPARK_WHITE_FOR: f32 = 0.11;

const SPARK_TRAIL_SECONDS: f32 = 0.022;

const LABEL_FONT_PX: f32 = 15.0;

const NOTE_GLOW: f32 = 9.5;

const WHITE_KEY_DEPTH_MM: f32 = 14.0;

const KEY_GAP_MM: f32 = 0.5;

const WHITE_KEY_EDGE_MM: f32 = 0.9;

const BLACK_KEY_EDGE_MM: f32 = 1.4;

const BLACK_KEY_NARROWING_MM: f32 = 2.6;

const BLACK_KEY_RECESS_MM: f32 = 6.0;

const KEY_EDGE_SEGMENTS: usize = 4;

const WHITE_KEY_ROUGHNESS: f32 = 0.28;

const BLACK_KEY_ROUGHNESS: f32 = 0.18;

const LACQUER_EYE_MM: f32 = 900.0;

const WHITE_LACQUER: Vec4 = Vec4::new(1.0, 0.04, 0.6, 0.0);

const BLACK_LACQUER: Vec4 = Vec4::new(1.0, 0.05, 5.0, 0.0);

const LACQUER_STRIP: Vec4 = Vec4::new(0.325, 0.007, 0.09, 0.03);

const HIT_LINE_DEPTH_MM: f32 = 6.0;

#[derive(Resource)]
pub struct Performance {
    pub timeline: Timeline,
    pub layout: Layout,
    pub animators: Vec<HandAnimator>,
    pub decisions: crate::timeline::Decisions,
    pub assets_root: std::path::PathBuf,
    pub hands_available: bool,
    pub soundfont: Option<std::path::PathBuf>,
    pub torso: on_hand::torso::Torso,
}

impl Performance {
    pub fn new(
        timeline: Timeline,
        layout: Layout,
        profile: HandProfile,
        assets_root: std::path::PathBuf,
        hands_available: bool,
        soundfont: Option<std::path::PathBuf>,
    ) -> Self {
        let animators = timeline.animators(&profile, BiomechWeights::default());

        let torso = on_hand::torso::Torso::new(&profile, layout.centre_mm().0);
        let decisions = crate::timeline::Decisions::new(&animators, timeline.duration + 2.0);
        Self { timeline, layout, animators, decisions, assets_root, hands_available, soundfont, torso }
    }
}

#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct ViewportInset {
    pub top: f32,
    pub bottom: f32,
}

#[derive(Resource, Default)]
pub struct Audio(Option<on_audio::Player>);

impl Audio {
    pub fn open(timeline: &Timeline, soundfont: Option<std::path::PathBuf>) -> Self {
        match on_audio::Player::new(timeline.audio_notes(), soundfont) {
            Ok(player) => {
                let transport = Transport::default();
                player.seek(transport.position);
                player.set_speed(transport.speed);
                player.set_playing(transport.playing);
                Self(Some(player))
            }
            Err(error) => {
                warn!("playing without sound: {error:#}");
                Self(None)
            }
        }
    }

    fn player(&self) -> Option<&on_audio::Player> {
        self.0.as_ref()
    }

    pub fn seek(&self, seconds: f64) {
        if let Some(player) = self.player() {
            player.seek(seconds);
        }
    }

    pub fn set_playing(&self, playing: bool) {
        if let Some(player) = self.player() {
            player.set_playing(playing);
        }
    }

    pub fn set_speed(&self, speed: f64) {
        if let Some(player) = self.player() {
            player.set_speed(speed);
        }
    }

    pub fn load(&self, notes: Vec<on_audio::Note>) {
        if let Some(player) = self.player() {
            player.load(notes);
        }
    }

    pub fn set_instrument(&self, soundfont: Option<std::path::PathBuf>) {
        if let Some(player) = self.player() {
            player.set_instrument(soundfont);
        }
    }
}

pub struct VisualizerPlugin;

impl Plugin for VisualizerPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "note.wgsl");
        bevy::asset::embedded_asset!(app, "key.wgsl");
        app.init_resource::<Transport>()
            .init_resource::<ViewportInset>()
            .init_resource::<NoteColours>()
            .init_resource::<Visuals>()
            .init_resource::<Playing>()
            .init_resource::<Keys>()
            .add_plugins(crate::hands::HandsPlugin)
            .add_plugins(MaterialPlugin::<NoteMaterial>::default())
            .add_plugins(MaterialPlugin::<Lacquered>::default())
            .add_systems(
                Startup,
                (setup_camera, setup_keyboard, setup_notes, setup_sparks, setup_note_lights, setup_flashes),
            )
            .add_systems(
                Update,
                (
                    advance_transport,
                    handle_input,
                    settle_keys,
                    mute_hands,
                    recolour_sparks,
                    apply_visuals,
                    (
                        fit_camera_viewport,
                        move_notes,
                        update_sparks,
                        press_keys,
                        light_struck_keys,
                        raise_flashes,
                        crate::hands::pose_hands,
                        update_labels,
                    ),
                )
                    .chain(),
            );
    }
}

const CLOCK_SNAP_SECONDS: f64 = 0.15;

const CLOCK_SETTLE_SECONDS: f64 = 0.25;
