//! Source's HDR presentation (`mat_hdr_level`, docs/backlog.md "HDR
//! parity"). Level 0 (the default, what refcmp compares against) leaves
//! the cameras alone. At levels 1 and 2 the map loads with its HDR look
//! (`MapLook::hdr`, games/cs_source/bsp.rs) and the cameras that draw the
//! view (sky, world, view model) render in HDR into one shared texture;
//! the last of them adds bloom and, at level 2, auto exposure kept within
//! the map's `env_tonemap_controller` bounds, then a linear scale-and-clamp
//! tone map (hdr_tonemap.wgsl).
//!
//! Not specified (no spec yet; open questions in docs/backlog.md): the
//! game's exposure target and adaptation speed, its metering, and its
//! bloom filter. The values here are stand-ins.

use bevy::{
    asset::embedded_asset,
    camera::Hdr,
    core_pipeline::{
        FullscreenShader,
        schedule::{Core3d, Core3dSystems},
        tonemapping::tonemapping,
    },
    math::cubic_splines::LinearSpline,
    post_process::{
        auto_exposure::{AutoExposure, AutoExposureCompensationCurve, AutoExposurePlugin},
        bloom::Bloom,
    },
    prelude::*,
    render::{
        RenderApp, RenderStartup,
        extract_component::{
            ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin,
        },
        render_resource::{
            binding_types::{sampler, texture_2d, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderDevice, ViewQuery},
        view::{ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms},
    },
};

use super::FirstPersonCamera;
use crate::{
    console::{Console, resource_cvar},
    map::{ActiveMapLook, MapHdr, SkyboxCamera, ViewModelCamera},
};

/// Source's `mat_hdr_level`: 0 LDR, 1 bloom, 2 HDR lighting with auto
/// exposure and bloom. Read when a map loads (as in the game, a change
/// shows from the next map load on).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct HdrSettings {
    pub level: u8,
}

/// The average scene luminance auto exposure aims for (before the
/// controller's bounds). A stand-in: the game's target isn't specified.
pub const EXPOSURE_TARGET: f32 = 0.5;
/// Bloom at the game's default bloom scale (1). A stand-in.
const BLOOM_INTENSITY: f32 = 0.05;
/// Auto exposure's metering range, in stops of scene luminance.
const METER_RANGE: (f32, f32) = (-8.0, 8.0);

pub struct HdrPlugin;

impl Plugin for HdrPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HdrSettings>();
        resource_cvar::<HdrSettings, u8>(
            app,
            "mat_hdr_level",
            "0: LDR (default); 1: bloom; 2: HDR lighting, auto exposure and bloom on maps with HDR data. Applies from the next map load.",
            |s| &mut s.level,
        );
        app.world_mut().resource_mut::<Console>().archive("mat_hdr_level");
        app.add_systems(Update, hdr_cameras);

        // The render side only exists with a renderer (not in headless tests).
        if app.get_sub_app(RenderApp).is_none() {
            return;
        }
        embedded_asset!(app, "hdr_tonemap.wgsl");
        app.add_plugins((
            AutoExposurePlugin,
            ExtractComponentPlugin::<SourceTonemap>::default(),
            UniformComponentPlugin::<SourceTonemap>::default(),
        ));
        let render_app = app.sub_app_mut(RenderApp);
        render_app.add_systems(RenderStartup, init_pipeline).add_systems(
            Core3d,
            // After Bevy's tone mapping (off on these cameras), so after
            // auto exposure and bloom, which run before it.
            source_tonemap.after(tonemapping).in_set(Core3dSystems::PostProcess),
        );
    }
}

/// The `mat_hdr_level` a run starts with, for loading the `--map` before
/// the console runs: the last `mat_hdr_level` in config.cfg, autoexec.cfg
/// or the command line's `+commands`, in that order.
pub fn startup_level(console_lines: &[String]) -> u8 {
    let mut level = 0;
    let mut scan = |text: &str| {
        for line in text.lines() {
            for part in line.split(';') {
                let mut words = part.split_whitespace();
                if words.next().is_some_and(|w| w.eq_ignore_ascii_case("mat_hdr_level"))
                    && let Some(v) = words.next().and_then(|v| v.trim_matches('"').parse::<u8>().ok())
                {
                    level = v;
                }
            }
        }
    };
    for name in ["config.cfg", "autoexec.cfg"] {
        if let Some(text) = crate::console::read_cfg(name) {
            scan(&text);
        }
    }
    for line in console_lines {
        scan(line);
    }
    level.min(2)
}

