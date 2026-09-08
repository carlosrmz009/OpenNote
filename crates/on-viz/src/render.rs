//! The window: a flat keyboard with notes falling onto it, and 3D hands playing.
//!
//! One world, one camera, one unit. The world is measured in millimetres of real
//! piano and the camera looks straight down at it, so the keys, the falling notes and
//! the hands are all in the same space at the same scale. Nothing has to be matched
//! up by eye — a hand that spans an octave on a real piano spans an octave here.
//!
//! The keyboard and the notes are drawn flat, with their shading painted into
//! generated textures rather than lit; see [`crate::skin`] for why. The hands are lit
//! properly, so they read as solid objects sitting above the keys.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use on_fingering::biomech::{BiomechModel, BiomechWeights};
use on_hand::keyboard::{is_black, BLACK_KEY_HEIGHT, KEY_DIP, WHITE_KEY_LENGTH};
use on_hand::{Hand, HandProfile};
use tracing::warn;

use crate::layout::Layout;
use crate::skin;
use crate::timeline::{HandAnimator, Timeline};

/// How high above the keys the camera sits. Orthographic, so this only has to clear
/// the tallest thing in the scene.
const CAMERA_HEIGHT_MM: f32 = 600.0;

/// Margin around the view, as a fraction of its height.
const VIEW_MARGIN: f32 = 1.06;

/// How far above the white keys the glow, sparks and hit line are drawn.
const LANE_Z: f32 = BLACK_KEY_HEIGHT + 4.0;

/// How deep the falling notes sit, in millimetres — *below* the top of a white key.
///
/// This is what makes a note disappear into the keyboard rather than shrink into it.
/// A note is one quad scaled along its length, so trimming it at the hit line squashes
/// its own texture: the rounded corners flatten and the grain compresses, and a long
/// note visibly de-stretches as it is consumed. Nothing is trimmed now. The bar keeps
/// its true length and passes behind the keys, which are opaque and nearer the camera,
/// so the keyboard eats it — which is what it looks like on a real one.
const NOTE_Z: f32 = -40.0;

/// Where the front of the instrument sits: behind the keys, including a key pressed all
/// the way down, and in front of the falling notes.
const CASE_Z: f32 = -20.0;

/// Where the octave markers sit: behind the notes, so a note passes in front of one.
const OCTAVE_LINE_Z: f32 = NOTE_Z - 1.0;


/// How long the view opens on an empty keyboard before the first note arrives.
///
/// A piece that begins on the downbeat of the very first frame gives the eye nothing
/// to read; a moment of stillness first makes the first note land.
pub const LEAD_IN: f64 = -1.0;

/// Where the view puts the keyboard, against where [`on_hand`] measures it.
///
/// `on-hand` works in the piano's own frame: the front edge of the keys is y = 0 and
/// y grows into the instrument. The view needs the far edge of the keys at y = 0
/// instead, because that is where the notes land and everything above it is lane. The
/// keyboard is drawn shifted by this; the hands are positioned by the hand model, so
/// they have to be shifted by exactly the same amount to land on the right key.
pub(crate) const KEYBOARD_Y_MM: f32 = -WHITE_KEY_LENGTH;

/// Colour of each hand's notes: the left hand cool, the right warm.
const HAND_COLOURS: [Color; 2] = [
    Color::srgb(0.85, 0.012, 0.012),
    Color::srgb(0.045, 0.88, 1.0),
];

/// What colour each hand's notes are, left first.
///
/// A resource rather than the constant above, because which colour goes with which hand
/// is a matter of taste and of what the piece is for: two hands the same for a study,
/// or a pair that reads clearly for somebody learning. Everything a note colours —
/// the bar, the dust, the flash, the light it throws on the keys — reads it from here,
/// so there is one place to change and no chance of the dust disagreeing with the note
/// that threw it.
#[derive(Resource, Debug, Clone, Copy)]
pub struct NoteColours(pub [Color; 2]);

impl Default for NoteColours {
    fn default() -> Self {
        Self(HAND_COLOURS)
    }
}

/// The shortest note that gets a number printed on it, in millimetres of lane.
const MIN_LABELLED_HEIGHT_MM: f32 = 12.0;

/// How many numbers to show at once. Beyond this they are too far up the lane to read.
const MAX_LABELS: usize = 128;

/// How much of its key a note bar covers, across.
///
/// Short of the whole key on purpose: the gap either side is what lets a run of
/// neighbouring notes read as separate notes rather than as one block of colour.
const NOTE_WIDTH: f32 = 0.78;

/// How wide an octave marker is drawn, in millimetres. The texture is mostly
/// falloff, so the line itself is a fraction of this.
const OCTAVE_LINE_WIDTH_MM: f32 = 7.0;

/// How brightly it is drawn.
///
/// Very low. It is there to be measured against, not looked at — the eye should find it
/// when it goes looking for an octave and not otherwise. The reference has no such
/// lines at all, and at any brightness where they are comfortably visible they are the
/// first thing that separates the two pictures.
const OCTAVE_LINE_GLOW: f32 = 0.055;

/// How far above the keys the blue band reaches, in millimetres, and how far past full
/// scale it is drawn.
const BACK_GLOW_DEPTH_MM: f32 = 17.0;
const BACK_GLOW_GLOW: f32 = 0.30;

/// How far past full scale the line where the notes land is drawn. It is the
/// brightest thing in the picture, and the band of light the bloom pass makes of it is
/// what says where "now" is at a glance.
const HIT_LINE_GLOW: f32 = 4.4;

/// How big the flash at a sounding key is, in millimetres across.
const FLASH_SIZE_MM: f32 = 84.0;

/// How far above the keys its centre sits. A little above the hit line, so the burst
/// looks like it is coming off the key rather than out of the front of the instrument.
const FLASH_HEIGHT_MM: f32 = 5.0;

/// How far past full scale it is drawn. This is the brightest thing in the picture,
/// and what the bloom pass turns into the wash of light over the keyboard.
///
/// The hit line itself is deliberately faint now. A bar bright along its whole length
/// tells you where "now" is and nothing else; the light belongs at the keys that are
/// actually sounding, which is what makes a keyboard look played rather than lit.
const FLASH_GLOW: f32 = 9.5;

/// How much light a sounding note actually casts into the scene, in lumens.
///
/// This is the difference between a picture of a glowing note and a note that glows.
/// A bar landing on a key puts a real light there: the keys either side of it pick up
/// its colour, the black keys catch a highlight along their lacquer, and the hand over
/// it is lit from below the way a hand over a lit surface is. None of that can be
/// painted on, because it depends on where the hand happens to be.
const NOTE_LIGHT_LUMENS: f32 = 5_400.0;

/// How far that light reaches, in millimetres. Short: it is a key lighting up, not a
/// lamp, and a long reach would wash the whole keyboard on a chord.
const NOTE_LIGHT_RANGE_MM: f32 = 175.0;

/// How high above the keys it sits. Just clear of the black keys, so it spills along
/// the keyboard rather than shining straight down into one key.
const NOTE_LIGHT_HEIGHT_MM: f32 = 26.0;

/// The most notes that light the scene at once.
///
/// A pedalled chord can hold more keys than this; the ones over the budget still glow,
/// they just do not cast. Ten is already more than a pianist has fingers.
const MAX_NOTE_LIGHTS: usize = 10;

/// How brightly a struck key glows, on top of its own colour.
///
/// Modest next to a note bar. A key that outshone the notes falling onto it would
/// pull the eye down to where the music has already been rather than up to where it
/// is going.
const KEY_GLOW: f32 = 5.2;

/// How far a black key travels when it is played.
///
/// Less than a white key's [`KEY_DIP`], and not as a matter of feel. The black keys are
/// boxes standing exactly `BLACK_KEY_HEIGHT` proud of the white ones, and that is the
/// same ten millimetres a key dips — so a black key pressed all the way put its top
/// face precisely in the plane of the white key tops. To a camera looking straight down
/// at it that is two coplanar surfaces, and the depth buffer picked between them
/// arbitrarily: the black key turned into a white stripe at the exact moment it was
/// played, which is a rather conspicuous time to disappear.
///
/// Leaving it a few millimetres proud settles that, and is what a real black key does
/// as well — its top stays above its neighbours through the whole of its travel.
const BLACK_KEY_DIP: f32 = BLACK_KEY_HEIGHT - 3.0;

