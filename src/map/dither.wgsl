// Screen-door fading for meshes with a Bevy visibility range (prop fade
// bands, map::vis::fade_band): drop a share of the pixels in a 4x4 ordered
// pattern by the mesh's dither level, as Bevy's own materials do. Levels
// (from the vertex stage's `visibility_range_dither`): 0 drawn, 1..15
// fading out (that many sixteenths dropped), -15..-1 fading in, +-16 gone.

#define_import_path mashup::dither

fn dithered_out(frag_coord: vec4<f32>, level: i32) -> bool {
    if level == 0 {
        return false;
    }
    if level <= -16 || level >= 16 {
        return true;
    }
    // A 4x4 Bayer matrix, one row per word, one threshold per byte.
    var rows = array<u32, 4>(0x0a020800u, 0x060e040cu, 0x09010b03u, 0x050d070fu);
    let c = vec2<u32>(floor(frag_coord.xy)) % 4u;
    let threshold = i32((rows[c.y] >> (c.x * 8u)) & 0xffu);
    return (level >= 0 && level + threshold >= 16) || (level < 0 && 1 + level + threshold <= 0);
}