/// The tone-map pass's settings, on the camera that runs it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, ExtractComponent, ShaderType)]
pub struct SourceTonemap {
    pub min_scale: f32,
    pub max_scale: f32,
    /// 1: scale by auto exposure; 0: scale 1.
    pub auto_exposure: f32,
    pub _pad: f32,
}

/// What a camera's HDR post-processing was set up from.
#[derive(Component, Clone, Debug, PartialEq)]
struct HdrPost(MapHdr);

/// Exposure bounds as scales; a minimum of 0 (de_nuke) means "no lower
/// bound", kept finite for the log domain.
fn scale_bounds(hdr: &MapHdr) -> Option<(f32, f32)> {
    hdr.exposure.map(|(lo, hi)| {
        let lo = lo.max(1.0 / 256.0);
        (lo, hi.max(lo))
    })
}

/// Bevy's auto exposure moves the exposure (stops) toward `curve(avg) -
/// avg`, `avg` being the scene's average log2 luminance. This curve makes
/// that `clamp(log2(target) - avg, log2(lo), log2(hi))`: the target
/// exposure stays within the controller's bounds, so it never drifts past
/// them.
pub fn compensation_points(target: f32, lo: f32, hi: f32) -> Vec<Vec2> {
    let (lo, hi, t) = (lo.log2(), hi.log2(), target.log2());
    let f = |avg: f32| (t - avg).clamp(lo, hi) + avg;
    let (min, max) = METER_RANGE;
    let mut xs = vec![min, (t - hi).clamp(min, max), (t - lo).clamp(min, max), max];
    xs.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    xs.into_iter().map(|x| Vec2::new(x, f(x))).collect()
}

#[allow(clippy::type_complexity)]
fn hdr_cameras(
    look: Option<Res<ActiveMapLook>>,
    first_person: Query<(Entity, &Camera, Option<&Children>), With<FirstPersonCamera>>,
    view_models: Query<&Camera, With<ViewModelCamera>>,
    cameras: Query<
        (Entity, Has<Hdr>, Option<&HdrPost>),
        Or<(
            With<FirstPersonCamera>,
            With<SkyboxCamera>,
            With<ViewModelCamera>,
            With<super::debug::UiCamera>,
        )>,
    >,
    mut curves: ResMut<Assets<AutoExposureCompensationCurve>>,
    args: Option<Res<super::ClientArgs>>,
    mut commands: Commands,
) {
    let hdr = look.and_then(|l| l.0.hdr.clone());
    // Captures (--views, --screenshot) settle within their few frames:
    // exposure closes most of the gap every frame instead of adapting at
    // the eye's pace, so a view's capture doesn't depend on frame times.
    let capture = args.is_some_and(|a| a.0.views.is_some() || a.0.screenshot.is_some());
    // The last camera drawing the view: the view model's when it draws,
    // else the world's.
    let post = first_person.iter().find(|(_, c, _)| c.is_active).map(|(e, _, children)| {
        children
            .into_iter()
            .flatten()
            .find(|c| view_models.get(**c).is_ok_and(|c| c.is_active))
            .copied()
            .unwrap_or(e)
    });
    for (entity, has_hdr, current) in &cameras {
        let mut e = commands.entity(entity);
        let Some(hdr) = hdr.as_ref() else {
            if has_hdr || current.is_some() {
                e.remove::<(Hdr, HdrPost, SourceTonemap, Bloom, AutoExposure)>();
            }
            continue;
        };
        if !has_hdr {
            e.insert(Hdr);
        }
        if Some(entity) != post {
            if current.is_some() {
                e.remove::<(HdrPost, SourceTonemap, Bloom, AutoExposure)>();
            }
            continue;
        }
        if current.map(|c| &c.0) == Some(hdr) {
            continue;
        }
        let bounds = scale_bounds(hdr);
        let (min_scale, max_scale) = bounds.unwrap_or((1.0, 1.0));
        e.insert((
            HdrPost(hdr.clone()),
            SourceTonemap {
                min_scale,
                max_scale,
                auto_exposure: if bounds.is_some() { 1.0 } else { 0.0 },
                _pad: 0.0,
            },
            Bloom {
                intensity: BLOOM_INTENSITY * hdr.bloom_scale.max(0.0),
                ..Bloom::OLD_SCHOOL
            },
        ));
        match bounds {
            Some((lo, hi)) => {
                let curve = AutoExposureCompensationCurve::from_curve(LinearSpline::new(compensation_points(
                    EXPOSURE_TARGET,
                    lo,
                    hi,
                )));
                let compensation_curve = match curve {
                    Ok(c) => curves.add(c),
                    Err(err) => {
                        warn!("auto exposure curve: {err}");
                        default()
                    }
                };
                let mut exposure = AutoExposure {
                    range: METER_RANGE.0..=METER_RANGE.1,
                    compensation_curve,
                    ..default()
                };
                if capture {
                    exposure.speed_brighten = 20.0;
                    exposure.speed_darken = 20.0;
                    exposure.exponential_transition_distance = 1.0;
                }
                e.insert(exposure);
            }
            None => {
                e.remove::<AutoExposure>();
            }
        }
    }
}

