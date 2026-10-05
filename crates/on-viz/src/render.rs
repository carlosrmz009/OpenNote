use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use on_fingering::biomech::BiomechWeights;
use on_hand::keyboard::{is_black, BLACK_KEY_HEIGHT, KEY_DIP, WHITE_KEY_LENGTH};
use on_hand::{Hand, HandProfile};
use tracing::warn;

use crate::layout::Layout;
use crate::skin;
use crate::timeline::{HandAnimator, Timeline};

const CAMERA_HEIGHT_MM: f32 = 600.0;

const VIEW_MARGIN: f32 = 1.06;

const LANE_Z: f32 = BLACK_KEY_HEIGHT + 4.0;

const NOTE_Z: f32 = -40.0;

const CASE_Z: f32 = -20.0;

const OCTAVE_LINE_Z: f32 = NOTE_Z - 1.0;

pub const LEAD_IN: f64 = -1.0;

pub(crate) const KEYBOARD_Y_MM: f32 = -WHITE_KEY_LENGTH;

const HAND_COLOURS: [Color; 2] = [
    Color::srgb(0.85, 0.012, 0.012),
    Color::srgb(0.045, 0.88, 1.0),
];

#[derive(Resource, Debug, Clone, Copy)]
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

const NOTE_LIGHT_LUMENS: f32 = 5_400.0;

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

const CRYSTAL_TILE_MM: f32 = 165.0;

const LABEL_FONT_PX: f32 = 15.0;

const NOTE_GLOW: f32 = 9.5;

const WHITE_KEY_DEPTH_MM: f32 = 14.0;

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

#[derive(Resource, Debug, Clone)]
pub struct Transport {
    pub position: f64,
    pub playing: bool,
    pub speed: f64,
    pub looping: bool,
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
    pub fn seek_to(&mut self, seconds: f64) {
        self.position = seconds;
        self.pending_seek = Some(seconds);
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

#[derive(Component)]
struct KeyTop {
    midi: u8,
    rest: Vec3,
}

#[derive(Component)]
struct NoteBar {
    start: f64,
    duration: f64,
    hand: Hand,
    finger: Option<u8>,
}

#[derive(Component)]
struct Spark;

#[derive(Component)]
struct NoteLight;

#[derive(Component)]
struct Flash {
    midi: u8,
    spread: f32,
}

#[derive(Resource)]
struct SparkColours {
    hot: [Handle<StandardMaterial>; 2],
    cool: [Handle<StandardMaterial>; 2],
}

#[derive(Component)]
struct FingerLabel;

pub struct VisualizerPlugin;

impl Plugin for VisualizerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Transport>()
            .init_resource::<ViewportInset>()
            .init_resource::<NoteColours>()
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

pub(crate) fn setup_camera(mut commands: Commands, performance: Res<Performance>) {
    let layout = &performance.layout;
    let (cx, cy) = layout.centre_mm();

    commands.spawn((
        Camera3d::default(),
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        camera_projection(layout),
        camera_placement(layout),
        bevy::post_process::bloom::Bloom {
            intensity: 0.30,
            prefilter: bevy::post_process::bloom::BloomPrefilter {
                threshold: 1.0,
                threshold_softness: 0.35,
            },
            composite_mode: bevy::post_process::bloom::BloomCompositeMode::Additive,
            low_frequency_boost: 0.05,
            max_mip_dimension: 512,
            ..bevy::post_process::bloom::Bloom::NATURAL
        },
        bevy::pbr::ScreenSpaceAmbientOcclusion::default(),
        bevy::anti_alias::taa::TemporalAntiAliasing::default(),
        Msaa::Off,
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 4_650.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(cx + 380.0, cy - 300.0, 1_050.0)
            .looking_at(Vec3::new(cx, cy, 0.0), Vec3::Z),
        bevy::light::CascadeShadowConfigBuilder {
            num_cascades: 2,
            first_cascade_far_bound: 900.0,
            maximum_distance: 2_400.0,
            ..default()
        }
        .build(),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 1_250.0,
            color: Color::srgb(0.86, 0.88, 1.0),
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(cx, cy - 1_100.0, 170.0).looking_at(Vec3::new(cx, cy, 30.0), Vec3::Z),
    ));
    commands.insert_resource(bevy::light::DirectionalLightShadowMap { size: 4096 });
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.52, 0.62, 0.90),
        brightness: 22.0,
        ..default()
    });
    commands.insert_resource(ClearColor(Color::BLACK));
}

