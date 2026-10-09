// Sky faces drawn into depth only (sky_occluder.rs): the pipeline writes
// no colour, so the sky the sky camera drew stays.

@fragment
fn fragment() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}