#[derive(Resource)]
struct TonemapPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    pipeline: CachedRenderPipelineId,
}

fn init_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    fullscreen_shader: Res<FullscreenShader>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "source_tonemap_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<SourceTonemap>(true),
                uniform_buffer::<ViewUniform>(true),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());
    let pipeline = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("source_tonemap_pipeline".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen_shader.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: asset_server.load("embedded://mashup/client/hdr_tonemap.wgsl"),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba16Float,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(TonemapPipeline {
        layout,
        sampler,
        pipeline,
    });
}

fn source_tonemap(
    view: ViewQuery<(&ViewTarget, &DynamicUniformIndex<SourceTonemap>, &ViewUniformOffset)>,
    pipeline: Option<Res<TonemapPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    settings: Res<ComponentUniforms<SourceTonemap>>,
    view_uniforms: Res<ViewUniforms>,
    mut ctx: RenderContext,
) {
    let Some(pipeline) = pipeline else { return };
    let (target, settings_index, view_offset) = view.into_inner();
    if target.main_texture_format() != TextureFormat::Rgba16Float {
        return;
    }
    let Some(render_pipeline) = pipeline_cache.get_render_pipeline(pipeline.pipeline) else {
        return;
    };
    let (Some(settings_binding), Some(view_binding)) = (settings.uniforms().binding(), view_uniforms.uniforms.binding())
    else {
        return;
    };
    let post = target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        "source_tonemap_bind_group",
        &pipeline_cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((post.source, &pipeline.sampler, settings_binding, view_binding)),
    );
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("source_tonemap_pass"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: post.destination,
            depth_slice: None,
            resolve_target: None,
            ops: Operations::default(),
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(render_pipeline);
    pass.set_bind_group(0, &bind_group, &[settings_index.index(), view_offset.offset]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposure_target_stays_within_the_controller_bounds() {
        // de_dust2: max 1 (its logic_auto), min 0.5 (the default).
        let points = compensation_points(EXPOSURE_TARGET, 0.5, 1.0);
        let spline = LinearSpline::new(points.clone());
        let curve = bevy::math::cubic_splines::CubicGenerator::to_curve(&spline).unwrap();
        for avg in [-7.5f32, -3.0, -1.2, -1.0, -0.5, 0.0, 2.0, 7.5] {
            // Find the curve's value at avg by sampling along it.
            let n = curve.segments().len() as f32;
            let y = (0..=4000)
                .map(|i| curve.position(n * i as f32 / 4000.0))
                .min_by(|a, b| (a.x - avg).abs().total_cmp(&(b.x - avg).abs()))
                .unwrap()
                .y;
            let exposure = y - avg;
            let want = (EXPOSURE_TARGET.log2() - avg).clamp(0.5f32.log2(), 0.0);
            assert!((exposure - want).abs() < 0.01, "avg {avg}: exposure {exposure}, want {want}");
        }
        // Increasing in x, as Bevy's curve builder requires.
        assert!(points.windows(2).all(|w| w[0].x < w[1].x), "{points:?}");
    }

    #[test]
    fn startup_level_takes_the_last_setting() {
        let lines = ["+map de_dust2".to_string(), "mat_hdr_level 1; mat_hdr_level \"2\"".to_string()];
        // config.cfg may set it too (the user's own); the command line wins.
        assert_eq!(startup_level(&lines), 2);
        assert_eq!(startup_level(&["mat_hdr_level 7".to_string()]), 2, "clamped");
    }
}
