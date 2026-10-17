use super::*;

#[derive(Resource, Debug, Clone)]
pub struct Capture {
    pub at: f64,
    pub path: std::path::PathBuf,
    pub warmup: u32,
}

#[derive(Resource, Default)]
pub(crate) struct CaptureState {
    pub(crate) frames: u32,
    pub(crate) taken: bool,
}

pub(crate) fn capture_frame(
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
    let mut plugins = DefaultPlugins.build().disable::<bevy::log::LogPlugin>().set(WindowPlugin {
        primary_window: Some(Window {
            title: title.to_string(),
            resolution: (1600u32, 900u32).into(),
            ..default()
        }),
        ..default()
    });
    if let Some(path) = assets.to_str() {
        plugins = plugins.set(AssetPlugin { file_path: path.to_string(), ..default() });
    }
    plugins
}

pub fn run(
    performance: Performance,
    settings: crate::SessionSettings,
    path: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    let title = performance.timeline.title.clone().unwrap_or_else(|| "OpenNote".into());

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
