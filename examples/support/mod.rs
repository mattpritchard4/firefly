//! Opt-in native render capture for the existing examples. No normal-launch changes.
use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};
use bevy_firefly::prelude::{Occluder2d, PointLight2d};

#[derive(Resource)]
struct Remaining(usize);

pub fn install(app: &mut App) {
    let Some(path) = std::env::var_os("FIREFLY_CAPTURE_FILE") else {
        return;
    };
    let seconds = std::env::var("FIREFLY_CAPTURE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite() && (1.0..=120.0).contains(value))
        .unwrap_or(5.0);
    let path = std::path::PathBuf::from(path);
    app.add_systems(
        Update,
        move |mut commands: Commands,
              time: Res<Time<Real>>,
              mut requested: Local<bool>,
              lights: Query<(), With<PointLight2d>>,
              occluders: Query<(), With<Occluder2d>>,
              windows: Query<(Entity, &Window)>| {
            if !*requested && time.elapsed_secs() >= seconds {
                *requested = true;
                println!(
                    "Native capture: elapsed={}s lights={} occluders={}",
                    time.elapsed_secs(),
                    lights.iter().count(),
                    occluders.iter().count()
                );
                commands.insert_resource(Remaining(windows.iter().count()));
                for (entity, window) in &windows {
                    println!(
                        "Capture window {:?}: logical={}x{} physical={}x{} scale={}",
                        window.title,
                        window.width(),
                        window.height(),
                        window.physical_width(),
                        window.physical_height(),
                        window.scale_factor()
                    );
                    let output = if windows.iter().count() == 1 {
                        path.clone()
                    } else {
                        let title: String = window
                            .title
                            .chars()
                            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                            .take(64)
                            .collect();
                        path.with_file_name(format!(
                            "{}-{title}.png",
                            path.file_stem().unwrap_or_default().to_string_lossy()
                        ))
                    };
                    commands
                        .spawn(Screenshot::window(entity))
                        .observe(save_to_disk(output))
                        .observe(
                            |_: On<bevy::render::view::screenshot::ScreenshotCaptured>,
                             mut remaining: ResMut<Remaining>,
                             mut exit: MessageWriter<AppExit>| {
                                remaining.0 -= 1;
                                if remaining.0 == 0 {
                                    exit.write(AppExit::Success);
                                }
                            },
                        );
                }
            }
        },
    );
}
