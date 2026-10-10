#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings::{view, lights},
}

struct Flesh {
    scatter: vec4<f32>,
    wrap: vec4<f32>,
    sheen: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> flesh: Flesh;

const PI: f32 = 3.14159265;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    var nail = 0.0;
#ifdef VERTEX_COLORS
    nail = 1.0 - in.color.a;
#endif
    pbr_input.material.base_color.a = 1.0;
    pbr_input.material.perceptual_roughness = mix(pbr_input.material.perceptual_roughness, flesh.sheen.w, nail);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);

    let albedo = pbr_input.material.base_color.rgb;
    let n = pbr_input.N;
    let facing = clamp(dot(n, pbr_input.V), 0.0, 1.0);
    let skin = 1.0 - nail;

    var scattered = vec3(0.0);
    var lit = vec3(0.0);
    for (var i = 0u; i < lights.n_directional_lights; i = i + 1u) {
        let light = &lights.directional_lights[i];
        let ndl = dot(n, (*light).direction_to_light);
        let wrap = flesh.wrap.rgb;
        let wrapped = clamp((vec3(ndl) + wrap) / (1.0 + wrap), vec3(0.0), vec3(1.0));
        scattered += max(wrapped * wrapped - vec3(max(ndl, 0.0)), vec3(0.0)) * (*light).color.rgb;
        lit += (*light).color.rgb * (0.5 + 0.5 * ndl);
    }
    let blood = albedo * flesh.scatter.rgb;
    var extra = scattered * blood / PI * flesh.scatter.w * skin;

    let rim = pow(1.0 - facing, 3.0);
    extra += rim * lit * flesh.sheen.rgb * mix(albedo, vec3(1.0), 0.35) / PI;

    let colour = out.color.rgb + extra * view.exposure;
    let level = max(dot(colour, vec3(0.2126, 0.7152, 0.0722)), 1e-5);
    let over = max(level - flesh.wrap.w, 0.0);
    let rolled = level - over + over / (1.0 + over / (1.0 - flesh.wrap.w));
    out.color = vec4(colour * (rolled / level), out.color.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
