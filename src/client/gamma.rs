//! The screen's brightness, `mat_monitorgamma` (the video tab's "Adjust
//! brightness levels...", `OptionsSubVideoGammaDlg.res`): CS:S's 2.2 by
//! default, 1.6 (LIGHT, the slider's left) to 2.6 (DARK). Source corrects
//! for the monitor's gamma; here the finished frame (world, HUD and menus,
//! as a display ramp would) is raised to `mat_monitorgamma / 2.2` in
//! display space, so 1.6 lightens the mid-tones, 2.6 darkens them and
//! black and white stay put. At 2.2 the pass doesn't run.
//!
//! The pass runs on the UI camera (`debug::UiCamera`, the last to draw)
//! after its UI and before the frame goes to the window. Not specified:
//! the game's exact ramp (a reference capture at 1.6 and 2.6 would settle
//! it; docs/plans/active/ui-parity.md).

use bevy::{
    asset::embedded_asset,
    core_pipeline::{FullscreenShader, schedule::Core2d, upscaling::upscaling},
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
        view::ViewTarget,
    },
};

use super::debug::UiCamera;
use crate::console::{Console, ConsoleAppExt};

/// CS:S's `mat_monitorgamma` default, and its slider's range.
pub const DEFAULT_GAMMA: f32 = 2.2;
pub const GAMMA_RANGE: (f32, f32) = (1.6, 2.6);

/// `mat_monitorgamma`.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct MonitorGammaSetting(pub f32);

impl Default for MonitorGammaSetting {
    fn default() -> Self {
        Self(DEFAULT_GAMMA)
    }
}

impl MonitorGammaSetting {
    /// The power a display-space value is raised to (1 at the default).
    pub fn exponent(self) -> f32 {
        self.0.clamp(GAMMA_RANGE.0, GAMMA_RANGE.1) / DEFAULT_GAMMA
    }
}

/// The pass's setting, on the camera that runs it. A power applies the
/// same in linear and display space (both are powers of each other), so
/// the shader raises the linear colour to `exponent`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, ExtractComponent, ShaderType)]
pub struct MonitorGamma {
    pub exponent: f32,
    pub _pad: Vec3,
}

pub struct GammaPlugin;

impl Plugin for GammaPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MonitorGammaSetting>()
            .add_systems(PostUpdate, gamma_camera);
        app.console_cvar(
            "mat_monitorgamma",
            "Screen brightness: 1.6 (light) to 2.6 (dark), 2.2 by default (Video > Adjust brightness levels).",
            &DEFAULT_GAMMA.to_string(),
            |w| w.get_resource::<MonitorGammaSetting>().map(|g| g.0.to_string()),
            |w, v| {
                let v: f32 = v.trim().parse().map_err(|_| format!("bad value \"{v}\""))?;
                w.get_resource_mut::<MonitorGammaSetting>().ok_or("not available")?.0 =
                    v.clamp(GAMMA_RANGE.0, GAMMA_RANGE.1);
                Ok(())
            },
        );
        app.world_mut().resource_mut::<Console>().archive("mat_monitorgamma");

        // The render side only exists with a renderer (not in headless tests).
        if app.get_sub_app(RenderApp).is_none() {
            return;
        }
        embedded_asset!(app, "gamma.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<MonitorGamma>::default(),
            UniformComponentPlugin::<MonitorGamma>::default(),
        ));
        let render_app = app.sub_app_mut(RenderApp);
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Core2d, gamma_pass.after(bevy::ui_render::ui_pass).before(upscaling));
    }
}

/// The UI camera carries the pass while the gamma isn't the default.
fn gamma_camera(
    setting: Res<MonitorGammaSetting>,
    cameras: Query<(Entity, Option<&MonitorGamma>), With<UiCamera>>,
    mut commands: Commands,
) {
    let exponent = setting.exponent();
    for (e, current) in &cameras {
        if (exponent - 1.0).abs() < 1e-4 {
            if current.is_some() {
                commands.entity(e).remove::<MonitorGamma>();
            }
        } else if current.is_none_or(|c| (c.exponent - exponent).abs() > 1e-6) {
            commands.entity(e).insert(MonitorGamma {
                exponent,
                _pad: Vec3::ZERO,
            });
        }
    }
}

#[derive(Resource)]
struct GammaPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    /// One per target format: the window's (sRGB) and HDR's.
    pipelines: [(TextureFormat, CachedRenderPipelineId); 2],
}

fn init_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    fullscreen_shader: Res<FullscreenShader>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "monitor_gamma_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<MonitorGamma>(true),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());
    let shader = asset_server.load("embedded://mashup/client/gamma.wgsl");
    let pipelines = [TextureFormat::Rgba8UnormSrgb, TextureFormat::Rgba16Float].map(|format| {
        let id = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("monitor_gamma_pipeline".into()),
            layout: vec![layout.clone()],
            vertex: fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: shader.clone(),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        });
        (format, id)
    });
    commands.insert_resource(GammaPipeline {
        layout,
        sampler,
        pipelines,
    });
}

fn gamma_pass(
    view: ViewQuery<(&ViewTarget, &DynamicUniformIndex<MonitorGamma>)>,
    pipeline: Option<Res<GammaPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    settings: Res<ComponentUniforms<MonitorGamma>>,
    mut ctx: RenderContext,
) {
    let Some(pipeline) = pipeline else { return };
    let (target, index) = view.into_inner();
    let format = target.main_texture_format();
    let Some(id) = pipeline.pipelines.iter().find(|(f, _)| *f == format).map(|(_, id)| *id) else {
        return;
    };
    let Some(render_pipeline) = pipeline_cache.get_render_pipeline(id) else {
        return;
    };
    let Some(binding) = settings.uniforms().binding() else {
        return;
    };
    let post = target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        "monitor_gamma_bind_group",
        &pipeline_cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((post.source, &pipeline.sampler, binding)),
    );
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("monitor_gamma_pass"),
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
    pass.set_bind_group(0, &bind_group, &[index.index()]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamma_is_css_and_clamped_to_its_slider() {
        let mut app = App::new();
        app.add_plugins(crate::console::ConsolePlugin).add_plugins(GammaPlugin);
        let cvar = app
            .world()
            .resource::<Console>()
            .cvar("mat_monitorgamma")
            .cloned()
            .unwrap();
        assert_eq!(cvar.default, "2.2");
        assert!(cvar.archive, "saved in config.cfg");
        assert_eq!(
            MonitorGammaSetting::default().exponent(),
            1.0,
            "the default changes nothing"
        );
        // Light at the slider's left: a power under 1 raises mid-tones.
        let light = MonitorGammaSetting(1.6).exponent();
        assert!(light < 1.0 && 0.5f32.powf(light) > 0.5);
        let dark = MonitorGammaSetting(2.6).exponent();
        assert!(dark > 1.0 && 0.5f32.powf(dark) < 0.5);
        app.world_mut().resource_mut::<Console>().submit("mat_monitorgamma 9");
        app.update();
        assert_eq!(app.world().resource::<MonitorGammaSetting>().0, 2.6, "clamped");
        // The UI camera carries the pass only off the default.
        let camera = app.world_mut().spawn(UiCamera).id();
        app.update();
        assert!(
            app.world()
                .get::<MonitorGamma>(camera)
                .is_some_and(|g| (g.exponent - 2.6 / 2.2).abs() < 1e-5)
        );
        app.world_mut().resource_mut::<Console>().submit("mat_monitorgamma 2.2");
        app.update();
        app.update();
        assert!(app.world().get::<MonitorGamma>(camera).is_none());
    }
}
