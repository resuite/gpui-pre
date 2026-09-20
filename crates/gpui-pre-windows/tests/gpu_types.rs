#[path = "../src/gpu_types.rs"]
mod gpu_types;

use gpui::{
    AtlasTextureId, AtlasTextureKind, Bounds, ContentMask, Corners, Edges, Hsla, PaddedBool32,
    SpatialId, TileId, point, size,
};

fn test_bounds() -> Bounds<gpui::ScaledPixels> {
    Bounds::new(
        point(gpui::ScaledPixels(10.0), gpui::ScaledPixels(20.0)),
        size(gpui::ScaledPixels(100.0), gpui::ScaledPixels(50.0)),
    )
}

#[test]
fn gpu_records_preserve_upstream_strides() {
    assert_eq!(std::mem::size_of::<gpu_types::GpuQuad>(), 160);
    assert_eq!(std::mem::size_of::<gpu_types::GpuShadow>(), 112);
    assert_eq!(std::mem::size_of::<gpu_types::GpuUnderline>(), 64);
    assert_eq!(std::mem::size_of::<gpu_types::GpuMonochromeSprite>(), 112);
    assert_eq!(std::mem::size_of::<gpu_types::GpuPolychromeSprite>(), 96);
    assert_eq!(std::mem::size_of::<gpu_types::GpuSpatialState>(), 32);
    assert_eq!(std::mem::size_of::<gpu_types::GpuTransformedClip>(), 40);
}

#[test]
fn spatial_id_replaces_order_without_changing_offsets() {
    let quad = gpui::Quad {
        order: 123,
        border_style: gpui::BorderStyle::Solid,
        bounds: test_bounds(),
        content_mask: ContentMask {
            bounds: test_bounds(),
        },
        background: gpui::Background::default(),
        border_color: Hsla::default(),
        corner_radii: Corners::default(),
        border_widths: Edges::default(),
        spatial_id: SpatialId(7),
    };
    let gpu = gpu_types::GpuQuad::from(&quad);
    assert_eq!(gpu.spatial_id, 7);
    assert_eq!(gpu.bounds, quad.bounds);
    // Identity keeps zero so ordinary batches skip the spatial lookup.
    let identity = gpui::Quad {
        spatial_id: SpatialId::IDENTITY,
        ..quad
    };
    assert_eq!(gpu_types::GpuQuad::from(&identity).spatial_id, 0);
}

#[test]
fn spatial_state_packs_matrix_and_clip_range() {
    let state = gpui::SpatialState {
        matrix: gpui::TransformationMatrix {
            rotation_scale: [[2.0, 0.5], [-0.5, 3.0]],
            translation: [11.0, -4.0],
        },
        clip_start: 3,
        clip_count: 2,
    };
    let gpu = gpu_types::GpuSpatialState::from(&state);
    assert_eq!(gpu.rotation_scale, [[2.0, 0.5], [-0.5, 3.0]]);
    assert_eq!(gpu.translation, [11.0, -4.0]);
    assert_eq!(gpu.clip_start, 3);
    assert_eq!(gpu.clip_count, 2);
}

#[test]
fn transformed_clip_packs_inverse_and_bounds() {
    let clip = gpui::TransformedClip {
        inverse: gpui::TransformationMatrix {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [5.0, 6.0],
        },
        bounds: test_bounds(),
    };
    let gpu = gpu_types::GpuTransformedClip::from(&clip);
    assert_eq!(gpu.translation, [5.0, 6.0]);
    assert_eq!(gpu.bounds_origin, [10.0, 20.0]);
    assert_eq!(gpu.bounds_size, [100.0, 50.0]);
}

#[test]
fn underline_and_sprite_ids_round_trip() {
    let underline = gpui::Underline {
        order: 0,
        pad: 0,
        bounds: test_bounds(),
        content_mask: ContentMask {
            bounds: test_bounds(),
        },
        color: Hsla::default(),
        thickness: gpui::ScaledPixels(2.0),
        wavy: PaddedBool32::default(),
        spatial_id: SpatialId(2),
    };
    assert_eq!(gpu_types::GpuUnderline::from(&underline).spatial_id, 2);

    let sprite = gpui::PolychromeSprite {
        order: 0,
        pad: 0,
        grayscale: PaddedBool32::default(),
        opacity: 1.0,
        bounds: test_bounds(),
        content_mask: ContentMask {
            bounds: test_bounds(),
        },
        corner_radii: Corners::default(),
        tile: gpui::AtlasTile {
            texture_id: AtlasTextureId {
                index: 0,
                kind: AtlasTextureKind::Polychrome,
            },
            tile_id: TileId(0),
            padding: 0,
            bounds: Bounds::new(
                point(gpui::DevicePixels(0), gpui::DevicePixels(0)),
                size(gpui::DevicePixels(8), gpui::DevicePixels(8)),
            ),
        },
        spatial_id: SpatialId(4),
    };
    let gpu = gpu_types::GpuPolychromeSprite::from(&sprite);
    assert_eq!(gpu.spatial_id, 4);
    assert_eq!(gpu.opacity, 1.0);
}