fn camera_placement(layout: &Layout) -> Transform {
    let (cx, cy) = layout.centre_mm();
    Transform::from_xyz(cx, cy, CAMERA_HEIGHT_MM).looking_at(Vec3::new(cx, cy, 0.0), Vec3::Y)
}

fn camera_projection(layout: &Layout) -> Projection {
    Projection::Orthographic(OrthographicProjection {
        scaling_mode: bevy::camera::ScalingMode::AutoMin {
            min_width: layout.width_mm() * VIEW_MARGIN,
            min_height: layout.height_mm() * VIEW_MARGIN,
        },
        near: 0.0,
        far: CAMERA_HEIGHT_MM * 4.0,
        ..OrthographicProjection::default_3d()
    })
}

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

fn key_material(image: Handle<Image>, roughness: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(image),
        perceptual_roughness: roughness,
        reflectance: 0.35,
        ..default()
    }
}

fn note_material(image: Handle<Image>, tint: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: glowing(tint, NOTE_GLOW),
        base_color_texture: Some(image),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    }
}

fn glowing(tint: Color, brightness: f32) -> Color {
    let colour = tint.to_linear();
    Color::LinearRgba(LinearRgba {
        red: colour.red * brightness,
        green: colour.green * brightness,
        blue: colour.blue * brightness,
        alpha: 1.0,
    })
}

fn painted(image: Handle<Image>, tint: Color, alpha: AlphaMode) -> StandardMaterial {
    StandardMaterial {
        base_color: tint,
        base_color_texture: Some(image),
        unlit: true,
        alpha_mode: alpha,
        ..default()
    }
}

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

        let material = materials.add(key_material(
            if black { black_texture.clone() } else { white_texture.clone() },
            if black { 0.14 } else { 0.42 },
        ));

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

    let octave = materials.add(painted(
        images.add(skin::octave_line()),
        glowing(Color::srgb(0.62, 0.72, 0.95), OCTAVE_LINE_GLOW),
        AlphaMode::Add,
    ));
    let line = meshes.add(Rectangle::new(OCTAVE_LINE_WIDTH_MM, layout.lane_height));
    for midi in layout.keys() {
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

    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(layout.width_mm(), BACK_GLOW_DEPTH_MM))),
        MeshMaterial3d(materials.add(painted(
            images.add(skin::back_glow()),
            glowing(Color::srgb(0.0, 0.14, 1.0), BACK_GLOW_GLOW),
            AlphaMode::Add,
        ))),
        Transform::from_xyz(centre_x, BACK_GLOW_DEPTH_MM / 2.0, LANE_Z - 1.0),
    ));

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

fn note_mesh(width: f32, height: f32) -> Mesh {
    let cap = (width * skin::NOTE_CAP).min(height * 0.5);
    let middle = height - 2.0 * cap;
    let bands = ((middle / CRYSTAL_TILE_MM).round() as usize).max(1);
    let band = middle / bands as f32;

    let (half_width, half_height) = (width / 2.0, height / 2.0);
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

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
            Flash { midi, spread: (x1 - x0) / on_hand::keyboard::WHITE_KEY_WIDTH },
        ));
    }
    commands.insert_resource(FlashColours(colours));
}

#[derive(Resource)]
struct FlashColours([Handle<StandardMaterial>; 2]);

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
        let Some(hand) = states.hand_on(flash.midi).filter(|_| depth > 0.0) else {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        };
        transform.scale = Vec3::splat(flash.spread * depth.powf(0.55));
        *visibility = Visibility::Visible;

        let wanted = &colours.0[hand as usize];
        if material.0.id() != wanted.id() {
            material.0 = wanted.clone();
        }
    }
}

fn setup_note_lights(mut commands: Commands) {
    for _ in 0..MAX_NOTE_LIGHTS {
        commands.spawn((
            PointLight {
                intensity: 0.0,
                range: NOTE_LIGHT_RANGE_MM,
                radius: 12.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, NOTE_LIGHT_HEIGHT_MM),
            Visibility::Hidden,
            NoteLight,
        ));
    }
}

