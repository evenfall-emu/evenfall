// Draws one source frame into the viewport with a scaling preset.
//
// The source is the system's raw frame as an integer texture, so decoding and
// every filter tap happen here in shader arithmetic. `reference.rs` defines
// the same presets in f64 for the tests.

struct Params {
    // The viewport's top-left corner and size in target pixels.
    origin: vec2<u32>,
    size: vec2<u32>,
    // 0: Pixel (nearest neighbour). 1: VanillaPlus (sharp bilinear).
    preset: u32,
}

@group(0) @binding(0) var source: texture_2d<u32>;
@group(0) @binding(1) var<uniform> params: Params;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the whole viewport.
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

// BGR555: red in bits 0-4, green 5-9, blue 10-14. Bit 15 is ignored.
fn decode(pixel: u32) -> vec3<f32> {
    let five = vec3<u32>(pixel, pixel >> 5u, pixel >> 10u) & vec3<u32>(31u);
    return vec3<f32>(five) / 31.0;
}

fn texel(at: vec2<i32>) -> vec3<f32> {
    let last = vec2<i32>(textureDimensions(source)) - 1;
    return decode(textureLoad(source, clamp(at, vec2<i32>(0), last), 0).r);
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = textureDimensions(source);
    let out = params.size;
    let pixel = vec2<u32>(position.xy) - params.origin;
    if params.preset == 0u {
        // The source pixel under the output pixel's centre, in exact integers:
        // floor((pixel + 0.5) * size / out).
        let at = ((2u * pixel + 1u) * size) / (2u * out);
        return vec4<f32>(texel(vec2<i32>(at)), 1.0);
    }
    // Sharp bilinear: enlarge by the whole multiple k with nearest neighbour,
    // then interpolate bilinearly between that image's texel centres.
    let scale = vec2<f32>(out) / vec2<f32>(size);
    let k = max(floor(scale), vec2<f32>(1.0));
    let f = (vec2<f32>(pixel) + 0.5) / scale * k - 0.5;
    let base = floor(f);
    let weight = f - base;
    let ki = vec2<i32>(k);
    let last = vec2<i32>(size) * ki - 1;
    let s0 = clamp(vec2<i32>(base), vec2<i32>(0), last) / ki;
    let s1 = clamp(vec2<i32>(base) + 1, vec2<i32>(0), last) / ki;
    let top = mix(texel(vec2<i32>(s0.x, s0.y)), texel(vec2<i32>(s1.x, s0.y)), weight.x);
    let bottom = mix(texel(vec2<i32>(s0.x, s1.y)), texel(vec2<i32>(s1.x, s1.y)), weight.x);
    return vec4<f32>(mix(top, bottom, weight.y), 1.0);
}
