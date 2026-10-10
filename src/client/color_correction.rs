//! Source's color correction as a post-process (`mat_colorcorrection`,
//! default 1): the map's blended colour table (`map::color_correction`)
//! applied to the finished frame by the last camera drawing the view (the
//! view model's when it draws, else the world's), after the tone map. No
//! pass while no table has weight.

use bevy::{
    asset::embedded_asset,
    core_pipeline::{
        FullscreenShader,
        schedule::{Core3d, Core3dSystems},
        tonemapping::tonemapping,
    },
    prelude::*,
    render::{
        RenderApp, RenderStartup,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::{
            binding_types::{sampler, texture_2d, texture_3d},
            *,
        },
        renderer::{RenderContext, RenderDevice, ViewQuery},
        texture::GpuImage,
        view::ViewTarget,
    },
};

use super::FirstPersonCamera;
use crate::{
    console::{Console, resource_cvar},
    map::{ViewModelCamera, color_correction::ColorCorrectionLut},
};

/// `mat_colorcorrection`: 1 applies maps' color correction, 0 doesn't.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ColorCorrectionSettings {
    pub enabled: u8,
}

impl Default for ColorCorrectionSettings {
    fn default() -> Self {
        Self { enabled: 1 }
    }
}

/// On the camera that runs the pass this frame.
#[derive(Component, Clone, Copy, Debug, Default, ExtractComponent)]
pub struct ColorCorrectionPass;

/// The blended table, for the render world.
#[derive(Resource, Clone, Debug, ExtractResource)]
struct ColorCorrectionImage(Handle<Image>);

pub struct ColorCorrectionPlugin;

impl Plugin for ColorCorrectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ColorCorrectionSettings>();
        resource_cvar::<ColorCorrectionSettings, u8>(
            app,
            "mat_colorcorrection",
            "1: apply the map's color_correction tables (default); 0: off.",
            |s| &mut s.enabled,
        );
        app.world_mut().resource_mut::<Console>().archive("mat_colorcorrection");
        app.add_systems(Update, pass_camera);

        // The render side only exists with a renderer (not in headless tests).
        if app.get_sub_app(RenderApp).is_none() {
            return;
        }
        embedded_asset!(app, "color_correction.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<ColorCorrectionPass>::default(),
            ExtractResourcePlugin::<ColorCorrectionImage>::default(),
        ));
        let render_app = app.sub_app_mut(RenderApp);
        render_app.add_systems(RenderStartup, init_pipeline).add_systems(
            Core3d,
            color_correct
                .after(tonemapping)
                .after(super::hdr::SourceTonemapSet)
                .in_set(Core3dSystems::PostProcess),
        );
    }
}

/// Put the pass on the last camera drawing the view while a table has
/// weight and the cvar is on; off every other camera.
fn pass_camera(
    settings: Res<ColorCorrectionSettings>,
    lut: Option<Res<ColorCorrectionLut>>,
    image: Option<Res<ColorCorrectionImage>>,
    first_person: Query<(Entity, &Camera, Option<&Children>), With<FirstPersonCamera>>,
    view_models: Query<&Camera, With<ViewModelCamera>>,
    marked: Query<Entity, With<ColorCorrectionPass>>,
    mut commands: Commands,
) {
    let lut = lut.filter(|l| l.active && settings.enabled != 0);
    match (&lut, &image) {
        (Some(l), Some(i)) if i.0 == l.image => {}
        (Some(l), _) => commands.insert_resource(ColorCorrectionImage(l.image.clone())),
        (None, Some(_)) => commands.remove_resource::<ColorCorrectionImage>(),
        (None, None) => {}
    }
    let post = lut.and_then(|_| {
        first_person
            .iter()
            .find(|(_, c, _)| c.is_active)
            .map(|(e, _, children)| {
                children
                    .into_iter()
                    .flatten()
                    .find(|c| view_models.get(**c).is_ok_and(|c| c.is_active))
                    .copied()
                    .unwrap_or(e)
            })
    });
    for e in &marked {
        if Some(e) != post {
            commands.entity(e).remove::<ColorCorrectionPass>();
        }
    }
    if let Some(e) = post
        && !marked.contains(e)
    {
        commands.entity(e).insert(ColorCorrectionPass);
    }
}

#[derive(Resource)]
struct ColorCorrectionPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    /// For HDR (Rgba16Float) and LDR (sRGB 8-bit) view targets.
    hdr: CachedRenderPipelineId,
    ldr: CachedRenderPipelineId,
}

fn init_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    fullscreen_shader: Res<FullscreenShader>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "color_correction_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_3d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());
    let shader = asset_server.load("embedded://mashup/client/color_correction.wgsl");
    let pipeline = |format: TextureFormat| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("color_correction_pipeline".into()),
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
        })
    };
    let hdr = pipeline(TextureFormat::Rgba16Float);
    let ldr = pipeline(TextureFormat::Rgba8UnormSrgb);
    commands.insert_resource(ColorCorrectionPipeline {
        layout,
        sampler,
        hdr,
        ldr,
    });
}

fn color_correct(
    view: ViewQuery<(&ViewTarget, &ColorCorrectionPass)>,
    pipeline: Option<Res<ColorCorrectionPipeline>>,
    image: Option<Res<ColorCorrectionImage>>,
    images: Res<RenderAssets<GpuImage>>,
    pipeline_cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (Some(pipeline), Some(image)) = (pipeline, image) else {
        return;
    };
    let Some(lut) = images.get(&image.0) else { return };
    let (target, _) = view.into_inner();
    let id = if target.main_texture_format() == TextureFormat::Rgba16Float {
        pipeline.hdr
    } else if target.main_texture_format() == TextureFormat::Rgba8UnormSrgb {
        pipeline.ldr
    } else {
        return;
    };
    let Some(render_pipeline) = pipeline_cache.get_render_pipeline(id) else {
        return;
    };
    let post = target.post_process_write();
    let bind_group = ctx.render_device().create_bind_group(
        "color_correction_bind_group",
        &pipeline_cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((post.source, &pipeline.sampler, &lut.texture_view, &lut.sampler)),
    );
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("color_correction_pass"),
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
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..3, 0..1);
}