/// The glow of a played black key.
///
/// Far lower, and not as a matter of taste. A white key shows the hand's colour twice
/// over: its lit surface takes the tint, and the emissive term on top of that pushes it
/// into the bloom. A black key has no lit surface to speak of — its texture is nearly
/// black, so whatever the base colour is multiplied by comes back as nothing — and the
/// emissive is all there is. Given the white key's figure it lands several times past
/// full scale with no colour underneath it, and the tonemapper does what it does with
/// any such value: returns white. The key vanished into a white rectangle at exactly
/// the moment it was played.
///
/// Just above one keeps it a colour rather than a clipped one, while still giving the
/// bloom something to find. It reads as brightly lit because it is surrounded by black
/// lacquer, which is contrast a white key never has.
const BLACK_KEY_GLOW: f32 = 1.45;

/// How much light a spark gives off. Well past full scale, so the bloom pass has
/// something to bloom.
/// Kept low enough that a mote stays the colour of the note that threw it.
///
/// It used to be able to run hotter, because the film curve took the top off anything
/// bright. Nothing does now — the picture goes to the screen as it was authored — so a
/// mote much past full scale simply clips every channel and the spray goes white,
/// losing the one thing that says which hand threw it.
///
/// Well past the bloom threshold, so each mote carries a halo rather than being a
/// coloured dot. Dust this small — a pixel or three — contributes almost nothing to a
/// screen-space bloom unless it is genuinely bright, and the reference dust plainly
/// glows: it lights the black around it.
const SPARK_GLOW: f32 = 4.2;

/// How long one mote of dust lives, in seconds.
const SPARK_LIFE: f32 = 1.05;

/// How long the full-rate burst at the strike lasts, in seconds.
///
/// A struck string throws most of its dust as it is hit and then keeps a haze while it
/// rings, so the plume is drawn the same way: everything at once for a quarter of a
/// second, and a thinner stream after that for as long as the key is down.
const SPARK_EMIT_SECONDS: f32 = 0.26;

/// What fraction of the motes keep coming once the strike is past.
///
/// A held note goes on throwing dust — it is still sounding, and a plume that stops
/// while the note does not leaves the key lit and bare. But it does not go on throwing
/// it as hard as the moment it was struck, which is what made a held chord stand under
/// a column of dust for a whole bar.
const SPARK_SUSTAIN_SHARE: f32 = 0.34;

/// How many motes each sounding note has in the air at once.
///
/// A sounding key does not throw one puff and stop; it goes on throwing dust for as
/// long as it is held. These are staggered across the life above so that at any moment
/// some are being born, some are at the top of their arc and some are fading — which is
/// what a fountain is, and what a single burst can never look like however many motes
/// it has in it.
const SPARKS_PER_NOTE: usize = 1_450;

/// The most that may be in the air at once, over all notes. A dense passage will exceed
/// it and the excess is simply not drawn.
const MAX_SPARKS: usize = 34_000;

/// How fast a mote is thrown, in millimetres per second, and how hard it falls.
///
/// Gently, on both counts. The dust in the reference is not thrown, it comes off the
/// key and goes out — a plume about seven white keys tall that thins as it climbs. Hard
/// enough to arc, and it reads as a firework instead.
const SPARK_RISE_MM: f32 = 370.0;
const SPARK_GRAVITY_MM: f32 = 45.0;

/// How far a mote wanders sideways by the end of its life, in millimetres.
///
/// The plume is not thrown anywhere. It comes off the top of the note about as wide as
/// the note is, widens as it climbs, and comes apart into a loose cloud near the top —
/// a flame, or steam off a cup. Nothing leaves at an angle and nothing arcs.
///
/// Which is why this is a wander and not a direction: each filament drifts its own way
/// by its own amount, and the plume widens because the filaments disagree, not because
/// anything is pushing them apart. Two arms leaning away from each other give a V, and
/// a V reads as spray.
const SPARK_WANDER_MM: f32 = 37.0;

/// How the wandering builds with time. Below one it is quick at first and then eases,
/// which is what leaves the plume narrow at the key and open at the top.
const SPARK_WANDER_SHAPE: f32 = 0.8;

/// How much of the wandering a mote has already done when it is born, as a fraction.
///
/// Without it the plume starts from a point and the first thing above the key is a
/// thread. It comes off the note about as wide as the note is.
const SPARK_WANDER_AT_BIRTH: f32 = 0.26;

/// How many motes share one trajectory.
///
/// The dust in the reference is not an even spray. It clumps into filaments with dark
/// lanes between them, which is what a real plume does and what stops it looking like
/// static. Motes are grouped and each group is thrown the same way, with only a little
/// jitter between them, so the filaments hold together as they climb.
const SPARK_FILAMENT: u32 = 13;

/// How far the whole plume leans as it rises, in millimetres per second.
///
/// The old dust at the top of a plume has drifted well off to one side in the
/// reference — there is air in the room. One direction per note, so a plume leans
/// rather than blurring.
const SPARK_DRIFT_MM: f32 = 26.0;

/// How far a mote wanders as it travels, and how tightly it curls.
const SPARK_CURL_MM: f32 = 15.0;
const SPARK_CURL_RATE: f32 = 2.6;

/// How big a mote is drawn, in millimetres. About a pixel at the usual framing: these
/// read as dust because they are small and there are thousands, not because of anything
/// drawn into them.
const SPARK_SIZE_MM: std::ops::Range<f32> = 0.9..3.2;

/// How long a mote stays white before it takes the colour of the note that threw it.
///
/// At the key the dust is too bright to have a colour at all, and it cools as it rises.
/// Two materials per hand rather than a gradient, because a mote is one quad and the
/// only thing that can vary per mote without a shader is which material it is drawn in.
const SPARK_WHITE_FOR: f32 = 0.11;

/// How long one period of a note's grain is, in millimetres of lane.
///
/// Long. A note is a slab of glass with a couple of facets in it, not a striped awning,
/// and the difference between the two is entirely this number. Short notes get one
/// period squeezed into whatever length they have, which is what they show in the
/// reference too.
const CRYSTAL_TILE_MM: f32 = 165.0;

/// Size of the finger numbers, in screen pixels.
const LABEL_FONT_PX: f32 = 15.0;

/// How much light a note gives off, beyond its own colour. This is what the bloom
/// pass turns into a glow.
const NOTE_GLOW: f32 = 9.5;

/// How thick a white key is drawn, so it reads as a key with a body rather than a
/// decal painted on the floor.
const WHITE_KEY_DEPTH_MM: f32 = 14.0;

/// Depth of the glowing line where the notes meet the keys.
const HIT_LINE_DEPTH_MM: f32 = 6.0;

/// Everything the visualizer needs to run.
#[derive(Resource)]
pub struct Performance {
    /// The piece.
    pub timeline: Timeline,
    /// Where things go on screen.
    pub layout: Layout,
    /// One animator per hand, left first.
    pub animators: Vec<HandAnimator>,
    /// Absolute path to the assets directory.
    ///
    /// Always set, and always an existing directory: Bevy resolves its asset root
    /// relative to the crate being run rather than to the working directory, and
    /// pointing it at somewhere that does not exist takes the whole render pass down
    /// with it rather than failing on the one asset that is missing.
    pub assets_root: std::path::PathBuf,
    /// Whether `hands/hand-left.glb` and `hands/hand-right.glb` are there.
    pub hands_available: bool,
    /// A recorded piano to play, if one was found or named. Without it the modelled
    /// one plays, which needs nothing downloaded.
    pub soundfont: Option<std::path::PathBuf>,
    /// The player the hands belong to: where they are sitting, and how far they can
    /// reach without getting up.
    pub torso: on_hand::torso::Torso,
}

impl Performance {
    /// Prepare a performance for display.
    pub fn new(
        timeline: Timeline,
        layout: Layout,
        profile: HandProfile,
        assets_root: std::path::PathBuf,
        hands_available: bool,
        soundfont: Option<std::path::PathBuf>,
    ) -> Self {
        let animators = Hand::ALL
            .iter()
            .map(|hand| {
                let model = BiomechModel::new(profile.clone(), *hand, BiomechWeights::default());
                HandAnimator::new(*hand, model, timeline.hand_grips(*hand).to_vec())
            })
            .collect();
        let torso = on_hand::torso::Torso::new(&profile, layout.centre_mm().0);
        Self { timeline, layout, animators, assets_root, hands_available, soundfont, torso }
    }
}