fn light_struck_keys(
    transport: Res<Transport>,
    performance: Res<Performance>,
    note_colours: Res<NoteColours>,
    lights: Query<(&mut PointLight, &mut Transform, &mut Visibility), With<NoteLight>>,
) {
    let layout = &performance.layout;
    let states = performance.timeline.key_depression(transport.position);
    let offset = Vec3::Y * KEYBOARD_Y_MM;

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
        let done = (note.end + f64::from(SPARK_TAIL_SECONDS))
            .min(note.restruck.unwrap_or(f64::INFINITY));
        if now < note.start || now >= done {
            continue;
        }
        let elapsed = (now - note.start) as f32;
        let throwing = (note.end.min(done) - note.start) as f32;

        let evaporation = (((done - now) as f32) / SPARK_TAIL_SECONDS)
            .clamp(0.0, 1.0)
            .powf(SPARK_EVAPORATE_SHAPE);

        let (x0, x1, _, _) = layout.key_rect(note.midi);
        let centre = (x0 + x1) / 2.0;
        let width = x1 - x0;

        for spark in 0..SPARKS_PER_NOTE {
            let Some((mut transform, mut visibility, mut material)) = slots.next() else {
                return;
            };
            *visibility = Visibility::Hidden;

            let since = elapsed - spark as f32 * stagger;
            if since < 0.0 {
                continue;
            }
            let turn = (since / SPARK_LIFE).floor();
            let age = since - turn * SPARK_LIFE;
            let seed = (index as u32)
                .wrapping_mul(0x9e37_79b9)
                .wrapping_add((spark as u32).wrapping_mul(0x85eb_ca6b))
                ^ (turn as u32).wrapping_mul(0xc2b2_ae35);

            let born = since - age;
            if born > throwing {
                continue;
            }
            if born > SPARK_EMIT_SECONDS
                && scramble(seed ^ 0x7f4a_7c15) > SPARK_SUSTAIN_SHARE
            {
                continue;
            }

            let strand = seed
                .wrapping_div(SPARK_FILAMENT)
                .wrapping_mul(0x9e37_79b9)
                ^ (turn as u32).wrapping_mul(0x85eb_ca6b);

            let reach =
                width * SPARK_WANDER_KEYS * (2.0 * scramble(strand ^ 0x94d0_49bb) - 1.0);
            let upward =
                SPARK_RISE_MM * (0.40 + 0.60 * scramble(strand ^ 0x5bf0_3635).powf(1.7));
            let size = SPARK_SIZE_MM.start
                + (SPARK_SIZE_MM.end - SPARK_SIZE_MM.start) * scramble(seed ^ 0x27d4_eb2f);
            let phase = scramble(seed ^ 0x1656_67b1) * std::f32::consts::TAU;

            let height = upward * age - 0.5 * SPARK_GRAVITY_MM * age * age;
            if height < 0.0 {
                continue;
            }
            let opened = (age / SPARK_LIFE).powf(SPARK_WANDER_SHAPE);
            let spread =
                reach * (SPARK_WANDER_AT_BIRTH + (1.0 - SPARK_WANDER_AT_BIRTH) * opened);
            let curl =
                (phase + age * SPARK_CURL_RATE).sin() * width * SPARK_CURL_KEYS * age;
            let lean = if note.id.0 % 2 == 0 { 1.0 } else { -1.0 };
            let drift = lean * width * SPARK_DRIFT_KEYS * age * age;

            let taper = (1.0 - age / SPARK_LIFE).clamp(0.0, 1.0).powf(0.45);
            let scale = size * taper * evaporation;
            if scale <= 0.0 {
                continue;
            }

            transform.translation =
                Vec3::new(centre + spread + curl + drift, height, LANE_Z + 2.0);
            transform.scale = Vec3::splat(scale);
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

    for (_, mut visibility, _) in slots {
        if *visibility != Visibility::Hidden {
            *visibility = Visibility::Hidden;
        }
    }
}

fn scramble(mut x: u32) -> f32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x as f32 / u32::MAX as f32
}

