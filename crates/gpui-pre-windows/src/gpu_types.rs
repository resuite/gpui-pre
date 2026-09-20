//! CPU-side GPU records for the Direct3D 11 renderer.
//!
//! The ordinary HLSL structs keep their upstream sizes. Transformed primitives
//! reuse those same strides by replacing the CPU-only `order` field with the
//! one-based `spatial_id` (`0` is identity and needs no GPU lookup).

use gpui::{
    AtlasTile, Background, BorderStyle, Bounds, ContentMask, Corners, Edges, Hsla, PaddedBool32,
    ScaledPixels, SpatialId,
};

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GpuQuad {
    pub spatial_id: u32,
    pub border_style: BorderStyle,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub background: Background,
    pub border_color: Hsla,
    pub corner_radii: Corners<ScaledPixels>,
    pub border_widths: Edges<ScaledPixels>,
}

impl From<&gpui::Quad> for GpuQuad {
    fn from(value: &gpui::Quad) -> Self {
        Self {
            spatial_id: value.spatial_id.0,
            border_style: value.border_style,
            bounds: value.bounds,
            content_mask: value.content_mask,
            background: value.background,
            border_color: value.border_color,
            corner_radii: value.corner_radii,
            border_widths: value.border_widths,
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GpuShadow {
    pub spatial_id: u32,
    pub blur_radius: ScaledPixels,
    pub bounds: Bounds<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub element_bounds: Bounds<ScaledPixels>,
    pub element_corner_radii: Corners<ScaledPixels>,
    pub inset: u32,
    pub pad: u32,
}

impl From<&gpui::Shadow> for GpuShadow {
    fn from(value: &gpui::Shadow) -> Self {
        Self {
            spatial_id: value.spatial_id.0,
            blur_radius: value.blur_radius,
            bounds: value.bounds,
            corner_radii: value.corner_radii,
            content_mask: value.content_mask,
            color: value.color,
            element_bounds: value.element_bounds,
            element_corner_radii: value.element_corner_radii,
            inset: value.inset,
            pad: value.pad,
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GpuUnderline {
    pub spatial_id: u32,
    pub pad: u32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub thickness: ScaledPixels,
    pub wavy: PaddedBool32,
}

impl From<&gpui::Underline> for GpuUnderline {
    fn from(value: &gpui::Underline) -> Self {
        Self {
            spatial_id: value.spatial_id.0,
            pad: value.pad,
            bounds: value.bounds,
            content_mask: value.content_mask,
            color: value.color,
            thickness: value.thickness,
            wavy: value.wavy,
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GpuMonochromeSprite {
    pub spatial_id: u32,
    pub pad: u32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: gpui::TransformationMatrix,
}

impl From<&gpui::MonochromeSprite> for GpuMonochromeSprite {
    fn from(value: &gpui::MonochromeSprite) -> Self {
        Self {
            spatial_id: value.spatial_id.0,
            pad: value.pad,
            bounds: value.bounds,
            content_mask: value.content_mask,
            color: value.color,
            tile: value.tile,
            transformation: value.transformation,
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GpuPolychromeSprite {
    pub spatial_id: u32,
    pub pad: u32,
    pub grayscale: PaddedBool32,
    pub opacity: f32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub tile: AtlasTile,
}

impl From<&gpui::PolychromeSprite> for GpuPolychromeSprite {
    fn from(value: &gpui::PolychromeSprite) -> Self {
        Self {
            spatial_id: value.spatial_id.0,
            pad: value.pad,
            grayscale: value.grayscale,
            opacity: value.opacity,
            bounds: value.bounds,
            content_mask: value.content_mask,
            corner_radii: value.corner_radii,
            tile: value.tile,
        }
    }
}

/// Packed spatial state uploaded to `t2`. Layout matches HLSL `SpatialState`:
/// row-major matrix coefficients, translation, absolute clip start, clip count.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GpuSpatialState {
    pub rotation_scale: [[f32; 2]; 2],
    pub translation: [f32; 2],
    pub clip_start: u32,
    pub clip_count: u32,
}

impl From<&gpui::SpatialState> for GpuSpatialState {
    fn from(value: &gpui::SpatialState) -> Self {
        Self {
            rotation_scale: value.matrix.rotation_scale,
            translation: value.matrix.translation,
            clip_start: value.clip_start,
            clip_count: value.clip_count,
        }
    }
}

/// Packed ancestor clip uploaded to `t3`. Layout matches HLSL
/// `TransformedClip`: row-major inverse matrix, translation, bounds.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct GpuTransformedClip {
    pub rotation_scale: [[f32; 2]; 2],
    pub translation: [f32; 2],
    pub bounds_origin: [f32; 2],
    pub bounds_size: [f32; 2],
}

impl From<&gpui::TransformedClip> for GpuTransformedClip {
    fn from(value: &gpui::TransformedClip) -> Self {
        Self {
            rotation_scale: value.inverse.rotation_scale,
            translation: value.inverse.translation,
            bounds_origin: [value.bounds.origin.x.0, value.bounds.origin.y.0],
            bounds_size: [value.bounds.size.width.0, value.bounds.size.height.0],
        }
    }
}

// Shader ABI: ordinary records keep upstream strides; spatial records are
// fixed 32/40 bytes to match HLSL StructuredBuffer packing.
const _: [(); 160] = [(); std::mem::size_of::<GpuQuad>()];
const _: [(); 112] = [(); std::mem::size_of::<GpuShadow>()];
const _: [(); 64] = [(); std::mem::size_of::<GpuUnderline>()];
const _: [(); 112] = [(); std::mem::size_of::<GpuMonochromeSprite>()];
const _: [(); 96] = [(); std::mem::size_of::<GpuPolychromeSprite>()];
const _: [(); 32] = [(); std::mem::size_of::<GpuSpatialState>()];
const _: [(); 40] = [(); std::mem::size_of::<GpuTransformedClip>()];

/// One-based spatial reference. Identity stays zero so ordinary batches skip
/// the spatial lookup entirely.
pub fn gpu_spatial_id(id: SpatialId) -> u32 {
    id.0
}