/// Where the playhead is and whether it is moving.
#[derive(Resource, Debug, Clone)]
pub struct Transport {
    /// Current position in seconds.
    pub position: f64,
    /// Whether playback is running.
    pub playing: bool,
    /// Playback rate, where 1.0 is the score's own tempo.
    pub speed: f64,
    /// Whether to loop back to the start at the end.
    pub looping: bool,
    /// Where the transport has just been told to jump to, until the sound gets there.
    ///
    /// The audio device is the clock, but it only learns about a jump at its next
    /// callback — a few milliseconds later. Without this the view would snap to the
    /// new position and then bounce back for one frame while the message was in
    /// flight.
    pending_seek: Option<f64>,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            position: LEAD_IN,
            playing: true,
            speed: 1.0,
            looping: true,
            pending_seek: None,
        }
    }
}

impl Transport {
    /// Jump to a moment in the piece.
    pub fn seek_to(&mut self, seconds: f64) {
        self.position = seconds;
        self.pending_seek = Some(seconds);
    }
}

/// How much of the window the interface has taken, in logical pixels.
///
/// The menu bar and the transport bar sit over the scene, so the camera is given only
/// what is left. Without this the keyboard would centre itself in the whole window
/// and spend its bottom centimetre behind the transport bar.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct ViewportInset {
    /// Height of the bar along the top.
    pub top: f32,
    /// Height of the bar along the bottom.
    pub bottom: f32,
}

/// The piano playing out of the speakers, if a device could be opened.
///
/// Optional on purpose. A machine with no sound card, or one whose card is already
/// taken, should still show the piece rather than refuse to start.
#[derive(Resource, Default)]
pub struct Audio(Option<on_audio::Player>);

impl Audio {
    /// Open the default sound device and load the piece onto it, paused at the start.
    ///
    /// Failure is not fatal and is not an error the caller has to handle: a machine
    /// with no sound card, or one whose card another program has taken, should still
    /// show the piece. It says so once and carries on in silence.
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

    /// The player, if there is one.
    fn player(&self) -> Option<&on_audio::Player> {
        self.0.as_ref()
    }

    /// Jump the playhead. Does nothing if there is no sound.
    pub fn seek(&self, seconds: f64) {
        if let Some(player) = self.player() {
            player.seek(seconds);
        }
    }

    /// Start or stop the sound.
    pub fn set_playing(&self, playing: bool) {
        if let Some(player) = self.player() {
            player.set_playing(playing);
        }
    }

    /// Play faster or slower.
    pub fn set_speed(&self, speed: f64) {
        if let Some(player) = self.player() {
            player.set_speed(speed);
        }
    }

    /// Put a different piece on the instrument.
    pub fn load(&self, notes: Vec<on_audio::Note>) {
        if let Some(player) = self.player() {
            player.load(notes);
        }
    }

    /// Play on a different recorded instrument, or on the modelled one.
    pub fn set_instrument(&self, soundfont: Option<std::path::PathBuf>) {
        if let Some(player) = self.player() {
            player.set_instrument(soundfont);
        }
    }
}

/// Build the keyboard and the falling notes again, for a different piece.
///
/// Everything the two setup systems made is thrown away and they are run again, which
/// keeps one description of what the scene contains rather than a second one that
/// only runs on reload and drifts away from the first.
pub(crate) fn rebuild_scene(world: &mut World) {
    let doomed: Vec<Entity> = world
        .query_filtered::<Entity, Or<(With<KeyTop>, With<NoteBar>, With<FingerLabel>)>>()
        .iter(world)
        .collect();
    for entity in doomed {
        world.entity_mut(entity).despawn();
    }
    let _ = world.run_system_cached(setup_keyboard);
    let _ = world.run_system_cached(setup_notes);
    let _ = world.run_system_cached(place_camera);
}

/// A key of the drawn keyboard.
#[derive(Component)]
struct KeyTop {
    midi: u8,
    /// Where the key sits when it is not pressed.
    rest: Vec3,
}

/// A bar in the falling-note lane.
#[derive(Component)]
struct NoteBar {
    /// When the note sounds.
    start: f64,
    /// How long it lasts.
    duration: f64,
    /// Which hand plays it.
    hand: Hand,
    /// The finger, if the note was fingered.
    finger: Option<u8>,
}

/// One of the sparks thrown up where a note lands.
#[derive(Component)]
struct Spark;

/// One of the lights a sounding note casts onto the keyboard.
#[derive(Component)]
struct NoteLight;

/// The column of light standing over a key that is sounding.
#[derive(Component)]
struct Flash {
    midi: u8,
}

/// The two colours sparks come in, one per hand.
#[derive(Resource)]
struct SparkColours {
    /// Freshly thrown, and too bright to have a colour.
    hot: [Handle<StandardMaterial>; 2],
    /// Cooled to the colour of the note that threw it.
    cool: [Handle<StandardMaterial>; 2],
}

/// One of the finger numbers floating over the note lane.
#[derive(Component)]
struct FingerLabel;

/// The plugin that draws everything.
pub struct VisualizerPlugin;

