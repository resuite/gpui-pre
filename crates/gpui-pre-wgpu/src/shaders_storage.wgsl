// Storage-buffer instance transport. Concatenated after `shaders.wgsl` on backends
// with storage buffer support; `shaders_webgl.wgsl` is the WebGL2 counterpart that
// decodes the same records from a texture instead.
//
// All buffers share `@group(1) @binding(0)` because each pipeline binds only the
// buffer its own entry points read.

@group(1) @binding(0) var<storage, read> b_quads: array<Quad>;
@group(1) @binding(0) var<storage, read> b_shadows: array<Shadow>;
@group(1) @binding(0) var<storage, read> b_path_vertices: array<PathRasterizationVertex>;
@group(1) @binding(0) var<storage, read> b_transformed_path_vertices: array<TransformedPathRasterizationVertex>;
@group(1) @binding(0) var<storage, read> b_path_sprites: array<PathSprite>;
@group(1) @binding(0) var<storage, read> b_underlines: array<Underline>;
@group(1) @binding(0) var<storage, read> b_mono_sprites: array<MonochromeSprite>;
@group(1) @binding(0) var<storage, read> b_poly_sprites: array<PolychromeSprite>;

fn load_quad(instance_id: u32) -> Quad {
    return b_quads[instance_id];
}

fn load_shadow(instance_id: u32) -> Shadow {
    return b_shadows[instance_id];
}

fn load_path_vertex(vertex_id: u32) -> PathRasterizationVertex {
    return b_path_vertices[vertex_id];
}
fn load_transformed_path_vertex(vertex_id: u32) -> TransformedPathRasterizationVertex {
    return b_transformed_path_vertices[vertex_id];
}

fn load_path_sprite(instance_id: u32) -> PathSprite {
    return b_path_sprites[instance_id];
}

fn load_underline(instance_id: u32) -> Underline {
    return b_underlines[instance_id];
}

fn load_mono_sprite(instance_id: u32) -> MonochromeSprite {
    return b_mono_sprites[instance_id];
}

fn load_poly_sprite(instance_id: u32) -> PolychromeSprite {
    return b_poly_sprites[instance_id];
}

// Sparse transform data is packed as raw vec4<u32> blocks: state records first, clips after.
@group(3) @binding(0) var<storage, read> b_spatial_data: array<vec4<u32>>;
fn spatial_word(index: u32) -> u32 { return b_spatial_data[index / 4u][index % 4u]; }
fn spatial_f32(index: u32) -> f32 { return bitcast<f32>(spatial_word(index)); }
fn load_spatial_state(start: u32) -> SpatialState {
    return SpatialState(
        TransformationMatrix(
            mat2x2<f32>(vec2<f32>(spatial_f32(start), spatial_f32(start+1u)), vec2<f32>(spatial_f32(start+2u), spatial_f32(start+3u))),
            vec2<f32>(spatial_f32(start+4u), spatial_f32(start+5u))
        ),
        spatial_word(start+6u), spatial_word(start+7u)
    );
}
fn load_transform_clip(start: u32) -> TransformedClip {
    return TransformedClip(
        TransformationMatrix(
            mat2x2<f32>(vec2<f32>(spatial_f32(start), spatial_f32(start+1u)), vec2<f32>(spatial_f32(start+2u), spatial_f32(start+3u))),
            vec2<f32>(spatial_f32(start+4u), spatial_f32(start+5u))
        ),
        Bounds(vec2<f32>(spatial_f32(start+6u), spatial_f32(start+7u)), vec2<f32>(spatial_f32(start+8u), spatial_f32(start+9u)))
    );
}
