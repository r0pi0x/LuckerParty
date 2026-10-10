// Source's fog at a world point, shared by the map's shaders
// (specs/cs_source/shaders.md section 7, water.md sections 8 and 9).
//
// `color`/`range`: the world's range fog (linear colour, w = 1 when on;
// start, end in meters, max density). `water_color`/`water_range`: the
// water's fog for points below `water_range.w` (the surface plus the
// 2-unit fudge): `water_color.w` 1 = range fog (the eye is under water),
// 2 = the water's height fog (the eye is just above the surface and the
// near plane crosses it: Source's intersection view).
//
// Returns the fog colour (rgb) and how much of it to apply (a): range fog
// is f^2, height fog is f (not squared).

#define_import_path mashup::fog

#import bevy_pbr::{
    mesh_view_bindings::view,
    view_transformations::position_world_to_view,
}
#ifdef DISTANCE_FOG
#import bevy_pbr::mesh_view_bindings::fog
#endif

// A view with Source fog of its own (a monitor's camera: map::monitor,
// `SOURCE_VIEW_FOG`) carries it as Bevy's linear distance fog, marked by
// the directional light exponent: colour (alpha 1 when on), start and end
// meters, max density in the directional light colour's red.
const SOURCE_VIEW_FOG: f32 = -7.0;

fn view_fog() -> bool {
#ifdef DISTANCE_FOG
    return fog.mode == 1u && fog.directional_light_exponent == SOURCE_VIEW_FOG;
#else
    return false;
#endif
}

fn range_amount(range: vec4<f32>, depth: f32) -> f32 {
    let f = clamp(min(range.z, (depth - range.x) / (range.y - range.x)), 0.0, 1.0);
    return f * f;
}

// Height fog (water.md section 8): the share of the eye's path to the
// point that is under the plane, times the point's depth, times one over
// the fog range.
fn height_amount(range: vec4<f32>, world: vec3<f32>, depth: f32) -> f32 {
    let plane = range.w;
    let path = view.world_position.y - world.y;
    let h = select(1.0, clamp((plane - world.y) / path, 0.0, 1.0), path > 0.0);
    let k = 1.0 / max(range.y - range.x, 1e-4);
    return clamp(h * depth * k, 0.0, 1.0);
}

fn source_fog(
    color: vec4<f32>,
    range: vec4<f32>,
    water_color: vec4<f32>,
    water_range: vec4<f32>,
    world: vec3<f32>,
) -> vec4<f32> {
    let depth = -position_world_to_view(world).z;
    // The view's own fog replaces the scene's, water fog included (that
    // follows the player's eye).
#ifdef DISTANCE_FOG
    if view_fog() {
        if fog.base_color.a < 0.5 {
            return vec4<f32>(0.0);
        }
        let r = vec4<f32>(fog.be.x, fog.be.y, fog.directional_light_color.r, 0.0);
        return vec4<f32>(fog.base_color.rgb, range_amount(r, depth));
    }
#endif
    if water_color.w > 0.5 && world.y < water_range.w {
        if water_color.w > 1.5 {
            return vec4<f32>(water_color.rgb, height_amount(water_range, world, depth));
        }
        return vec4<f32>(water_color.rgb, range_amount(water_range, depth));
    }
    if color.w < 0.5 {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(color.rgb, range_amount(range, depth));
}