impl Plugin for VisualizerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Transport>()
            .init_resource::<ViewportInset>()
            .add_plugins(crate::hands::HandsPlugin)
            .add_systems(
                Startup,
                (
                    setup_camera,
                    setup_keyboard,
                    setup_notes,
                    setup_sparks,
                    setup_note_lights,
                    setup_flashes,
                ),
            )
            .add_systems(
                Update,
                (
                    advance_transport,
                    handle_input,
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

/// Place the camera looking straight down at the keyboard.
pub(crate) fn setup_camera(mut commands: Commands, performance: Res<Performance>) {
    let layout = &performance.layout;
    let (cx, cy) = layout.centre_mm();

    commands.spawn((
        Camera3d::default(),
        // Straight through, rather than through a film curve.
        //
        // Bevy's default tonemapper deliberately washes very bright colours towards
        // white — it is what makes a photographed highlight look photographed. This
        // picture is not a photograph: its notes are flat saturated colour taken well
        // past full scale so the bloom pass can find them, and the film curve was
        // turning a pure red note into a pink one. Measured against the reference, the
        // note's own colour channel was arriving at 206 of 255 and the two that should
        // have been near zero were arriving at 60.
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        camera_projection(layout),
        camera_placement(layout),
        // The notes are emissive, and bloom is what turns that into the glow a
        // falling-note display lives on. Restrained on purpose: enough to bleed light
        // past a note edge, not enough to smear the finger numbers.
        bevy::post_process::bloom::Bloom {
            intensity: 0.30,
            // Only things brighter than the screen can show are allowed to glow.
            //
            // Without this the keyboard blooms too — it is the biggest pale area in
            // the picture — and the haze it throws lifts the whole frame off black.
            // Measured before: the corners sat at 13 of 255 and the sky above the keys
            // at 45. A threshold at full scale confines the glow to the things that
            // are actually emitting: the notes, the line they land on, the sparks and
            // a struck key.
            prefilter: bevy::post_process::bloom::BloomPrefilter {
                threshold: 1.0,
                threshold_softness: 0.35,
            },
            // Additive rather than energy-conserving. Conserving energy dims the
            // original image to pay for the glow, which greys the black it is supposed
            // to be glowing against.
            composite_mode: bevy::post_process::bloom::BloomCompositeMode::Additive,
            // Tight, not hazy. `low_frequency_boost` is the weight on the widest mips,
            // which are what smear a glow across the whole frame.
            //
            // `max_mip_dimension` is the resolution the bloom chain *starts* at, and
            // reads backwards: a small number is blurrier, not tighter, because the
            // frame is reduced to that size before any of the work is done. At 128 the
            // whole effect was computed on a thumbnail and upsampled, which put a soft
            // grey wash over everything — the black keys came out as smudges with
            // haloes rather than as keys.
            low_frequency_boost: 0.05,
            max_mip_dimension: 512,
            ..bevy::post_process::bloom::Bloom::NATURAL
        },
        // Contact shadows. From straight overhead the only thing that says a
        // fingertip is *on* a key rather than above it is how the light dies in the
        // gap, and a shadow map at this scale cannot resolve a gap of a millimetre.
        bevy::pbr::ScreenSpaceAmbientOcclusion::default(),
        // Temporal antialiasing rather than multisampling. Both cost about the same
        // here; only one of them also cleans up the crawling edge of a slowly moving
        // finger, and the falling notes are the kind of high-contrast near-vertical
        // edge that multisampling handles worst.
        bevy::anti_alias::taa::TemporalAntiAliasing::default(),
        Msaa::Off,
    ));

    // The keyboard is lit rather than painted, so the black keys cast real shadows
    // across the white ones. That shadow is the single biggest thing separating a
    // keyboard that looks like an instrument from one that looks like a barcode.
    //
    // High and a little to the player's right. Steep, so a hand hovering four
    // centimetres over the keys drops its shadow just beside itself rather than half
    // an octave away, and offset enough that the shadow is not hidden underneath the
    // hand that casts it. That shadow is doing real work: in an orthographic view
    // from straight above it is the only cue for how far off the keys a finger is.
    commands.spawn((
        DirectionalLight {
            illuminance: 4_650.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(cx + 380.0, cy - 300.0, 1_050.0)
            .looking_at(Vec3::new(cx, cy, 0.0), Vec3::Z),
        // Cascades have to be told the size of the world. Bevy's defaults are set for
        // a scene measured in metres, and this one is measured in millimetres of
        // piano — over a metre of it — so with the defaults every shadow falls
        // outside the first cascade and nothing is ever drawn.
        bevy::light::CascadeShadowConfigBuilder {
            num_cascades: 2,
            first_cascade_far_bound: 900.0,
            maximum_distance: 2_400.0,
            ..default()
        }
        .build(),
    ));
    // A fill, from low down in front of the instrument. Without it the only light is
    // from overhead, and anything not facing the ceiling falls to black: a finger
    // angled down onto a key loses its whole underside and the hand ends up looking
    // like it is wearing a dark glove.
    //
    // Coming in almost flat is what makes this safe. The key tops face straight up, so
    // a grazing light barely touches them and the keyboard's own contrast is left
    // alone; the sides of the fingers face this light almost square on. No shadows —
    // it is filling the ones the key light leaves, not casting more.
    commands.spawn((
        DirectionalLight {
            illuminance: 1_250.0,
            color: Color::srgb(0.86, 0.88, 1.0),
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(cx, cy - 1_100.0, 170.0).looking_at(Vec3::new(cx, cy, 30.0), Vec3::Z),
    ));
    // A bigger shadow map than Bevy's default. The keyboard is over a metre wide and
    // the thing the shadows have to resolve is a ten-millimetre step at the edge of a
    // black key; at the default size that step is a couple of texels and every key
    // arrives with a soft grey fringe around it.
    commands.insert_resource(bevy::light::DirectionalLightShadowMap { size: 4096 });
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.52, 0.62, 0.90),
        brightness: 22.0,
        ..default()
    });
    commands.insert_resource(ClearColor(Color::BLACK));
}

/// Where the camera sits for a given layout.
///
/// Straight above the middle of what is being shown, far enough up to clear the
/// tallest thing in the scene. The projection is orthographic, so the height only has
/// to be enough to keep everything in front of the near plane.
fn camera_placement(layout: &Layout) -> Transform {
    let (cx, cy) = layout.centre_mm();
    Transform::from_xyz(cx, cy, CAMERA_HEIGHT_MM).looking_at(Vec3::new(cx, cy, 0.0), Vec3::Y)
}

/// How much of the world the camera takes in.
fn camera_projection(layout: &Layout) -> Projection {
    Projection::Orthographic(OrthographicProjection {
        // Fit whichever way round is tighter, so nothing is ever cut off and a
        // cropped keyboard actually fills the frame instead of sitting small in the
        // middle of it. On a sixteen-by-nine window showing all eighty-eight keys the
        // two constraints meet, which is what the layout's proportions are set for.
        scaling_mode: bevy::camera::ScalingMode::AutoMin {
            min_width: layout.width_mm() * VIEW_MARGIN,
            min_height: layout.height_mm() * VIEW_MARGIN,
        },
        near: 0.0,
        far: CAMERA_HEIGHT_MM * 4.0,
        ..OrthographicProjection::default_3d()
    })
}

/// Move the camera to suit whatever layout is current.
///
/// Cropping the keyboard to the piece, or showing all eighty-eight keys again,
/// changes how wide and how tall the world is, and the camera has to follow.
fn place_camera(
    performance: Res<Performance>,
    cameras: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
) {
    let layout = &performance.layout;
    for (mut transform, mut projection) in cameras {
        *transform = camera_placement(layout);
        *projection = camera_projection(layout);
    }
}

/// A key material: lit, so it takes the shadow the black keys cast, with its own
/// texture supplying the seam and the gloss that a flat overhead light cannot.
fn key_material(image: Handle<Image>, roughness: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(image),
        perceptual_roughness: roughness,
        reflectance: 0.35,
        ..default()
    }
}

/// A note material: unlit, so the colour arrives as authored, and brighter than the
/// screen can show, so the bloom pass has something to catch.
///
/// The brightness goes in the base colour rather than in `emissive`, which is not a
/// stylistic choice: Bevy's shader returns the base colour directly for an unlit
/// material and never reaches the emissive term at all. Setting `emissive` here looks
/// right and does nothing.
fn note_material(image: Handle<Image>, tint: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: glowing(tint, NOTE_GLOW),
        base_color_texture: Some(image),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    }
}

/// A colour taken past full scale, so the bloom pass sees it as a light source.
///
/// Alpha is left alone: the texture supplies coverage, and multiplying that up would
/// only make the soft edge of a note opaque.
fn glowing(tint: Color, brightness: f32) -> Color {
    let colour = tint.to_linear();
    Color::LinearRgba(LinearRgba {
        red: colour.red * brightness,
        green: colour.green * brightness,
        blue: colour.blue * brightness,
        alpha: 1.0,
    })
}

/// A flat, self-lit material, for the trim that should not take light at all.
fn painted(image: Handle<Image>, tint: Color, alpha: AlphaMode) -> StandardMaterial {
    StandardMaterial {
        base_color: tint,
        base_color_texture: Some(image),
        unlit: true,
        alpha_mode: alpha,
        ..default()
    }
}

