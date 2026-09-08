#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_functions,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}

struct SurfaceUniform { region: vec4<f32>, mapping: vec4<f32>, options: vec4<f32> }
@group(2) @binding(100) var<uniform> surface: SurfaceUniform;
@group(2) @binding(101) var icon: texture_2d<f32>;
@group(2) @binding(102) var icon_sampler: sampler;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> FragmentOutput {
    var pbr = pbr_input_from_standard_material(in, front);
#ifdef VERTEX_UVS_A
    if surface.options.x > 0.5 {
        var uv = (in.uv - vec2<f32>(0.5)) / (1.0 - 2.0 * surface.mapping.z) + vec2<f32>(0.5);
        let world = mesh_functions::get_world_from_local(in.instance_index);
        let aspect = max(length(world[0].xyz), 0.00001) / max(length(world[1].xyz), 0.00001);
        let ratio = surface.mapping.y / aspect;
        if surface.mapping.x > 0.5 && surface.mapping.x < 1.5 {
            uv = (uv - vec2<f32>(0.5)) * vec2<f32>(max(1.0, 1.0 / ratio), max(1.0, ratio)) + vec2<f32>(0.5);
        } else if surface.mapping.x > 1.5 {
            uv = (uv - vec2<f32>(0.5)) * vec2<f32>(min(1.0, 1.0 / ratio), min(1.0, ratio)) + vec2<f32>(0.5);
        }
        let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
        let front_normal = mesh_functions::mesh_normal_local_to_world(vec3<f32>(0.0,0.0,1.0), in.instance_index);
        let use_face = surface.mapping.w < 0.5 || dot(normalize(in.world_normal), normalize(front_normal)) > 0.98;
        // Half-texel inset protects atlas cells with linear sampling.
        let inset = vec2<f32>(0.5) / vec2<f32>(textureDimensions(icon));
        let atlas_uv = clamp(surface.region.xy + uv * surface.region.zw, surface.region.xy + inset, surface.region.xy + surface.region.zw - inset);
        let texel = textureSample(icon, icon_sampler, atlas_uv);
        if surface.options.y > 0.5 {
            if inside && use_face { pbr.material.base_color = vec4<f32>(mix(pbr.material.base_color.rgb, texel.rgb, texel.a), pbr.material.base_color.a); }
        } else {
            if inside && use_face { pbr.material.base_color *= texel; }
            else { pbr.material.base_color = vec4<f32>(0.0); }
        }
    }
#endif
    pbr.material.base_color = alpha_discard(pbr.material, pbr.material.base_color);
    var out: FragmentOutput;
    if (pbr.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u { out.color = apply_pbr_lighting(pbr); }
    else { out.color = pbr.material.base_color; }
    out.color = main_pass_post_lighting_processing(pbr, out.color);
    return out;
}
