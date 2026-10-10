#import bevy_pbr::forward_io::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> tint: vec4<f32>;

fn rounded_box(p: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half + vec2(radius);
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.uv;
    let half = in.uv_b;
    let radius = min(half.x * 0.5, half.y);
    let d = rounded_box(p, half, radius);
    let aa = max(fwidth(d), 1e-4);
    let coverage = clamp(0.5 - d / aa, 0.0, 1.0);
    if coverage <= 0.0 {
        discard;
    }

    let inside = max(-d, 0.0);
    let across = p.x / half.x;
    let from_bottom = p.y + half.y;

    let rim = exp(-inside / 0.95);
    let bevel = exp(-pow((inside - 1.25) / 0.55, 2.0));
    let lean = normalize(p / max(half - vec2(radius), vec2(1e-3)) + vec2(1e-4));
    let lit_side = 0.55 + 0.45 * dot(lean, normalize(vec2(-0.55, 0.85)));

    let tube = 1.0 - across * across;
    let core = pow(tube, 2.5);
    let lead = exp(-from_bottom / 9.0);

    let slant = in.world_position.y + p.x * 0.85;
    let facet = 0.5 + 0.5 * sin(slant * 0.105 + 0.6 * sin(slant * 0.031));
    let glint = pow(0.5 + 0.5 * sin(in.world_position.y * 0.021 + p.x * 0.05), 18.0);

    let colour = tint.rgb;
    let glow = tint.w;
    let pale = mix(colour, vec3(1.0), 0.3);

    var light = colour * (0.050 + 0.022 * facet + 0.030 * core + 0.10 * lead);
    light += colour * rim * 0.85;
    light += pale * bevel * lit_side * 0.045;
    light += pale * glint * tube * 0.035;

    return vec4(light * glow, coverage);
}