/// Draw the keyboard: a quad per key, black keys sitting above the white ones, and a
/// felt rail across the back for the keys to meet.
fn setup_keyboard(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    performance: Res<Performance>,
) {
    let layout = &performance.layout;
    let white_texture = images.add(skin::white_key());
    let black_texture = images.add(skin::black_key());

    for midi in layout.keys() {
        let (x0, x1, y0, y1) = layout.key_rect(midi);
        let (width, length) = (x1 - x0, y1 - y0);
        let black = is_black(midi);

        // Each key gets its own material so it can be lit independently when played.
        let material = materials.add(key_material(
            if black { black_texture.clone() } else { white_texture.clone() },
            if black { 0.14 } else { 0.42 },
        ));

        // White keys are slabs at the surface; black keys are boxes standing proud of
        // them. Giving the black keys real height is what lets them throw a shadow
        // across their neighbours.
        let (mesh, z) = if black {
            (
                meshes.add(Cuboid::new(width, length, BLACK_KEY_HEIGHT)),
                BLACK_KEY_HEIGHT / 2.0,
            )
        } else {
            (
                meshes.add(Cuboid::new(width, length, WHITE_KEY_DEPTH_MM)),
                -WHITE_KEY_DEPTH_MM / 2.0,
            )
        };
        let rest = Vec3::new((x0 + x1) / 2.0, (y0 + y1) / 2.0, z);
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_translation(rest),
            KeyTop { midi, rest },
        ));
    }

    let centre_x = (layout.left_mm() + layout.right_mm()) / 2.0;

    // Octave markers. Piano music is read in octaves and a bare lane gives the eye
    // nothing to measure a leap against; one faint line at every C places a note
    // without turning the lane into graph paper. No backdrop behind them — the
    // background is black, and anything painted on it only greys it.
    let octave = materials.add(painted(
        images.add(skin::octave_line()),
        glowing(Color::srgb(0.62, 0.72, 0.95), OCTAVE_LINE_GLOW),
        AlphaMode::Add,
    ));
    let line = meshes.add(Rectangle::new(OCTAVE_LINE_WIDTH_MM, layout.lane_height));
    for midi in layout.keys() {
        // Every C, at the seam where it meets the B below it.
        if midi % 12 != 0 {
            continue;
        }
        let (left, _, _, _) = layout.key_rect(midi);
        commands.spawn((
            Mesh3d(line.clone()),
            MeshMaterial3d(octave.clone()),
            Transform::from_xyz(left, layout.lane_height / 2.0, OCTAVE_LINE_Z),
        ));
    }

    // The instrument's front, below the keys.
    //
    // It is not decoration. The falling notes are drawn at their true length and pass
    // behind the keyboard rather than being trimmed at the hit line, which is what
    // keeps a long note from squashing its own texture as it is played — but it means
    // a long note carries on below the keys, where there is nothing but background to
    // hide it. This is what hides it: opaque, nearer the camera than the notes, and
    // reaching all the way to the bottom of the view.
    // Half again as deep as the view needs, because the camera fits the layout to
    // whichever of width or height binds and can show a little past the bottom edge.
    //
    // It reaches from the very top of the keys, not from their bottom edge. Butting it
    // against the keyboard leaves a seam: a key dips towards the player as it is pressed
    // and the front of the instrument does not, so the two slide apart by a few
    // millimetres and a note shows through the gap. Running the whole way up costs
    // nothing, because the keys are opaque and nearer the camera than this is, so the
    // part behind them is never seen.
    let case_depth = -layout.bottom_mm() * 1.4;
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(layout.width_mm(), case_depth))),
        MeshMaterial3d(materials.add(painted(
            images.add(skin::case()),
            Color::WHITE,
            AlphaMode::Opaque,
        ))),
        Transform::from_xyz(centre_x, -case_depth / 2.0, CASE_Z),
    ));

    // The blue band standing above the back of the keys. See [`skin::back_glow`] for
    // why it is not simply the hit line's own bloom.
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(layout.width_mm(), BACK_GLOW_DEPTH_MM))),
        MeshMaterial3d(materials.add(painted(
            images.add(skin::back_glow()),
            glowing(Color::srgb(0.0, 0.14, 1.0), BACK_GLOW_GLOW),
            AlphaMode::Add,
        ))),
        Transform::from_xyz(centre_x, BACK_GLOW_DEPTH_MM / 2.0, LANE_Z - 1.0),
    ));

    // The line where the notes arrive. Every falling-note display has one, because a
    // keyboard on its own does not tell the eye where "now" is.
    let hit = materials.add(painted(
        images.add(skin::hit_line()),
        glowing(Color::srgb(0.40, 1.0, 1.0), HIT_LINE_GLOW),
        AlphaMode::Add,
    ));
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(layout.width_mm(), HIT_LINE_DEPTH_MM))),
        MeshMaterial3d(hit),
        Transform::from_xyz(centre_x, 0.0, LANE_Z - 1.0),
    ));
}

/// Spawn a bar for every note. They are created once and moved each frame, which is
/// steadier than spawning and despawning as the music goes past.
fn setup_notes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    performance: Res<Performance>,
    note_colours: Res<NoteColours>,
) {
    let layout = &performance.layout;
    let texture = images.add(skin::note_bar());
    let colours: Vec<Handle<StandardMaterial>> = note_colours
        .0
        .iter()
        .map(|colour| materials.add(note_material(texture.clone(), *colour)))
        .collect();

    for note in &performance.timeline.notes {
        if !layout.shows(note.midi) {
            continue;
        }
        let (x0, x1, _, _) = layout.note_rect(note.midi, 0.0, 0.0);
        // Built at its true length, rather than as a unit quad stretched by its
        // transform. A note's length never changes once the tempo is fixed, and the
        // stretching was what made the rounded ends of a long note flatten out.
        let width = (x1 - x0) * NOTE_WIDTH;
        let mesh = meshes.add(note_mesh(width, layout.note_height(note.end - note.start).max(6.0)));
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(colours[note.hand as usize].clone()),
            Transform::from_xyz((x0 + x1) / 2.0, 0.0, NOTE_Z),
            Visibility::Hidden,
            NoteBar {
                start: note.start,
                duration: note.end - note.start,
                hand: note.hand,
                finger: note.finger.map(|f| f.number()),
            },
        ));
    }
}

/// One note bar, as geometry.
///
/// Quads stacked: a head, a tail, and as many copies of the middle band as it takes to
/// fill what is between them. The head and the tail each take a fixed slice of the
/// texture and are given a piece of the world with the same aspect, so the rounded
/// corner drawn into the texture arrives on screen at the shape it was drawn. The
/// middle band is one period of the note's grain and is repeated rather than stretched,
/// which is what lets a note carry detail along its length: stretch one row of texture
/// and all you can ever have is vertical stripes.
///
/// The band is sized so a whole number of them fits exactly, which means no repeat is
/// ever cut in half. It is measured in millimetres of lane rather than in note widths:
/// the grain has to be a few broad sweeps down the length of a note, and a period tied
/// to the note's width gives a period of about a centimetre, which is a barcode.
fn note_mesh(width: f32, height: f32) -> Mesh {
    let cap = (width * skin::NOTE_CAP).min(height * 0.5);
    let middle = height - 2.0 * cap;
    let bands = ((middle / CRYSTAL_TILE_MM).round() as usize).max(1);
    let band = middle / bands as f32;

    let (half_width, half_height) = (width / 2.0, height / 2.0);
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    // Top to bottom, matching the texture, whose `v` runs down from the far end.
    let mut top = half_height;
    let mut quad = |top: f32, bottom: f32, v_top: f32, v_bottom: f32| {
        let base = positions.len() as u32;
        for (y, v) in [(top, v_top), (bottom, v_bottom)] {
            for (x, u) in [(-half_width, 0.0), (half_width, 1.0)] {
                positions.push([x, y, 0.0]);
                uvs.push([u, v]);
            }
        }
        indices.extend([base, base + 2, base + 3, base, base + 3, base + 1]);
    };

    quad(top, top - cap, 0.0, skin::NOTE_CAP);
    top -= cap;
    for _ in 0..bands {
        quad(top, top - band, skin::NOTE_CAP, 1.0 - skin::NOTE_CAP);
        top -= band;
    }
    quad(top, -half_height, 1.0 - skin::NOTE_CAP, 1.0);

    let count = positions.len();
    Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; count])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(bevy::mesh::Indices::U32(indices))
}

/// Make the pool of sparks that notes throw up as they land.
///
/// Pooled rather than spawned and despawned. A busy passage lands several notes a
/// frame, and churning entities for something that lives four tenths of a second
/// would cost more than keeping a few hundred hidden quads around.
fn setup_sparks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    note_colours: Res<NoteColours>,
) {
    let texture = images.add(skin::spark());
    let mesh = meshes.add(Rectangle::new(1.0, 1.0));
    let mut dust = |colour: Color, whiteness: f32, glow: f32| {
        materials.add(StandardMaterial {
            base_color: glowing(mix(colour, Color::WHITE, whiteness), glow),
            base_color_texture: Some(texture.clone()),
            unlit: true,
            alpha_mode: AlphaMode::Add,
            ..default()
        })
    };
    let colours = SparkColours {
        hot: note_colours.0.map(|colour| dust(colour, 0.70, SPARK_GLOW * 2.6)),
        cool: note_colours.0.map(|colour| dust(colour, 0.06, SPARK_GLOW)),
    };

    for _ in 0..MAX_SPARKS {
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(colours.cool[0].clone()),
            Transform::from_xyz(0.0, 0.0, LANE_Z + 2.0),
            Visibility::Hidden,
            Spark,
        ));
    }
    commands.insert_resource(colours);
}

/// Give every key a flash, switched off.
///
/// One per key rather than a pool, because a flash belongs to its key and never moves;
/// only its size changes. Eighty-eight hidden quads cost nothing.
fn setup_flashes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    performance: Res<Performance>,
    note_colours: Res<NoteColours>,
) {
    let layout = &performance.layout;
    let texture = images.add(skin::flash());
    let colours = note_colours.0.map(|colour| {
        materials.add(StandardMaterial {
            // Mostly the note's own colour, taken far past full scale. The middle
            // then clips to white on its own and the edges keep the colour, which is
            // what the reference does — a white-hot centre inside a coloured flare.
            // Mixing white in beforehand makes the whole thing uniformly pale instead.
            base_color: glowing(mix(colour, Color::WHITE, 0.20), FLASH_GLOW),
            base_color_texture: Some(texture.clone()),
            unlit: true,
            alpha_mode: AlphaMode::Add,
            ..default()
        })
    });

    let quad = meshes.add(Rectangle::new(FLASH_SIZE_MM, FLASH_SIZE_MM));
    for midi in layout.keys() {
        let (x0, x1, _, _) = layout.key_rect(midi);
        commands.spawn((
            Mesh3d(quad.clone()),
            MeshMaterial3d(colours[0].clone()),
            Transform::from_xyz((x0 + x1) / 2.0, FLASH_HEIGHT_MM, LANE_Z + 1.0),
            Visibility::Hidden,
            Flash { midi },
        ));
    }
    commands.insert_resource(FlashColours(colours));
}

