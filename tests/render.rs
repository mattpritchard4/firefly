//! Actual GPU compilation and composed-App lifecycle proof. One test owns the device.
use bevy::{
    camera::RenderTarget,
    prelude::*,
    render::{
        RenderApp,
        pipelined_rendering::PipelinedRenderingPlugin,
        render_resource::{
            CachedPipelineState, Extent3d, PipelineCache, SpecializedRenderPipeline,
            TextureDimension, TextureFormat, TextureUsages,
        },
        renderer::RenderDevice,
        view::{
            Msaa,
            screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
        },
    },
    shader::ShaderCacheError,
    window::ExitCondition,
    winit::WinitPlugin,
};
use bevy_firefly::{pipelines::*, prelude::*};
use std::{
    thread,
    time::{Duration, Instant},
};

#[derive(Resource)]
struct Capture(Image);

fn pump(app: &mut App, mut ready: impl FnMut(&mut App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        app.update();
        if ready(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "GPU/asset progress exceeded 120 seconds"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn compiled(app: &mut App) -> bool {
    let world = app.sub_app_mut(RenderApp).world_mut();
    let requested: Vec<_> = world
        .query::<&SpecializedApplicationPipeline>()
        .iter(world)
        .map(|pipeline| pipeline.id)
        .collect();
    let cache = world.resource::<PipelineCache>();
    if requested.iter().any(|id| {
        !matches!(
            cache.get_render_pipeline_state(*id),
            CachedPipelineState::Ok(_)
        )
    }) {
        return false;
    }
    let mut count = 0;
    for pipeline in cache.pipelines() {
        count += 1;
        match &pipeline.state {
            CachedPipelineState::Ok(_) => (),
            CachedPipelineState::Err(
                ShaderCacheError::ShaderNotLoaded(_)
                | ShaderCacheError::ShaderImportNotYetAvailable,
            ) => return false,
            CachedPipelineState::Err(error) => panic!("GPU pipeline failed: {error:?}"),
            _ => return false,
        }
    }
    count > 0 && cache.waiting_pipelines().next().is_none()
}

fn capture(app: &mut App, target: &Handle<Image>, name: &str) -> Image {
    app.world_mut().remove_resource::<Capture>();
    let mut entity = app.world_mut().spawn(Screenshot::image(target.clone()));
    entity.observe(|on: On<ScreenshotCaptured>, mut commands: Commands| {
        commands.insert_resource(Capture(on.image.clone()));
    });
    if let Ok(directory) = std::env::var("FIREFLY_EVIDENCE_DIR") {
        entity.observe(save_to_disk(
            std::path::Path::new(&directory).join(format!("{name}.png")),
        ));
    }
    pump(app, |app| app.world().contains_resource::<Capture>());
    app.world_mut().remove_resource::<Capture>().unwrap().0
}

#[test]
fn firefly_shader_variants_and_view_lifecycle() {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .disable::<WinitPlugin>()
            .disable::<PipelinedRenderingPlugin>(),
    );
    app.add_plugins(FireflyPlugin);
    app.insert_resource(ClearColor(Color::WHITE));
    app.finish();
    app.cleanup();

    let mut image = Image::new_fill(
        Extent3d {
            width: 256,
            height: 256,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        default(),
    );
    image.texture_descriptor.usage |= TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let camera = app
        .world_mut()
        .spawn((
            Camera2d,
            RenderTarget::Image(target.clone().into()),
            FireflyConfig::default(),
            Msaa::Off,
            bevy::render::view::Tonemapping::None,
        ))
        .id();
    let light = app
        .world_mut()
        .spawn((
            PointLight2d {
                color: Color::srgb(1.0, 0.0, 0.0),
                intensity: 3.0,
                radius: 180.0,
                ..default()
            },
            Transform::default(),
        ))
        .id();
    let occluder = app
        .world_mut()
        .spawn((
            Occluder2d::rectangle(12.0, 35.0),
            Transform::from_xyz(35.0, 0.0, 0.0),
        ))
        .id();
    // A real sprite must enter Firefly's stencil pass through Bevy 0.20's Mesh2d visibility class.
    app.world_mut().spawn((
        Sprite::from_color(Color::WHITE, Vec2::splat(16.0)),
        Transform::from_xyz(-25.0, 0.0, 1.0),
    ));
    pump(&mut app, compiled);

    let world = app.sub_app(RenderApp).world();
    assert!(
        world
            .resource::<RenderDevice>()
            .features()
            .contains(bevy::render::render_resource::WgpuFeatures::SHADER_F16),
        "UNAVAILABLE: Firefly shaders require SHADER_F16"
    );
    let cache = world.resource::<PipelineCache>();
    let creation = world.resource::<LightmapCreationPipeline>();
    let application = world.resource::<LightmapApplicationPipeline>();
    let sprite = world.resource::<SpritePipeline>();
    let mut variants = Vec::new();
    let methods = [
        LightPipelineKey::TONEMAP_METHOD_NONE,
        LightPipelineKey::TONEMAP_METHOD_LINEAR,
        LightPipelineKey::TONEMAP_METHOD_REINHARD,
        LightPipelineKey::TONEMAP_METHOD_REINHARD_LUMINANCE,
        LightPipelineKey::TONEMAP_METHOD_ACES_FITTED,
        LightPipelineKey::TONEMAP_METHOD_AGX,
        LightPipelineKey::TONEMAP_METHOD_SOMEWHAT_BORING_DISPLAY_TRANSFORM,
        LightPipelineKey::TONEMAP_METHOD_TONY_MC_MAPFACE,
        LightPipelineKey::TONEMAP_METHOD_BLENDER_FILMIC,
        LightPipelineKey::TONEMAP_METHOD_PBR_NEUTRAL,
    ];
    for format in [TextureFormat::Rgba8UnormSrgb, TextureFormat::Rgba16Float] {
        let base = LightPipelineKey::from_target_format(format);
        for (index, method) in methods.into_iter().enumerate() {
            let key = if index == 0 {
                base
            } else {
                base | method
                    | LightPipelineKey::TONEMAP_IN_SHADER
                    | LightPipelineKey::DEBAND_DITHER
            };
            variants.push(cache.queue_render_pipeline(creation.specialize(key)));
        }
        for combined in [LightPipelineKey::NONE, LightPipelineKey::COMBINE_LIGHTMAPS] {
            for filtering in [LightPipelineKey::NONE, LightPipelineKey::LIGHTMAP_FILTERING] {
                for samples in [1, 4] {
                    variants.push(cache.queue_render_pipeline(application.specialize(
                        base | combined | filtering | LightPipelineKey::from_msaa_samples(samples),
                    )));
                }
            }
        }
        for stencil in [
            SpritePipelineKey::NONE,
            SpritePipelineKey::ENABLED_32BIT_STENCIL,
        ] {
            variants.push(cache.queue_render_pipeline(
                sprite.specialize(SpritePipelineKey::from_target_format(format) | stencil),
            ));
        }
    }
    pump(&mut app, |app| {
        let cache = app.sub_app(RenderApp).world().resource::<PipelineCache>();
        variants.iter().all(|id| {
            matches!(
                cache.get_render_pipeline_state(*id),
                CachedPipelineState::Ok(_)
            )
        }) && compiled(app)
    });
    println!(
        "firefly_shader_variants: {} GPU pipelines compiled",
        variants.len()
    );

    assert!(app.sub_app(RenderApp).world().resource::<bevy::render::render_phase::ViewSortedRenderPhases<bevy_firefly::phases::SpritePhase>>().iter().any(|(_, phase)| !phase.items.is_empty()), "the real sprite must enter Firefly stencil/normal rendering");
    let lit = capture(&mut app, &target, "lit");
    if std::env::var_os("FIREFLY_DIAGNOSTICS").is_some() {
        println!(
            "main light visibility {:?}; camera {:?}",
            app.world().get::<ViewVisibility>(light),
            app.world().get::<Camera>(camera)
        );
        let world = app.sub_app_mut(RenderApp).world_mut();
        println!(
            "extracted lights {} configs {} views {} lightmaps {}",
            world
                .query::<&bevy_firefly::lights::ExtractedPointLight>()
                .iter(world)
                .count(),
            world.query::<&FireflyConfig>().iter(world).count(),
            world
                .query::<&bevy::render::view::ExtractedView>()
                .iter(world)
                .count(),
            world
                .query::<&bevy_firefly::LightMapTexture>()
                .iter(world)
                .count()
        );
        for vis in world
            .query::<&bevy::render::view::RenderVisibleEntities>()
            .iter(world)
        {
            println!(
                "visible lights {:?}",
                vis.get::<PointLight2d>().map(|v| v.iter_visible().count())
            );
        }
        println!(
            "lit maximum channel {:?}",
            lit.data.as_ref().unwrap().iter().step_by(4).max()
        );
    }
    assert!(
        lit.data
            .as_ref()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[0] > pixel[1].saturating_add(10))
            .count()
            > 100,
        "expected actual red light output"
    );
    app.world_mut().despawn(light);
    app.world_mut().despawn(occluder);
    for _ in 0..5 {
        app.update();
    }
    let dark = capture(&mut app, &target, "empty");
    assert!(
        dark.data
            .as_ref()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] < 3 && p[1] < 3 && p[2] < 3),
        "deleted light must leave no illumination"
    );

    // A real image-target resize must propagate through view textures and capture dimensions.
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .get_mut(&target)
        .unwrap()
        .resize(Extent3d {
            width: 320,
            height: 192,
            depth_or_array_layers: 1,
        });
    for _ in 0..5 {
        app.update();
    }
    let resized = capture(&mut app, &target, "resized");
    assert_eq!(resized.size(), UVec2::new(320, 192));

    // Match the existing fov example's target-sharing contract: no MSAA writeback from the source camera.
    app.world_mut()
        .get_mut::<Camera>(camera)
        .unwrap()
        .msaa_writeback = bevy::camera::MsaaWriteback::Off;
    // Exercise the combined arm with an actual second camera and a disjoint light layer.
    app.world_mut()
        .get_mut::<FireflyConfig>(camera)
        .unwrap()
        .combination_mode = CombinationMode::Add;
    let source = app
        .world_mut()
        .spawn((
            Camera2d,
            Camera {
                order: -1,
                output_mode: bevy::camera::CameraOutputMode::Skip,
                ..default()
            },
            RenderTarget::Image(target.clone().into()),
            FireflyConfig::default(),
            Msaa::Off,
            bevy::render::view::Tonemapping::None,
            bevy::camera::visibility::RenderLayers::layer(1),
            CombineLightmapTo(camera),
        ))
        .id();
    let blue = app
        .world_mut()
        .spawn((
            PointLight2d {
                color: Color::srgb(0.0, 0.0, 1.0),
                intensity: 3.0,
                radius: 180.0,
                ..default()
            },
            Transform::default(),
            bevy::camera::visibility::RenderLayers::layer(1),
        ))
        .id();
    pump(&mut app, compiled);
    let combined = capture(&mut app, &target, "combined");
    if std::env::var_os("FIREFLY_DIAGNOSTICS").is_some() {
        let world = app.sub_app_mut(RenderApp).world_mut();
        for (entity, config, requested, maps, source, view) in world
            .query::<(
                Entity,
                &FireflyConfig,
                Option<&SpecializedApplicationPipeline>,
                Option<&bevy_firefly::CombinedLightMapTextures>,
                Option<&bevy_firefly::data::ExtractedCombineLightmapTo>,
                Option<&bevy::render::view::ExtractedView>,
            )>()
            .iter(world)
        {
            println!(
                "combined view {entity:?}: config {:?} requested {:?} maps {} source {:?} extracted {}",
                config.combination_mode,
                requested.map(|p| (p.is_combined, p.id)),
                maps.is_some(),
                source.map(|s| (s.0, s.1)),
                view.is_some()
            );
        }
    }
    assert!(
        combined
            .data
            .as_ref()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[2] > p[0].saturating_add(10))
            .count()
            > 100,
        "second camera must contribute real blue illumination"
    );
    // The source remains live while its destination is disabled and restored.
    app.world_mut().entity_mut(camera).remove::<FireflyConfig>();
    app.update();
    let disabled_destination = capture(&mut app, &target, "disabled_destination");
    assert!(
        disabled_destination
            .data
            .as_ref()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] > 240 && p[1] > 240 && p[2] > 240)
    );
    app.world_mut().entity_mut(camera).insert(FireflyConfig {
        combination_mode: CombinationMode::Add,
        ..default()
    });
    pump(&mut app, compiled);
    let restored_destination = capture(&mut app, &target, "restored_destination");
    assert!(
        restored_destination
            .data
            .as_ref()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[2] > p[0].saturating_add(10))
    );
    app.world_mut()
        .get_mut::<FireflyConfig>(camera)
        .unwrap()
        .lightmap_size = LightmapSize::Fixed(UVec2::ZERO);
    app.update();
    capture(&mut app, &target, "invalid_destination");
    app.world_mut()
        .get_mut::<FireflyConfig>(camera)
        .unwrap()
        .lightmap_size = LightmapSize::Window;
    pump(&mut app, compiled);
    let recovered_destination = capture(&mut app, &target, "recovered_destination");
    assert!(
        recovered_destination
            .data
            .as_ref()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[2] > p[0].saturating_add(10))
    );
    let later_source = app
        .world_mut()
        .spawn((
            Camera2d,
            Camera {
                order: -2,
                output_mode: bevy::camera::CameraOutputMode::Skip,
                ..default()
            },
            RenderTarget::Image(target.clone().into()),
            FireflyConfig::default(),
            Msaa::Off,
            bevy::render::view::Tonemapping::None,
            bevy::camera::visibility::RenderLayers::layer(2),
            CombineLightmapTo(camera),
        ))
        .id();
    let green = app
        .world_mut()
        .spawn((
            PointLight2d {
                color: Color::srgb(0.0, 1.0, 0.0),
                intensity: 3.0,
                radius: 180.0,
                ..default()
            },
            Transform::default(),
            bevy::camera::visibility::RenderLayers::layer(2),
        ))
        .id();
    pump(&mut app, compiled);
    app.world_mut().get_mut::<Camera>(source).unwrap().is_active = false;
    app.update();
    let inactive = capture(&mut app, &target, "inactive_source");
    assert!(
        inactive
            .data
            .as_ref()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[1] > p[0].saturating_add(10)),
        "later active source must keep a valid array layer"
    );
    app.world_mut()
        .entity_mut(later_source)
        .remove::<CombineLightmapTo>();
    for _ in 0..3 {
        app.update();
    }
    let unlinked = capture(&mut app, &target, "unlinked_source");
    assert!(
        unlinked
            .data
            .as_ref()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] < 3 && p[1] < 3 && p[2] < 3),
        "unlinked source must not retain its destination binding"
    );
    app.world_mut().despawn(later_source);
    app.world_mut().despawn(green);
    app.world_mut().get_mut::<Camera>(source).unwrap().is_active = true;
    pump(&mut app, compiled);
    capture(&mut app, &target, "reactivated_source");
    app.world_mut().despawn(source);
    app.world_mut().despawn(blue);
    for _ in 0..5 {
        app.update();
    }
    let uncombined = capture(&mut app, &target, "uncombined");
    assert!(
        uncombined
            .data
            .as_ref()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] < 3 && p[1] < 3 && p[2] < 3),
        "retired source must not retain a combined lightmap"
    );

    // Remove/re-add the feature, then retire its camera.
    app.world_mut()
        .get_mut::<FireflyConfig>(camera)
        .unwrap()
        .lightmap_size = bevy_firefly::data::LightmapSize::Fixed(UVec2::new(80, 64));
    app.update();
    app.world_mut().entity_mut(camera).remove::<FireflyConfig>();
    app.update();
    let disabled = capture(&mut app, &target, "disabled");
    assert!(
        disabled
            .data
            .as_ref()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] > 240 && p[1] > 240 && p[2] > 240),
        "removing FireflyConfig must retire the lightmap, leaving the ordinary white scene"
    );
    app.world_mut()
        .entity_mut(camera)
        .insert(FireflyConfig::default());
    app.update();
    pump(&mut app, compiled);
    capture(&mut app, &target, "recreated");
    app.world_mut()
        .get_mut::<FireflyConfig>(camera)
        .unwrap()
        .lightmap_size = LightmapSize::Fixed(UVec2::ZERO);
    app.update();
    capture(&mut app, &target, "zero_extent");
    app.world_mut().get_mut::<Camera>(camera).unwrap().is_active = false;
    app.update();
    app.world_mut().despawn(camera);
    for _ in 0..5 {
        app.update();
    }
    // Shutdown also exercises pending pipeline tasks: enqueue a distinct valid variant, then drop App.
    let world = app.sub_app(RenderApp).world();
    let pipeline = world.resource::<LightmapApplicationPipeline>();
    world
        .resource::<PipelineCache>()
        .queue_render_pipeline(pipeline.specialize(LightPipelineKey::from_target_format(
            TextureFormat::Bgra8UnormSrgb,
        )));
    drop(app);
    println!(
        "firefly_view_lifecycle: captures, delete, resize, rebind and pending shutdown passed"
    );
}
