// Sprites (specs/cs_source/sprites_dust.md): a quad parallel to the view
// plane (view right/up, so camera roll turns it too), added to the image:
// texture x color, weighted by texture alpha x color alpha.

#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput},
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

struct SpriteParams {
    color: vec4<f32>,
    size: vec2<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: SpriteParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var sprite_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var sprite_sampler: sampler;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let centre = (world_from_local * vec4<f32>(vertex.position, 1.0)).xyz;
    let right = normalize(view.world_from_view[0].xyz);
    let up = normalize(view.world_from_view[1].xyz);
    let corner = vertex.uv_b;
    let world = centre + right * corner.x * params.size.x + up * corner.y * params.size.y;
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    out.world_normal = -normalize(view.world_from_view[2].xyz);
    out.uv = vertex.uv;
    out.uv_b = corner;
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = 0;
#endif
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = textureSample(sprite_texture, sprite_sampler, in.uv);
    let rgb = t.rgb * params.color.rgb * t.a * params.color.a;
    return vec4<f32>(rgb, 0.0);
}