/// The two colours a flash comes in, one per hand.
#[derive(Resource)]
struct FlashColours([Handle<StandardMaterial>; 2]);

/// Light a flash over every key that is sounding.
fn raise_flashes(
    transport: Res<Transport>,
    performance: Res<Performance>,
    colours: Res<FlashColours>,
    flashes: Query<(
        &Flash,
        &mut Transform,
        &mut Visibility,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let states = performance.timeline.key_depression(transport.position);
    for (flash, mut transform, mut visibility, mut material) in flashes {
        let depth = states.depth_of(flash.midi);
        let Some(hand) = states.hand_on(flash.midi).filter(|_| depth > 0.02) else {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        };
        // Biggest at the instant the key lands and shrinking as it is held, so a
        // struck key flares and a held one keeps a glow rather than a flare.
        transform.scale = Vec3::splat(0.35 + 0.65 * depth.powf(0.55));
        *visibility = Visibility::Visible;

        let wanted = &colours.0[hand as usize];
        if material.0.id() != wanted.id() {
            material.0 = wanted.clone();
        }
    }
}

/// Make the pool of lights that sounding notes cast.
///
/// Pooled and switched on rather than spawned, because a light coming into existence
/// makes the renderer rebuild its light bindings, and a fast passage would do that
/// several times a frame.
fn setup_note_lights(mut commands: Commands) {
    for _ in 0..MAX_NOTE_LIGHTS {
        commands.spawn((
            PointLight {
                intensity: 0.0,
                range: NOTE_LIGHT_RANGE_MM,
                radius: 12.0,
                // No shadows. Ten shadow-casting lights over a skinned mesh costs more
                // than the whole rest of the frame, and what these are for is the
                // colour they throw, not the shapes they cut.
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, NOTE_LIGHT_HEIGHT_MM),
            Visibility::Hidden,
            NoteLight,
        ));
    }
}

/// Put a light on every key that is sounding.
fn light_struck_keys(
    transport: Res<Transport>,
    performance: Res<Performance>,
    note_colours: Res<NoteColours>,
    lights: Query<(&mut PointLight, &mut Transform, &mut Visibility), With<NoteLight>>,
) {
    let layout = &performance.layout;
    let states = performance.timeline.key_depression(transport.position);
    let offset = Vec3::Y * KEYBOARD_Y_MM;

    // Brightest first, so if there are more sounding keys than lights the ones that
    // matter get them.
    let mut sounding: Vec<(f32, u8, Hand)> = layout
        .keys()
        .filter_map(|midi| {
            let depth = states.depth_of(midi);
            let hand = states.hand_on(midi)?;
            (depth > 0.02).then_some((depth, midi, hand))
        })
        .collect();
    sounding.sort_by(|a, b| b.0.total_cmp(&a.0));

    for (slot, (mut light, mut transform, mut visibility)) in lights.into_iter().enumerate() {
        match sounding.get(slot) {
            Some((depth, midi, hand)) => {
                let (x0, x1, y0, y1) = layout.key_rect(*midi);
                light.color = note_colours.0[*hand as usize];
                light.intensity = NOTE_LIGHT_LUMENS * depth;
                transform.translation = Vec3::new(
                    (x0 + x1) / 2.0,
                    // Toward the near end of the key, which is where the finger is and
                    // where there is something to light.
                    y0 + (y1 - y0) * 0.28,
                    NOTE_LIGHT_HEIGHT_MM,
                ) + offset;
                *visibility = Visibility::Visible;
            }
            None => {
                if *visibility != Visibility::Hidden {
                    *visibility = Visibility::Hidden;
                    light.intensity = 0.0;
                }
            }
        }
    }
}

/// Place the sparks for whatever has just landed.
///
/// Worked out from the playhead rather than accumulated frame by frame: a spark's
/// whole flight is a function of how long ago its note landed, so seeking, looping and
/// pausing all do the obvious thing without any state to keep in step, and a frame of
/// Throw dust up from every key that is sounding.
///
/// A fountain rather than a puff. Each note keeps a fixed number of motes and their
/// births are staggered evenly across one lifetime, so at any instant some are leaving
/// the key, some are at the top of their arc and some are fading out — and the stream
/// goes on for as long as the note is held. A single burst at the onset cannot look
/// like this however many motes are in it, because after the first tenth of a second
/// they are all the same age and the whole thing fades at once.
///
/// Every mote is a pure function of its note, its index and which time round it is, so
/// the same passage throws the same dust every time it is played and an exported frame
/// matches the live one exactly.
fn update_sparks(
    transport: Res<Transport>,
    performance: Res<Performance>,
    colours: Res<SparkColours>,
    sparks: Query<
        (&mut Transform, &mut Visibility, &mut MeshMaterial3d<StandardMaterial>),
        With<Spark>,
    >,
) {
    let layout = &performance.layout;
    let now = transport.position;
    let mut slots = sparks.into_iter();
    let stagger = SPARK_LIFE / SPARKS_PER_NOTE as f32;

    for (index, note) in performance.timeline.notes.iter().enumerate() {
        let elapsed = (now - note.start) as f32;
        // A key throws dust for as long as it is down, hardest at the strike.
        let throwing = (note.end - note.start) as f32;
        // Nothing yet, or the last mote thrown has already died.
        if elapsed < 0.0 || elapsed > throwing + SPARK_LIFE {
            continue;
        }
        let (x0, x1, _, _) = layout.key_rect(note.midi);
        let centre = (x0 + x1) / 2.0;

        for spark in 0..SPARKS_PER_NOTE {
            let Some((mut transform, mut visibility, mut material)) = slots.next() else {
                return;
            };
            *visibility = Visibility::Hidden;

            // Where this mote is in its own cycle. Each one is offset a little further
            // into the lifetime than the last, which is what spreads the births out.
            let since = elapsed - spark as f32 * stagger;
            if since < 0.0 {
                continue;
            }
            let turn = (since / SPARK_LIFE).floor();
            let age = since - turn * SPARK_LIFE;
            // Fresh randomness each time round, so a long note does not throw the same
            // handful of trajectories over and over.
            let seed = (index as u32)
                .wrapping_mul(0x9e37_79b9)
                .wrapping_add((spark as u32).wrapping_mul(0x85eb_ca6b))
                ^ (turn as u32).wrapping_mul(0xc2b2_ae35);

            // The mote has to have been thrown while the key was still down, and once
            // the strike is past only some of them are.
            let born = since - age;
            if born > throwing {
                continue;
            }
            if born > SPARK_EMIT_SECONDS
                && scramble(seed ^ 0x7f4a_7c15) > SPARK_SUSTAIN_SHARE
            {
                continue;
            }

            // Motes travel in filaments rather than one at a time: a group shares a
            // trajectory and differs only in jitter, which is what leaves the dark lanes
            // between the threads of a plume instead of an even wash.
            let strand = seed
                .wrapping_div(SPARK_FILAMENT)
                .wrapping_mul(0x9e37_79b9)
                ^ (turn as u32).wrapping_mul(0x85eb_ca6b);

            // Where this filament wanders to, and how fast it climbs. Both are its own,
            // and the plume opens out because they differ rather than because anything
            // is pushing.
            let reach = SPARK_WANDER_MM * (2.0 * scramble(strand ^ 0x94d0_49bb) - 1.0);
            let upward =
                SPARK_RISE_MM * (0.40 + 0.60 * scramble(strand ^ 0x5bf0_3635).powf(1.7));
            let size = SPARK_SIZE_MM.start
                + (SPARK_SIZE_MM.end - SPARK_SIZE_MM.start) * scramble(seed ^ 0x27d4_eb2f);
            let phase = scramble(seed ^ 0x1656_67b1) * std::f32::consts::TAU;

            let height = upward * age - 0.5 * SPARK_GRAVITY_MM * age * age;
            // Back on the keys and spent, whatever is left of its allotted life.
            if height < 0.0 {
                continue;
            }
            // Narrow where it leaves the note and open at the top, which is the shape of
            // something coming apart rather than something being thrown.
            let opened = (age / SPARK_LIFE).powf(SPARK_WANDER_SHAPE);
            let spread =
                reach * (SPARK_WANDER_AT_BIRTH + (1.0 - SPARK_WANDER_AT_BIRTH) * opened);
            // A slow curl, and the whole plume leaning as it goes: there is air in the
            // room, and the oldest dust at the top of one has drifted well off to the
            // side of the key that threw it.
            let curl = (phase + age * SPARK_CURL_RATE).sin() * SPARK_CURL_MM * age;
            let lean = if note.id.0 % 2 == 0 { 1.0 } else { -1.0 };
            let drift = lean * SPARK_DRIFT_MM * age * age;
            let fade = (1.0 - age / SPARK_LIFE).powf(1.3);

            transform.translation =
                Vec3::new(centre + spread + curl + drift, height, LANE_Z + 2.0);
            transform.scale = Vec3::splat(size * (0.5 + 0.5 * fade));
            *visibility = Visibility::Visible;

            let hand = note.hand as usize;
            let wanted = if age < SPARK_WHITE_FOR {
                &colours.hot[hand]
            } else {
                &colours.cool[hand]
            };
            if material.0.id() != wanted.id() {
                material.0 = wanted.clone();
            }
        }
    }

    // Everything left over is spent, or was never lit.
    for (_, mut visibility, _) in slots {
        if *visibility != Visibility::Hidden {
            *visibility = Visibility::Hidden;
        }
    }
}

/// A repeatable number between 0 and 1, from an integer.
///
///
/// A hash rather than a random number generator: the sparks have to be the same every
/// time a passage is played, or the video export would not match the window.
fn scramble(mut x: u32) -> f32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x as f32 / u32::MAX as f32
}

