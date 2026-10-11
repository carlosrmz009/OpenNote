#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
}

struct Lacquer {
    shine: vec4<f32>,
    eye: vec4<f32>,
    strip: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> lacquer: Lacquer;

fn studio(r: vec3<f32>) -> f32 {
    let above = clamp(r.z, 0.0, 1.0);
    let room = 0.04 + 0.10 * above * above;
    let strip = exp(-pow((r.y - lacquer.strip.x) / lacquer.strip.y, 2.0)) * smoothstep(0.05, 0.35, r.z);
    let fill = exp(-pow((r.y - lacquer.strip.z) / lacquer.strip.w, 2.0)) * smoothstep(0.2, 0.6, r.z);
    return room + strip + 0.35 * fill;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);

    let n = pbr_input.N;
    let v = normalize(lacquer.eye.xyz - in.world_position.xyz);
    let r = reflect(-v, n);
    let facing = clamp(dot(n, v), 0.0, 1.0);
    let f0 = lacquer.shine.y;
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - facing, 5.0);
    let reflected = lacquer.shine.x * fresnel * studio(r) * lacquer.shine.z;

    out.color = vec4(out.color.rgb + vec3(reflected), out.color.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