fn advance_transport(
    time: Res<Time>,
    mut transport: ResMut<Transport>,
    performance: Res<Performance>,
    audio: Res<Audio>,
) {
    match audio.player() {
        Some(player) => {
            let heard = player.position();
            match transport.pending_seek {
                Some(target) if (heard - target).abs() > 0.25 => {}
                _ => {
                    transport.pending_seek = None;
                    transport.position = follow(
                        transport.position,
                        heard,
                        time.delta_secs_f64(),
                        transport.speed,
                        transport.playing,
                    );
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

const CLOCK_SNAP_SECONDS: f64 = 0.15;

const CLOCK_SETTLE_SECONDS: f64 = 0.25;

pub(crate) fn follow(position: f64, heard: f64, dt: f64, speed: f64, playing: bool) -> f64 {
    let error = heard - position;
    if !playing || error.abs() > CLOCK_SNAP_SECONDS {
        return heard;
    }
    position + (dt * speed + error * (dt / CLOCK_SETTLE_SECONDS).min(1.0)).max(0.0)
}

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

fn bar_extent(layout: &Layout, bar: &NoteBar, now: f64) -> (f32, f32) {
    let bottom = layout.note_y(bar.start - now);
    let height = layout.note_height(bar.duration).max(6.0);
    (bottom, height)
}

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

        transform.translation.y = bottom + height / 2.0;

        *visibility = if top > 0.0 && bottom < layout.lane_height {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

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
        let black = is_black(key.midi);
        let dip = if black { BLACK_KEY_DIP } else { KEY_DIP };
        transform.translation = key.rest - Vec3::new(0.0, 0.0, depth * dip);

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
    let inset = camera
        .logical_viewport_rect()
        .map(|rect| rect.min)
        .unwrap_or(Vec2::ZERO);
    let layout = &performance.layout;
    let now = transport.position;

    let mut wanted: Vec<(f32, u8, Hand, Vec3)> = Vec::new();
    for (bar, transform) in &bars {
        let Some(finger) = bar.finger else { continue };
        let (bottom, height) = bar_extent(layout, bar, now);
        let top = bottom + height;
        let visible_bottom = bottom.max(0.0);
        let visible_height = top - visible_bottom;
        if visible_height < MIN_LABELLED_HEIGHT_MM || bottom > layout.lane_height {
            continue;
        }
        let anchor = Vec3::new(
            transform.translation.x,
            visible_bottom + (visible_height / 2.0).min(13.0),
            LANE_Z + 1.0,
        );
        wanted.push((visible_bottom, finger, bar.hand, anchor));
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
    wanted.truncate(MAX_LABELS);

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

fn fit_camera_viewport(
    inset: Res<ViewportInset>,
    windows: Query<&Window>,
    cameras: Query<&mut Camera, With<Camera3d>>,
) {
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
        let unchanged = camera.viewport.as_ref().is_some_and(|current| {
            current.physical_position == wanted.physical_position
                && current.physical_size == wanted.physical_size
        });
        if !unchanged {
            camera.viewport = Some(wanted.clone());
        }
    }
}

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

#[derive(Resource, Debug, Clone)]
pub struct Capture {
    pub at: f64,
    pub path: std::path::PathBuf,
    pub warmup: u32,
}

#[derive(Resource, Default)]
struct CaptureState {
    frames: u32,
    taken: bool,
}

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
        .add_plugins(crate::ui::MenuPlugin)
        .add_systems(Update, capture_frame)
        .run();
    Ok(())
}

pub(crate) fn default_plugins(title: &str, assets: &std::path::Path) -> impl PluginGroup {
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
    let note_colours = NoteColours(settings.note_colours);
    App::new()
        .add_plugins(default_plugins(&format!("OpenNote — {title}"), &assets))
        .insert_resource(note_colours)
        .insert_resource(performance)
        .insert_resource(audio)
        .insert_resource(crate::ui::Session::new(settings, path))
        .init_resource::<crate::ui::Reload>()
        .add_plugins(VisualizerPlugin)
        .add_plugins(crate::ui::MenuPlugin)
        .run();
    Ok(())
}

#[cfg(test)]
mod clock_tests {
    use super::follow;

    fn play(buffer: f64, frames: usize) -> Vec<f64> {
        let (frame, mut position, mut out) = (1.0 / 60.0, 0.0, Vec::new());
        for n in 1..=frames {
            let now = n as f64 * frame;
            let heard = (now / buffer).floor() * buffer;
            position = follow(position, heard, frame, 1.0, true);
            out.push(position);
        }
        out
    }

    #[test]
    fn the_clock_never_runs_backwards_between_audio_buffers() {
        let track = play(0.0107, 600);
        assert!(track.windows(2).all(|w| w[1] >= w[0]));
    }

    #[test]
    fn the_clock_moves_evenly_and_stays_with_the_audio() {
        let track = play(0.0107, 600);
        let steps: Vec<f64> = track.windows(2).skip(60).map(|w| w[1] - w[0]).collect();
        let (lo, hi) = steps.iter().fold((f64::MAX, f64::MIN), |(a, b), s| (a.min(*s), b.max(*s)));
        assert!(hi - lo < 0.004, "frame steps vary from {lo} to {hi}");
        assert!((track[599] - 10.0).abs() < 0.012, "{}", track[599]);
    }

    #[test]
    fn a_jump_in_the_audio_is_followed_at_once() {
        assert_eq!(follow(1.0, 5.0, 1.0 / 60.0, 1.0, true), 5.0);
        assert_eq!(follow(1.0, 1.2, 1.0 / 60.0, 1.0, false), 1.2);
    }
}