/// Move the playhead.
fn advance_transport(
    time: Res<Time>,
    mut transport: ResMut<Transport>,
    performance: Res<Performance>,
    audio: Res<Audio>,
) {
    // The sound is the clock whenever there is sound. Counting frames instead would
    // drift against the sample clock over the length of a piece, and a note landing
    // on the hit line a beat away from when it is heard is worse than no sound at all.
    match audio.player() {
        Some(player) => {
            let heard = player.position();
            match transport.pending_seek {
                // Still waiting for the device to act on a jump; hold the view where
                // the jump put it rather than snapping back.
                Some(target) if (heard - target).abs() > 0.25 => {}
                _ => {
                    transport.pending_seek = None;
                    transport.position = heard;
                }
            }
        }
        None if transport.playing => {
            transport.position += time.delta_secs_f64() * transport.speed;
        }
        None => return,
    }

    let end = performance.timeline.duration + 1.5;
    if transport.position > end {
        let looping = transport.looping;
        transport.playing = looping;
        let target = if looping { LEAD_IN } else { end };
        transport.seek_to(target);
        if let Some(player) = audio.player() {
            player.seek(target);
            player.set_playing(looping);
        }
    }
}

/// Play, pause, scrub and change speed.
fn handle_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut transport: ResMut<Transport>,
    audio: Res<Audio>,
) {
    let before = (transport.playing, transport.speed);
    let mut jump = None;

    if keys.just_pressed(KeyCode::Space) {
        transport.playing = !transport.playing;
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        jump = Some(transport.position - 2.0);
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        jump = Some(transport.position + 2.0);
    }
    if keys.just_pressed(KeyCode::Home) {
        jump = Some(LEAD_IN);
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        transport.speed = (transport.speed * 1.25).min(4.0);
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        transport.speed = (transport.speed / 1.25).max(0.1);
    }
    if keys.just_pressed(KeyCode::KeyL) {
        transport.looping = !transport.looping;
    }

    if let Some(target) = jump {
        transport.seek_to(target);
    }
    if let Some(player) = audio.player() {
        if let Some(target) = jump {
            player.seek(target);
        }
        if transport.playing != before.0 {
            player.set_playing(transport.playing);
        }
        if transport.speed != before.1 {
            player.set_speed(transport.speed);
        }
    }
}

/// Where a bar sits in the lane at a given moment: `(bottom, height)`.
fn bar_extent(layout: &Layout, bar: &NoteBar, now: f64) -> (f32, f32) {
    let bottom = layout.note_y(bar.start - now);
    let height = layout.note_height(bar.duration).max(6.0);
    (bottom, height)
}

