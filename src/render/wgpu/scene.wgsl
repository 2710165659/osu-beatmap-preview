struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

@vertex
fn scene_vertex(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(input.position, 0.0, 1.0);
    output.uv = input.uv;
    output.color = input.color;
    return output;
}

fn premultiply(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(color.rgb * color.a, color.a);
}

@fragment
fn solid_fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    return premultiply(input.color);
}

@fragment
fn sprite_fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    return premultiply(textureSample(source_texture, source_sampler, input.uv) * input.color);
}

@fragment
fn glyph_fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    let alpha = textureSample(source_texture, source_sampler, input.uv).a * input.color.a;
    return vec4<f32>(input.color.rgb * alpha, alpha);
}

@vertex
fn resolve_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[index], 0.0, 1.0);
}

@fragment
fn resolve_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(source_texture));
    let uv = position.xy / size;
    let color = textureSample(source_texture, source_sampler, uv);
    if color.a <= 0.00001 {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(color.rgb / color.a, color.a);
}