/// Slide the note bars down the lane.
fn move_notes(
    transport: Res<Transport>,
    performance: Res<Performance>,
    bars: Query<(&NoteBar, &mut Transform, &mut Visibility)>,
) {
    let layout = &performance.layout;
    let now = transport.position;
    for (bar, mut transform, mut visibility) in bars {
        let (bottom, height) = bar_extent(layout, bar, now);
        let top = bottom + height;

        // Only moved, never scaled: the mesh is already the right length. It is drawn
        // behind the keys, so the keyboard hides whatever has passed the hit line.
        transform.translation.y = bottom + height / 2.0;

        *visibility = if top > 0.0 && bottom < layout.lane_height {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Push the keys down as they are played, and light them by which hand is on them.
fn press_keys(
    transport: Res<Transport>,
    performance: Res<Performance>,
    note_colours: Res<NoteColours>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    keys: Query<(&KeyTop, &mut Transform, &MeshMaterial3d<StandardMaterial>)>,
) {
    let states = performance.timeline.key_depression(transport.position);
    for (key, mut transform, material) in keys {
        let depth = states.depth_of(key.midi);
        // Straight down, into the instrument. Nudging the key towards the player
        // instead — which is the obvious way to suggest a dip in a view from directly
        // overhead — pushes its front edge out past the front of the keyboard, where
        // there is nothing behind it but background, and a held note then looks like it
        // is leaking out of the bottom of the piano.
        //
        // What sells the press is the colour and the shadow, not the millimetre or two
        // of travel, so the travel can be honest.
        let black = is_black(key.midi);
        let dip = if black { BLACK_KEY_DIP } else { KEY_DIP };
        transform.translation = key.rest - Vec3::new(0.0, 0.0, depth * dip);

        // Tinting the material rather than swapping it keeps the painted highlight,
        // so a held key glows rather than becoming a flat coloured rectangle. The
        // emissive term is what makes it *glow* rather than merely change colour: it
        // pushes the key past full brightness where the bloom pass can find it, which
        // is the difference between a coloured key and a lit one.
        let handle = material.0.clone();
        if let Some(mut material) = materials.get_mut(&handle) {
            match states.hand_on(key.midi) {
                Some(hand) => {
                    let tint = note_colours.0[hand as usize];
                    let glow = if black { BLACK_KEY_GLOW } else { KEY_GLOW };
                    material.base_color = mix(Color::WHITE, tint, depth);
                    material.emissive = tint.to_linear() * (depth * glow);
                }
                None => {
                    material.base_color = Color::WHITE;
                    material.emissive = LinearRgba::BLACK;
                }
            }
        }
    }
}

/// Put a finger number on every note bar close enough to read.
///
/// Drawn as interface text projected from the world rather than as objects in the
/// scene, so the numbers stay the same size however far the view is zoomed. That is
/// what makes them legible on a bar only nine millimetres wide.
fn update_labels(
    mut commands: Commands,
    transport: Res<Transport>,
    performance: Res<Performance>,
    camera: Query<(&Camera, &GlobalTransform)>,
    bars: Query<(&NoteBar, &Transform)>,
    labels: Query<(&mut Node, &mut Text, &mut TextColor), With<FingerLabel>>,
) {
    let Ok((camera, camera_transform)) = camera.single() else {
        return;
    };
    // Two coordinate systems meet here. `world_to_viewport` answers in window
    // coordinates — it adds the viewport's own offset — while interface nodes are
    // laid out inside the camera's viewport, which the menu bar has pushed down the
    // window. Taking the offset back off is what keeps a digit on its note.
    let inset = camera
        .logical_viewport_rect()
        .map(|rect| rect.min)
        .unwrap_or(Vec2::ZERO);
    let layout = &performance.layout;
    let now = transport.position;

    // Which bars want a number, nearest the keyboard first: those are the ones the
    // player is about to need.
    let mut wanted: Vec<(f32, u8, Hand, Vec3)> = Vec::new();
    for (bar, transform) in &bars {
        let Some(finger) = bar.finger else { continue };
        let (bottom, height) = bar_extent(layout, bar, now);
        let top = bottom + height;
        // Only the part still above the keys can carry a number.
        let visible_bottom = bottom.max(0.0);
        let visible_height = top - visible_bottom;
        if visible_height < MIN_LABELLED_HEIGHT_MM || bottom > layout.lane_height {
            continue;
        }
        // Ride near the leading edge of what is still visible, so the digit stays
        // with the note as it is consumed at the hit line.
        let anchor = Vec3::new(
            transform.translation.x,
            visible_bottom + (visible_height / 2.0).min(13.0),
            LANE_Z + 1.0,
        );
        wanted.push((visible_bottom, finger, bar.hand, anchor));
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
    wanted.truncate(MAX_LABELS);

    // Reuse a pool of text nodes rather than spawning and despawning every frame.
    let mut slot = 0;
    for (mut node, mut text, mut colour) in labels {
        match wanted.get(slot) {
            Some((_, finger, hand, anchor)) => {
                match camera.world_to_viewport(camera_transform, *anchor) {
                    Ok(screen) => {
                        let screen = screen - inset;
                        node.display = Display::Flex;
                        node.left = Val::Px(screen.x - LABEL_FONT_PX * 0.28);
                        node.top = Val::Px(screen.y - LABEL_FONT_PX * 0.62);
                        let digit = finger.to_string();
                        if text.0 != digit {
                            text.0 = digit;
                        }
                        // Dark digits read better on the light note bars, and the
                        // bar's own colour already says which hand it is.
                        colour.0 = match hand {
                            Hand::Left => Color::srgb(0.03, 0.11, 0.22),
                            Hand::Right => Color::srgb(0.20, 0.08, 0.01),
                        };
                    }
                    Err(_) => node.display = Display::None,
                }
            }
            None => node.display = Display::None,
        }
        slot += 1;
    }

    // Grow the pool towards what the piece needs. A few per frame is plenty: the
    // shortfall only ever appears in the first moments of playback.
    for _ in slot..wanted.len().min(slot + 16) {
        commands.spawn((
            Text::new(""),
            TextFont {
                font_size: bevy::text::FontSize::Px(LABEL_FONT_PX),
                ..default()
            },
            TextColor(Color::WHITE),
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                ..default()
            },
            FingerLabel,
        ));
    }
}

/// Keep the camera inside whatever the interface has left it.
fn fit_camera_viewport(
    inset: Res<ViewportInset>,
    windows: Query<&Window>,
    cameras: Query<&mut Camera, With<Camera3d>>,
) {
    // With no interface over the scene there is nothing to make room for, and the
    // camera is left to fill whatever it is drawing into. That matters during a video
    // export, where what it is drawing into is an off-screen image that has nothing
    // to do with the size of the window.
    if inset.top <= 0.5 && inset.bottom <= 0.5 {
        for mut camera in cameras {
            if camera.viewport.is_some() {
                camera.viewport = None;
            }
        }
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let scale = window.scale_factor();
    let top = (inset.top * scale).round() as u32;
    let bottom = (inset.bottom * scale).round() as u32;
    let size = window.physical_size();
    if size.x == 0 || size.y <= top + bottom {
        return;
    }

    let wanted = bevy::camera::Viewport {
        physical_position: UVec2::new(0, top),
        physical_size: UVec2::new(size.x, size.y - top - bottom),
        ..default()
    };
    for mut camera in cameras {
        // Only when it changes: assigning the viewport every frame would mark the
        // camera as changed and make everything downstream of it recompute.
        let unchanged = camera.viewport.as_ref().is_some_and(|current| {
            current.physical_position == wanted.physical_position
                && current.physical_size == wanted.physical_size
        });
        if !unchanged {
            camera.viewport = Some(wanted.clone());
        }
    }
}

/// Blend two colours.
fn mix(a: Color, b: Color, t: f32) -> Color {
    let (a, b) = (a.to_linear(), b.to_linear());
    let t = t.clamp(0.0, 1.0);
    Color::LinearRgba(bevy::color::LinearRgba {
        red: a.red + (b.red - a.red) * t,
        green: a.green + (b.green - a.green) * t,
        blue: a.blue + (b.blue - a.blue) * t,
        alpha: 1.0,
    })
}

/// Capture a single frame at a given moment instead of playing.
///
/// Used to check the view without watching it, and as the basis of the offline video
/// export, which is the same thing repeated at a fixed timestep.
#[derive(Resource, Debug, Clone)]
pub struct Capture {
    /// The moment to capture, in seconds.
    pub at: f64,
    /// Where to write the image.
    pub path: std::path::PathBuf,
    /// Frames to let pass before capturing, so assets have loaded and the scene has
    /// settled.
    pub warmup: u32,
}

/// Counts down the warmup frames.
#[derive(Resource, Default)]
struct CaptureState {
    frames: u32,
    taken: bool,
}

/// Freeze the playhead at the requested moment and grab a frame.
fn capture_frame(
    mut commands: Commands,
    capture: Res<Capture>,
    mut state: ResMut<CaptureState>,
    mut transport: ResMut<Transport>,
    mut exit: MessageWriter<AppExit>,
) {
    transport.playing = false;
    transport.position = capture.at;
    state.frames += 1;

    if state.frames < capture.warmup || state.taken {
        // The screenshot travels through the render world and comes back a few frames
        // later, so the app has to stay alive long enough to receive it.
        if state.taken && state.frames > capture.warmup + 40 {
            exit.write(AppExit::Success);
        }
        return;
    }
    commands
        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
        .observe(bevy::render::view::screenshot::save_to_disk(capture.path.clone()));
    state.taken = true;
}

/// Render one frame of the piece to an image file and exit.
pub fn capture(
    performance: Performance,
    capture: Capture,
    settings: crate::SessionSettings,
    path: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    let assets = performance.assets_root.clone();
    App::new()
        .add_plugins(default_plugins("OpenNote", &assets))
        .insert_resource(NoteColours(settings.note_colours))
        .insert_resource(performance)
        .insert_resource(capture)
        .init_resource::<Audio>()
        .init_resource::<CaptureState>()
        .insert_resource(crate::ui::Session::new(settings, path))
        .init_resource::<crate::ui::Reload>()
        .add_plugins(VisualizerPlugin)
        // The screenshot is of the program, menus and all, so that what it shows is
        // what somebody looking at the window would see.
        .add_plugins(crate::ui::MenuPlugin)
        .add_systems(Update, capture_frame)
        .run();
    Ok(())
}

/// The plugin set, with the window titled and the asset root pointed at wherever the
/// hand models actually are.
///
/// Bevy resolves its asset root relative to the crate being run, not to the working
/// directory, so a relative path here would break the moment the binary is launched
/// with `cargo run -p`. The caller resolves the directory to an absolute path and it
/// is used verbatim.
pub(crate) fn default_plugins(title: &str, assets: &std::path::Path) -> impl PluginGroup {
    // The command line installs its own tracing subscriber before it gets here, and
    // two global subscribers cannot coexist: Bevy's would fail to install and take
    // every renderer warning down with it.
    let mut plugins = DefaultPlugins
        .build()
        .disable::<bevy::log::LogPlugin>()
        .set(WindowPlugin {
            primary_window: Some(Window {
                title: title.to_string(),
                resolution: (1600u32, 900u32).into(),
                ..default()
            }),
            ..default()
        });
    if let Some(path) = assets.to_str() {
        plugins = plugins.set(AssetPlugin {
            file_path: path.to_string(),
            ..default()
        });
    }
    plugins
}

/// Open a window and play the piece, with menus.
pub fn run(
    performance: Performance,
    settings: crate::SessionSettings,
    path: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    let title = performance
        .timeline
        .title
        .clone()
        .unwrap_or_else(|| "OpenNote".into());

    let assets = performance.assets_root.clone();
    let audio = Audio::open(&performance.timeline, performance.soundfont.clone());
    App::new()
        .add_plugins(default_plugins(&format!("OpenNote — {title}"), &assets))
        .insert_resource(performance)
        .insert_resource(audio)
        .insert_resource(crate::ui::Session::new(settings, path))
        .init_resource::<crate::ui::Reload>()
        .add_plugins(VisualizerPlugin)
        .add_plugins(crate::ui::MenuPlugin)
        .run();
    Ok(())
}
