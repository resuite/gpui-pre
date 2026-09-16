// todo("windows"): remove
#![cfg_attr(windows, allow(dead_code))]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AtlasTextureId, AtlasTile, Background, Bounds, ContentMask, Corners, Edges, Hsla, Pixels,
    Point, Radians, ScaledPixels, Size, bounds_tree::BoundsTree, point,
};
use std::{
    fmt::Debug,
    iter::Peekable,
    ops::{Add, Range, Sub},
    slice,
};

#[allow(non_camel_case_types, unused)]
#[expect(missing_docs)]
pub type PathVertex_ScaledPixels = PathVertex<ScaledPixels>;

#[expect(missing_docs)]
pub type DrawOrder = u32;

/// A boolean stored as a `u32` so that GPU-facing structs contain no
/// compiler-inserted padding bytes, which would be undefined behavior to
/// reinterpret as `&[u8]` when writing instance buffers. Guaranteed to be
/// `0` or `1` by construction; shaders read it as a `u32`/`uint`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct PaddedBool32(u32);

impl From<bool> for PaddedBool32 {
    fn from(value: bool) -> Self {
        PaddedBool32(value as u32)
    }
}

#[derive(Default)]
#[expect(missing_docs)]
pub struct Scene {
    pub(crate) paint_operations: Vec<PaintOperation>,
    /// Spatial state applied to newly inserted primitives. Zero is the identity fast path.
    pub(crate) spatial_id: SpatialId,
    /// Sparse affine state. IDs are one-based so `SpatialId(0)` needs no GPU lookup.
    pub spatial_states: Vec<SpatialState>,
    /// Ancestor clip rectangles referenced by transformed spatial states.
    pub transform_clips: Vec<TransformedClip>,
    primitive_bounds: BoundsTree<ScaledPixels>,
    layer_stack: Vec<DrawOrder>,
    pub shadows: Vec<Shadow>,
    pub quads: Vec<Quad>,
    pub paths: Vec<Path<ScaledPixels>>,
    pub underlines: Vec<Underline>,
    pub monochrome_sprites: Vec<MonochromeSprite>,
    pub subpixel_sprites: Vec<SubpixelSprite>,
    pub polychrome_sprites: Vec<PolychromeSprite>,
    pub surfaces: Vec<PaintSurface>,
}

#[expect(missing_docs)]
impl Scene {
    pub fn clear(&mut self) {
        self.paint_operations.clear();
        self.spatial_id = SpatialId::IDENTITY;
        self.spatial_states.clear();
        self.transform_clips.clear();
        self.primitive_bounds.clear();
        self.layer_stack.clear();
        self.paths.clear();
        self.shadows.clear();
        self.quads.clear();
        self.underlines.clear();
        self.monochrome_sprites.clear();
        self.subpixel_sprites.clear();
        self.polychrome_sprites.clear();
        self.surfaces.clear();
    }

    pub fn len(&self) -> usize {
        self.paint_operations.len()
    }

    pub fn push_layer(&mut self, bounds: Bounds<ScaledPixels>) {
        let order = self.primitive_bounds.insert(bounds);
        self.layer_stack.push(order);
        self.paint_operations
            .push(PaintOperation::StartLayer(bounds));
    }

    pub fn pop_layer(&mut self) {
        self.layer_stack.pop();
        self.paint_operations.push(PaintOperation::EndLayer);
    }

    pub fn insert_primitive(&mut self, primitive: impl Into<Primitive>) {
        let mut primitive = primitive.into();
        *primitive.spatial_id_mut() = self.spatial_id;
        let matrix = self.spatial_matrix(self.spatial_id);
        self.insert_spatial_primitive(primitive, matrix);
    }

    fn insert_spatial_primitive(&mut self, primitive: Primitive, matrix: TransformationMatrix) {
        let clipped_bounds = primitive.clipped_bounds(matrix);

        if clipped_bounds.is_empty() {
            return;
        }

        let order = self
            .layer_stack
            .last()
            .copied()
            .unwrap_or_else(|| self.primitive_bounds.insert(clipped_bounds));
        self.push_primitive(primitive, order);
    }

    fn push_primitive(&mut self, mut primitive: Primitive, order: DrawOrder) {
        match &mut primitive {
            Primitive::Shadow(shadow) => {
                shadow.order = order;
                self.shadows.push(*shadow);
            }
            Primitive::Quad(quad) => {
                quad.order = order;
                self.quads.push(*quad);
            }
            Primitive::Path(path) => {
                path.order = order;
                path.id = PathId(self.paths.len());
                self.paths.push(path.clone());
            }
            Primitive::Underline(underline) => {
                underline.order = order;
                self.underlines.push(*underline);
            }
            Primitive::MonochromeSprite(sprite) => {
                sprite.order = order;
                self.monochrome_sprites.push(*sprite);
            }
            Primitive::SubpixelSprite(sprite) => {
                sprite.order = order;
                self.subpixel_sprites.push(*sprite);
            }
            Primitive::PolychromeSprite(sprite) => {
                sprite.order = order;
                self.polychrome_sprites.push(*sprite);
            }
            Primitive::Surface(surface) => {
                surface.order = order;
                self.surfaces.push(surface.clone());
            }
        }
        self.paint_operations
            .push(PaintOperation::Primitive(primitive));
    }

    pub fn replay(&mut self, range: Range<usize>, prev_scene: &Scene) {
        let mut spatial_map = vec![None; prev_scene.spatial_states.len() + 1];
        spatial_map[0] = Some(SpatialId::IDENTITY);

        // An enclosing layer assigns one order to every primitive inside it,
        // so fall back to per-operation replay to keep the cached primitives at
        // the current layer's order.
        if self.layer_stack.last().is_some() {
            for operation in &prev_scene.paint_operations[range] {
                match operation {
                    PaintOperation::Primitive(primitive) => {
                        let primitive =
                            self.replay_primitive(primitive, prev_scene, &mut spatial_map);
                        let matrix = self.spatial_matrix(primitive.spatial_id());
                        self.insert_spatial_primitive(primitive, matrix)
                    }
                    PaintOperation::StartLayer(bounds) => self.push_layer(*bounds),
                    PaintOperation::EndLayer => self.pop_layer(),
                }
            }
            return;
        }

        // Cached operations already have a stable internal paint order. Reserve
        // a single order range for the whole fragment and shift the cached
        // orders into it, instead of re-running a bounds-tree search per
        // primitive. This keeps replay linear in the number of primitives.
        let mut union: Option<Bounds<ScaledPixels>> = None;
        let mut min_order = DrawOrder::MAX;
        let mut max_order = 0;
        for operation in &prev_scene.paint_operations[range.clone()] {
            if let PaintOperation::Primitive(primitive) = operation {
                let clipped_bounds =
                    primitive.clipped_bounds(prev_scene.spatial_matrix(primitive.spatial_id()));
                union = Some(match union {
                    Some(bounds) => bounds.union(&clipped_bounds),
                    None => clipped_bounds,
                });
                min_order = min_order.min(primitive.order());
                max_order = max_order.max(primitive.order());
            }
        }

        let base = match union {
            Some(bounds) => self
                .primitive_bounds
                .insert_group(bounds, max_order - min_order),
            None => 0,
        };

        for operation in &prev_scene.paint_operations[range] {
            match operation {
                PaintOperation::Primitive(primitive) => {
                    let order = base + (primitive.order() - min_order);
                    let primitive = self.replay_primitive(primitive, prev_scene, &mut spatial_map);
                    self.push_primitive(primitive, order);
                }
                // Route layers through the normal entry points so an enclosing
                // layer stays on `layer_stack` for later live primitives, even
                // if the replayed range is not layer-balanced.
                PaintOperation::StartLayer(bounds) => self.push_layer(*bounds),
                PaintOperation::EndLayer => self.pop_layer(),
            }
        }
    }

    fn replay_primitive(
        &mut self,
        primitive: &Primitive,
        previous: &Scene,
        spatial_map: &mut [Option<SpatialId>],
    ) -> Primitive {
        let mut primitive = primitive.clone();
        let old_id = primitive.spatial_id();
        if old_id != SpatialId::IDENTITY {
            let index = old_id.index().expect("non-identity spatial id");
            let new_id = match spatial_map[index + 1] {
                Some(id) => id,
                None => {
                    let mut state = previous.spatial_states[index];
                    if state.clip_count != 0 {
                        let start = state.clip_start as usize;
                        let clips =
                            &previous.transform_clips[start..start + state.clip_count as usize];
                        state.clip_start = self.transform_clips.len() as u32;
                        self.transform_clips.extend_from_slice(clips);
                    }
                    let id = self.push_spatial_state(state);
                    spatial_map[index + 1] = Some(id);
                    id
                }
            };
            *primitive.spatial_id_mut() = new_id;
        }
        primitive
    }

    /// Register sparse paint state and return its one-based identifier.
    pub(crate) fn push_spatial_state(&mut self, state: SpatialState) -> SpatialId {
        self.spatial_states.push(state);
        SpatialId(self.spatial_states.len() as u32)
    }

    /// Resolve a compact spatial identifier to its affine state.
    #[inline]
    pub fn spatial_state(&self, id: SpatialId) -> Option<&SpatialState> {
        id.index().map(|index| &self.spatial_states[index])
    }

    /// Resolve the affine matrix for a spatial identifier. Identity needs no table entry.
    #[inline]
    pub fn spatial_matrix(&self, id: SpatialId) -> TransformationMatrix {
        self.spatial_state(id)
            .map_or_else(TransformationMatrix::unit, |state| state.matrix)
    }

    pub fn finish(&mut self) {
        self.shadows.sort_by_key(|shadow| shadow.order);
        self.quads.sort_by_key(|quad| quad.order);
        self.paths.sort_by_key(|path| path.order);
        self.underlines.sort_by_key(|underline| underline.order);
        self.monochrome_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.subpixel_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.polychrome_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.surfaces.sort_by_key(|surface| surface.order);
    }

    #[cfg_attr(
        all(
            any(target_os = "linux", target_os = "freebsd"),
            not(any(feature = "x11", feature = "wayland"))
        ),
        allow(dead_code)
    )]
    pub fn batches(&self) -> impl Iterator<Item = PrimitiveBatch> + '_ {
        BatchIterator {
            shadows_start: 0,
            shadows_iter: self.shadows.iter().peekable(),
            quads_start: 0,
            quads_iter: self.quads.iter().peekable(),
            paths_start: 0,
            paths_iter: self.paths.iter().peekable(),
            underlines_start: 0,
            underlines_iter: self.underlines.iter().peekable(),
            monochrome_sprites_start: 0,
            monochrome_sprites_iter: self.monochrome_sprites.iter().peekable(),
            subpixel_sprites_start: 0,
            subpixel_sprites_iter: self.subpixel_sprites.iter().peekable(),
            polychrome_sprites_start: 0,
            polychrome_sprites_iter: self.polychrome_sprites.iter().peekable(),
            surfaces_start: 0,
            surfaces_iter: self.surfaces.iter().peekable(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Default)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
pub(crate) enum PrimitiveKind {
    Shadow,
    #[default]
    Quad,
    Path,
    Underline,
    MonochromeSprite,
    SubpixelSprite,
    PolychromeSprite,
    Surface,
}

pub(crate) enum PaintOperation {
    Primitive(Primitive),
    StartLayer(Bounds<ScaledPixels>),
    EndLayer,
}

#[derive(Clone)]
#[expect(missing_docs)]
pub enum Primitive {
    Shadow(Shadow),
    Quad(Quad),
    Path(Path<ScaledPixels>),
    Underline(Underline),
    MonochromeSprite(MonochromeSprite),
    SubpixelSprite(SubpixelSprite),
    PolychromeSprite(PolychromeSprite),
    Surface(PaintSurface),
}

#[expect(missing_docs)]
impl Primitive {
    fn spatial_id_mut(&mut self) -> &mut SpatialId {
        match self {
            Self::Shadow(value) => &mut value.spatial_id,
            Self::Quad(value) => &mut value.spatial_id,
            Self::Path(value) => &mut value.spatial_id,
            Self::Underline(value) => &mut value.spatial_id,
            Self::MonochromeSprite(value) => &mut value.spatial_id,
            Self::SubpixelSprite(value) => &mut value.spatial_id,
            Self::PolychromeSprite(value) => &mut value.spatial_id,
            Self::Surface(value) => &mut value.spatial_id,
        }
    }

    fn spatial_id(&self) -> SpatialId {
        match self {
            Self::Shadow(value) => value.spatial_id,
            Self::Quad(value) => value.spatial_id,
            Self::Path(value) => value.spatial_id,
            Self::Underline(value) => value.spatial_id,
            Self::MonochromeSprite(value) => value.spatial_id,
            Self::SubpixelSprite(value) => value.spatial_id,
            Self::PolychromeSprite(value) => value.spatial_id,
            Self::Surface(value) => value.spatial_id,
        }
    }

    fn clipped_bounds(&self, matrix: TransformationMatrix) -> Bounds<ScaledPixels> {
        let bounds = match self {
            Self::MonochromeSprite(value) => {
                value.transformation.transform_scaled_bounds(value.bounds)
            }
            Self::SubpixelSprite(value) => {
                value.transformation.transform_scaled_bounds(value.bounds)
            }
            _ => *self.bounds(),
        };
        matrix.transform_scaled_bounds(bounds.intersect(&self.content_mask().bounds))
    }

    pub fn bounds(&self) -> &Bounds<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.bounds,
            Primitive::Quad(quad) => &quad.bounds,
            Primitive::Path(path) => &path.bounds,
            Primitive::Underline(underline) => &underline.bounds,
            Primitive::MonochromeSprite(sprite) => &sprite.bounds,
            Primitive::SubpixelSprite(sprite) => &sprite.bounds,
            Primitive::PolychromeSprite(sprite) => &sprite.bounds,
            Primitive::Surface(surface) => &surface.bounds,
        }
    }

    pub fn content_mask(&self) -> &ContentMask<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.content_mask,
            Primitive::Quad(quad) => &quad.content_mask,
            Primitive::Path(path) => &path.content_mask,
            Primitive::Underline(underline) => &underline.content_mask,
            Primitive::MonochromeSprite(sprite) => &sprite.content_mask,
            Primitive::SubpixelSprite(sprite) => &sprite.content_mask,
            Primitive::PolychromeSprite(sprite) => &sprite.content_mask,
            Primitive::Surface(surface) => &surface.content_mask,
        }
    }

    pub fn order(&self) -> DrawOrder {
        match self {
            Primitive::Shadow(shadow) => shadow.order,
            Primitive::Quad(quad) => quad.order,
            Primitive::Path(path) => path.order,
            Primitive::Underline(underline) => underline.order,
            Primitive::MonochromeSprite(sprite) => sprite.order,
            Primitive::SubpixelSprite(sprite) => sprite.order,
            Primitive::PolychromeSprite(sprite) => sprite.order,
            Primitive::Surface(surface) => surface.order,
        }
    }
}

#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
struct BatchIterator<'a> {
    shadows_start: usize,
    shadows_iter: Peekable<slice::Iter<'a, Shadow>>,
    quads_start: usize,
    quads_iter: Peekable<slice::Iter<'a, Quad>>,
    paths_start: usize,
    paths_iter: Peekable<slice::Iter<'a, Path<ScaledPixels>>>,
    underlines_start: usize,
    underlines_iter: Peekable<slice::Iter<'a, Underline>>,
    monochrome_sprites_start: usize,
    monochrome_sprites_iter: Peekable<slice::Iter<'a, MonochromeSprite>>,
    subpixel_sprites_start: usize,
    subpixel_sprites_iter: Peekable<slice::Iter<'a, SubpixelSprite>>,
    polychrome_sprites_start: usize,
    polychrome_sprites_iter: Peekable<slice::Iter<'a, PolychromeSprite>>,
    surfaces_start: usize,
    surfaces_iter: Peekable<slice::Iter<'a, PaintSurface>>,
}

impl<'a> Iterator for BatchIterator<'a> {
    type Item = PrimitiveBatch;

    fn next(&mut self) -> Option<Self::Item> {
        let mut orders_and_kinds = [
            (
                self.shadows_iter.peek().map(|s| s.order),
                PrimitiveKind::Shadow,
            ),
            (self.quads_iter.peek().map(|q| q.order), PrimitiveKind::Quad),
            (self.paths_iter.peek().map(|q| q.order), PrimitiveKind::Path),
            (
                self.underlines_iter.peek().map(|u| u.order),
                PrimitiveKind::Underline,
            ),
            (
                self.monochrome_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::MonochromeSprite,
            ),
            (
                self.subpixel_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::SubpixelSprite,
            ),
            (
                self.polychrome_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::PolychromeSprite,
            ),
            (
                self.surfaces_iter.peek().map(|s| s.order),
                PrimitiveKind::Surface,
            ),
        ];
        orders_and_kinds.sort_by_key(|(order, kind)| (order.unwrap_or(u32::MAX), *kind));

        let first = orders_and_kinds[0];
        let second = orders_and_kinds[1];
        let (batch_kind, max_order_and_kind) = if first.0.is_some() {
            (first.1, (second.0.unwrap_or(u32::MAX), second.1))
        } else {
            return None;
        };

        match batch_kind {
            PrimitiveKind::Shadow => {
                let shadows_start = self.shadows_start;
                let mut shadows_end = shadows_start + 1;
                let transformed = !self.shadows_iter.peek().unwrap().spatial_id.is_identity();
                self.shadows_iter.next();
                while self
                    .shadows_iter
                    .next_if(|shadow| {
                        (shadow.order, batch_kind) < max_order_and_kind
                            && (!shadow.spatial_id.is_identity()) == transformed
                    })
                    .is_some()
                {
                    shadows_end += 1;
                }
                self.shadows_start = shadows_end;
                Some(PrimitiveBatch::Shadows(shadows_start..shadows_end))
            }
            PrimitiveKind::Quad => {
                let quads_start = self.quads_start;
                let mut quads_end = quads_start + 1;
                let transformed = !self.quads_iter.peek().unwrap().spatial_id.is_identity();
                self.quads_iter.next();
                while self
                    .quads_iter
                    .next_if(|quad| {
                        (quad.order, batch_kind) < max_order_and_kind
                            && (!quad.spatial_id.is_identity()) == transformed
                    })
                    .is_some()
                {
                    quads_end += 1;
                }
                self.quads_start = quads_end;
                Some(PrimitiveBatch::Quads(quads_start..quads_end))
            }
            PrimitiveKind::Path => {
                let paths_start = self.paths_start;
                let mut paths_end = paths_start + 1;
                let transformed = !self.paths_iter.peek().unwrap().spatial_id.is_identity();
                self.paths_iter.next();
                while self
                    .paths_iter
                    .next_if(|path| {
                        (path.order, batch_kind) < max_order_and_kind
                            && (!path.spatial_id.is_identity()) == transformed
                    })
                    .is_some()
                {
                    paths_end += 1;
                }
                self.paths_start = paths_end;
                Some(PrimitiveBatch::Paths(paths_start..paths_end))
            }
            PrimitiveKind::Underline => {
                let underlines_start = self.underlines_start;
                let mut underlines_end = underlines_start + 1;
                let transformed = !self
                    .underlines_iter
                    .peek()
                    .unwrap()
                    .spatial_id
                    .is_identity();
                self.underlines_iter.next();
                while self
                    .underlines_iter
                    .next_if(|underline| {
                        (underline.order, batch_kind) < max_order_and_kind
                            && (!underline.spatial_id.is_identity()) == transformed
                    })
                    .is_some()
                {
                    underlines_end += 1;
                }
                self.underlines_start = underlines_end;
                Some(PrimitiveBatch::Underlines(underlines_start..underlines_end))
            }
            PrimitiveKind::MonochromeSprite => {
                let texture_id = self.monochrome_sprites_iter.peek().unwrap().tile.texture_id;
                let transformed = !self
                    .monochrome_sprites_iter
                    .peek()
                    .unwrap()
                    .spatial_id
                    .is_identity();
                let sprites_start = self.monochrome_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.monochrome_sprites_iter.next();
                while self
                    .monochrome_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                            && (!sprite.spatial_id.is_identity()) == transformed
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.monochrome_sprites_start = sprites_end;
                Some(PrimitiveBatch::MonochromeSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::SubpixelSprite => {
                let texture_id = self.subpixel_sprites_iter.peek().unwrap().tile.texture_id;
                let transformed = !self
                    .subpixel_sprites_iter
                    .peek()
                    .unwrap()
                    .spatial_id
                    .is_identity();
                let sprites_start = self.subpixel_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.subpixel_sprites_iter.next();
                while self
                    .subpixel_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                            && (!sprite.spatial_id.is_identity()) == transformed
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.subpixel_sprites_start = sprites_end;
                Some(PrimitiveBatch::SubpixelSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::PolychromeSprite => {
                let texture_id = self.polychrome_sprites_iter.peek().unwrap().tile.texture_id;
                let transformed = !self
                    .polychrome_sprites_iter
                    .peek()
                    .unwrap()
                    .spatial_id
                    .is_identity();
                let sprites_start = self.polychrome_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.polychrome_sprites_iter.next();
                while self
                    .polychrome_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                            && (!sprite.spatial_id.is_identity()) == transformed
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.polychrome_sprites_start = sprites_end;
                Some(PrimitiveBatch::PolychromeSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::Surface => {
                let surfaces_start = self.surfaces_start;
                let mut surfaces_end = surfaces_start + 1;
                let transformed = !self.surfaces_iter.peek().unwrap().spatial_id.is_identity();
                self.surfaces_iter.next();
                while self
                    .surfaces_iter
                    .next_if(|surface| {
                        (surface.order, batch_kind) < max_order_and_kind
                            && (!surface.spatial_id.is_identity()) == transformed
                    })
                    .is_some()
                {
                    surfaces_end += 1;
                }
                self.surfaces_start = surfaces_end;
                Some(PrimitiveBatch::Surfaces(surfaces_start..surfaces_end))
            }
        }
    }
}

#[derive(Debug)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
#[allow(missing_docs)]
pub enum PrimitiveBatch {
    Shadows(Range<usize>),
    Quads(Range<usize>),
    Paths(Range<usize>),
    Underlines(Range<usize>),
    MonochromeSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    SubpixelSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    PolychromeSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    Surfaces(Range<usize>),
}

impl PrimitiveBatch {
    #[expect(missing_docs)]
    pub fn label(&self) -> String {
        match self {
            Self::Shadows(range) => format!("shadows ({})", range.len()),
            Self::Quads(range) => format!("quads ({})", range.len()),
            Self::Paths(range) => format!("paths ({})", range.len()),
            Self::Underlines(range) => format!("underlines ({})", range.len()),
            Self::MonochromeSprites { texture_id, range } => {
                format!(
                    "monochrome sprites ({}) on atlas {}",
                    range.len(),
                    texture_id.index
                )
            }
            Self::SubpixelSprites { texture_id, range } => {
                format!(
                    "subpixel sprites ({}) on atlas {}",
                    range.len(),
                    texture_id.index
                )
            }
            Self::PolychromeSprites { texture_id, range } => {
                format!(
                    "polychrome sprites ({}) on atlas {}",
                    range.len(),
                    texture_id.index
                )
            }
            Self::Surfaces(range) => format!("surfaces ({})", range.len()),
        }
    }
}

#[derive(Default, Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Quad {
    pub order: DrawOrder,
    pub border_style: BorderStyle,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub background: Background,
    pub border_color: Hsla,
    pub corner_radii: Corners<ScaledPixels>,
    pub border_widths: Edges<ScaledPixels>,
    pub spatial_id: SpatialId,
}

impl From<Quad> for Primitive {
    fn from(quad: Quad) -> Self {
        Primitive::Quad(quad)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Underline {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub thickness: ScaledPixels,
    pub wavy: PaddedBool32,
    pub spatial_id: SpatialId,
}

impl From<Underline> for Primitive {
    fn from(underline: Underline) -> Self {
        Primitive::Underline(underline)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Shadow {
    pub order: DrawOrder,
    pub blur_radius: ScaledPixels,
    pub bounds: Bounds<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub element_bounds: Bounds<ScaledPixels>,
    pub element_corner_radii: Corners<ScaledPixels>,
    /// 0 = drop shadow (rendered outside the element), 1 = inset shadow (rendered inside).
    pub inset: u32,
    pub pad: u32, // align to 8 bytes
    pub spatial_id: SpatialId,
}

impl From<Shadow> for Primitive {
    fn from(shadow: Shadow) -> Self {
        Primitive::Shadow(shadow)
    }
}

/// Compact reference to sparse affine paint state. Zero is always identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct SpatialId(pub u32);

impl SpatialId {
    /// The untransformed root state. Primitives with this ID use the ordinary renderer path.
    pub const IDENTITY: Self = Self(0);

    /// Whether this ID denotes the untransformed root state.
    #[inline]
    pub fn is_identity(self) -> bool {
        self == Self::IDENTITY
    }

    #[inline]
    fn index(self) -> Option<usize> {
        self.0.checked_sub(1).map(|value| value as usize)
    }
}

/// Affine paint state shared by every primitive under one transformed subtree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct SpatialState {
    /// Maps layout coordinates to window coordinates, in device pixels.
    pub matrix: TransformationMatrix,
    /// First cross-space ancestor clip in `Scene::transform_clips`.
    pub clip_start: u32,
    /// Number of cross-space ancestor clips.
    pub clip_count: u32,
}

/// A clip rectangle and the inverse transform into its coordinate space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct TransformedClip {
    /// Maps window coordinates into the clip's coordinates.
    pub inverse: TransformationMatrix,
    /// Rectangle before transformation.
    pub bounds: Bounds<ScaledPixels>,
}

/// The style of a border.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub enum BorderStyle {
    /// A solid border.
    #[default]
    Solid = 0,
    /// A dashed border.
    Dashed = 1,
}

/// A data type representing a 2 dimensional transformation that can be applied to an element.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct TransformationMatrix {
    /// 2x2 matrix containing rotation and scale,
    /// stored row-major
    pub rotation_scale: [[f32; 2]; 2],
    /// translation vector
    pub translation: [f32; 2],
}

impl Eq for TransformationMatrix {}

impl TransformationMatrix {
    /// The unit matrix, has no effect.
    pub fn unit() -> Self {
        Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [0.0, 0.0],
        }
    }

    /// Move the origin by a given point
    pub fn translate(mut self, point: Point<ScaledPixels>) -> Self {
        self.compose(Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [point.x.0, point.y.0],
        })
    }

    /// Clockwise rotation in radians around the origin
    pub fn rotate(self, angle: Radians) -> Self {
        self.compose(Self {
            rotation_scale: [
                [angle.0.cos(), -angle.0.sin()],
                [angle.0.sin(), angle.0.cos()],
            ],
            translation: [0.0, 0.0],
        })
    }

    /// Scale around the origin
    pub fn scale(self, size: Size<f32>) -> Self {
        self.compose(Self {
            rotation_scale: [[size.width, 0.0], [0.0, size.height]],
            translation: [0.0, 0.0],
        })
    }

    /// Perform matrix multiplication with another transformation
    /// to produce a new transformation that is the result of
    /// applying both transformations: first, `other`, then `self`.
    #[inline]
    pub fn compose(self, other: TransformationMatrix) -> TransformationMatrix {
        if other == Self::unit() {
            return self;
        }
        // Perform matrix multiplication
        TransformationMatrix {
            rotation_scale: [
                [
                    self.rotation_scale[0][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][0],
                    self.rotation_scale[0][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][1],
                ],
                [
                    self.rotation_scale[1][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][0],
                    self.rotation_scale[1][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][1],
                ],
            ],
            translation: [
                self.translation[0]
                    + self.rotation_scale[0][0] * other.translation[0]
                    + self.rotation_scale[0][1] * other.translation[1],
                self.translation[1]
                    + self.rotation_scale[1][0] * other.translation[0]
                    + self.rotation_scale[1][1] * other.translation[1],
            ],
        }
    }

    /// Change the units of the translation while preserving the linear part.
    pub fn scaled(mut self, factor: f32) -> Self {
        self.translation[0] *= factor;
        self.translation[1] *= factor;
        self
    }

    /// Invert a finite, nonsingular affine transform.
    pub fn inverse(self) -> Option<Self> {
        let [[a, c], [b, d]] = self.rotation_scale;
        let determinant = a * d - b * c;
        if determinant == 0.0 || !determinant.is_finite() {
            return None;
        }
        let inverse = Self {
            rotation_scale: [
                [d / determinant, -c / determinant],
                [-b / determinant, a / determinant],
            ],
            translation: [0.0, 0.0],
        };
        let offset = inverse.apply(point(
            Pixels(-self.translation[0]),
            Pixels(-self.translation[1]),
        ));
        let result = Self {
            translation: [offset.x.0, offset.y.0],
            ..inverse
        };
        result
            .rotation_scale
            .iter()
            .flatten()
            .chain(result.translation.iter())
            .all(|v| v.is_finite())
            .then_some(result)
    }

    /// The axis-aligned bounds enclosing all four transformed corners.
    pub fn transform_bounds(self, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        let corners = [
            bounds.origin,
            bounds.top_right(),
            bounds.bottom_left(),
            bounds.bottom_right(),
        ];
        let first = self.apply(corners[0]);
        let (mut min, mut max) = (first, first);
        for corner in &corners[1..] {
            let point = self.apply(*corner);
            min = min.min(&point);
            max = max.max(&point);
        }
        Bounds::from_corners(min, max)
    }

    /// The device-pixel counterpart of `transform_bounds`.
    pub fn transform_scaled_bounds(self, bounds: Bounds<ScaledPixels>) -> Bounds<ScaledPixels> {
        self.transform_bounds(bounds.map(|value| Pixels(value.0)))
            .map(|value| ScaledPixels(value.0))
    }

    /// Apply transformation to a point.
    pub fn apply(&self, point: Point<Pixels>) -> Point<Pixels> {
        let input = [point.x.0, point.y.0];
        let mut output = self.translation;
        for (i, output_cell) in output.iter_mut().enumerate() {
            for (k, input_cell) in input.iter().enumerate() {
                *output_cell += self.rotation_scale[i][k] * *input_cell;
            }
        }
        Point::new(output[0].into(), output[1].into())
    }
}

impl Default for TransformationMatrix {
    fn default() -> Self {
        Self::unit()
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct MonochromeSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
    pub spatial_id: SpatialId,
}

impl From<MonochromeSprite> for Primitive {
    fn from(sprite: MonochromeSprite) -> Self {
        Primitive::MonochromeSprite(sprite)
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct SubpixelSprite {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
    pub spatial_id: SpatialId,
}

impl From<SubpixelSprite> for Primitive {
    fn from(sprite: SubpixelSprite) -> Self {
        Primitive::SubpixelSprite(sprite)
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct PolychromeSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub grayscale: PaddedBool32,
    pub opacity: f32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub tile: AtlasTile,
    pub spatial_id: SpatialId,
}

impl From<PolychromeSprite> for Primitive {
    fn from(sprite: PolychromeSprite) -> Self {
        Primitive::PolychromeSprite(sprite)
    }
}

#[derive(Clone, Debug)]
#[allow(missing_docs)]
pub struct PaintSurface {
    pub order: DrawOrder,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    #[cfg(target_os = "macos")]
    pub image_buffer: core_video::pixel_buffer::CVPixelBuffer,
    pub spatial_id: SpatialId,
}

impl From<PaintSurface> for Primitive {
    fn from(surface: PaintSurface) -> Self {
        Primitive::Surface(surface)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[expect(missing_docs)]
pub struct PathId(pub usize);

/// A line made up of a series of vertices and control points.
#[derive(Clone, Debug)]
#[expect(missing_docs)]
pub struct Path<P: Clone + Debug + Default + PartialEq> {
    pub id: PathId,
    pub order: DrawOrder,
    pub bounds: Bounds<P>,
    pub content_mask: ContentMask<P>,
    pub vertices: Vec<PathVertex<P>>,
    pub spatial_id: SpatialId,
    pub color: Background,
    start: Point<P>,
    current: Point<P>,
    contour_count: usize,
}

impl Path<Pixels> {
    /// Create a new path with the given starting point.
    pub fn new(start: Point<Pixels>) -> Self {
        Self {
            id: PathId(0),
            spatial_id: SpatialId::IDENTITY,
            order: DrawOrder::default(),
            vertices: Vec::new(),
            start,
            current: start,
            bounds: Bounds {
                origin: start,
                size: Default::default(),
            },
            content_mask: Default::default(),
            color: Default::default(),
            contour_count: 0,
        }
    }

    /// Scale this path by the given factor.
    pub fn scale(&self, factor: f32) -> Path<ScaledPixels> {
        Path {
            id: self.id,
            spatial_id: self.spatial_id,
            order: self.order,
            bounds: self.bounds.scale(factor),
            content_mask: self.content_mask.scale(factor),
            vertices: self
                .vertices
                .iter()
                .map(|vertex| vertex.scale(factor))
                .collect(),
            start: self.start.map(|start| start.scale(factor)),
            current: self.current.scale(factor),
            contour_count: self.contour_count,
            color: self.color,
        }
    }

    /// Move the start, current point to the given point.
    pub fn move_to(&mut self, to: Point<Pixels>) {
        self.contour_count += 1;
        self.start = to;
        self.current = to;
    }

    /// Draw a straight line from the current point to the given point.
    pub fn line_to(&mut self, to: Point<Pixels>) {
        self.contour_count += 1;
        if self.contour_count > 1 {
            self.push_triangle(
                (self.start, self.current, to),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }
        self.current = to;
    }

    /// Draw a curve from the current point to the given point, using the given control point.
    pub fn curve_to(&mut self, to: Point<Pixels>, ctrl: Point<Pixels>) {
        self.contour_count += 1;
        if self.contour_count > 1 {
            self.push_triangle(
                (self.start, self.current, to),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }

        self.push_triangle(
            (self.current, ctrl, to),
            (point(0., 0.), point(0.5, 0.), point(1., 1.)),
        );
        self.current = to;
    }

    /// Push a triangle to the Path.
    pub fn push_triangle(
        &mut self,
        xy: (Point<Pixels>, Point<Pixels>, Point<Pixels>),
        st: (Point<f32>, Point<f32>, Point<f32>),
    ) {
        self.bounds = self
            .bounds
            .union(&Bounds {
                origin: xy.0,
                size: Default::default(),
            })
            .union(&Bounds {
                origin: xy.1,
                size: Default::default(),
            })
            .union(&Bounds {
                origin: xy.2,
                size: Default::default(),
            });

        self.vertices.push(PathVertex {
            xy_position: xy.0,
            st_position: st.0,
            content_mask: Default::default(),
        });
        self.vertices.push(PathVertex {
            xy_position: xy.1,
            st_position: st.1,
            content_mask: Default::default(),
        });
        self.vertices.push(PathVertex {
            xy_position: xy.2,
            st_position: st.2,
            content_mask: Default::default(),
        });
    }
}

impl<T> Path<T>
where
    T: Clone + Debug + Default + PartialEq + PartialOrd + Add<T, Output = T> + Sub<Output = T>,
{
    #[allow(unused)]
    #[expect(missing_docs)]
    pub fn clipped_bounds(&self) -> Bounds<T> {
        self.bounds.intersect(&self.content_mask.bounds)
    }
}

impl Path<ScaledPixels> {
    /// Window-space bounds used when copying rasterized paths.
    pub fn transformed_bounds(&self, matrix: TransformationMatrix) -> Bounds<ScaledPixels> {
        matrix.transform_scaled_bounds(self.clipped_bounds())
    }
}

impl From<Path<ScaledPixels>> for Primitive {
    fn from(path: Path<ScaledPixels>) -> Self {
        Primitive::Path(path)
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct PathVertex<P: Clone + Debug + Default + PartialEq> {
    pub xy_position: Point<P>,
    pub st_position: Point<f32>,
    pub content_mask: ContentMask<P>,
}

#[expect(missing_docs)]
impl PathVertex<Pixels> {
    pub fn scale(&self, factor: f32) -> PathVertex<ScaledPixels> {
        PathVertex {
            xy_position: self.xy_position.scale(factor),
            st_position: self.st_position,
            content_mask: self.content_mask.scale(factor),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32, w: f32, h: f32) -> Bounds<ScaledPixels> {
        Bounds {
            origin: point(ScaledPixels(x), ScaledPixels(y)),
            size: Size {
                width: ScaledPixels(w),
                height: ScaledPixels(h),
            },
        }
    }

    fn prim(order: DrawOrder, x: f32, y: f32, w: f32, h: f32) -> Primitive {
        Primitive::Quad(Quad {
            spatial_id: SpatialId::IDENTITY,
            order,
            border_style: BorderStyle::Solid,
            bounds: at(x, y, w, h),
            content_mask: ContentMask {
                bounds: at(0.0, 0.0, 1000.0, 1000.0),
            },
            background: Background::default(),
            border_color: Hsla::default(),
            corner_radii: Corners::default(),
            border_widths: Edges::default(),
        })
    }

    fn underline(order: DrawOrder, x: f32, y: f32, w: f32, h: f32) -> Primitive {
        Primitive::Underline(Underline {
            spatial_id: SpatialId::IDENTITY,
            order,
            pad: 0,
            bounds: at(x, y, w, h),
            content_mask: ContentMask {
                bounds: at(0.0, 0.0, 1000.0, 1000.0),
            },
            color: Hsla::default(),
            thickness: ScaledPixels(1.0),
            wavy: false.into(),
        })
    }

    fn batch_labels(scene: &Scene) -> Vec<String> {
        scene.batches().map(|batch| batch.label()).collect()
    }

    #[test]
    fn affine_inverse_and_bounds_include_all_corners() {
        let matrix = TransformationMatrix::unit()
            .translate(point(ScaledPixels(30.0), ScaledPixels(20.0)))
            .rotate(Radians(std::f32::consts::FRAC_PI_2))
            .scale(crate::size(-2.0, 3.0));
        let point = point(Pixels(4.0), Pixels(5.0));
        let roundtrip = matrix.inverse().unwrap().apply(matrix.apply(point));
        assert!((roundtrip.x - point.x).abs() < Pixels(0.001));
        assert!((roundtrip.y - point.y).abs() < Pixels(0.001));
        let bounds = matrix.transform_scaled_bounds(at(0.0, 0.0, 10.0, 5.0));
        assert!((bounds.size.width.0 - 15.0).abs() < 0.001);
        assert!((bounds.size.height.0 - 20.0).abs() < 0.001);
        assert!(
            TransformationMatrix::unit()
                .scale(crate::size(0.0, 1.0))
                .inverse()
                .is_none()
        );
    }

    #[test]
    fn transformed_scene_order_and_replay_keep_clip_references() {
        let mut previous = Scene::default();
        let clip = TransformedClip {
            inverse: TransformationMatrix::unit(),
            bounds: at(0.0, 0.0, 500.0, 500.0),
        };
        previous.transform_clips.push(clip);
        previous.spatial_id = previous.push_spatial_state(SpatialState {
            matrix: TransformationMatrix::unit()
                .translate(point(ScaledPixels(100.0), ScaledPixels(0.0))),
            clip_start: 0,
            clip_count: 1,
        });
        previous.insert_primitive(prim(0, 0.0, 0.0, 40.0, 40.0));
        previous.spatial_id = SpatialId::IDENTITY;
        previous.insert_primitive(prim(0, 110.0, 10.0, 20.0, 20.0));
        assert!(previous.quads[1].order > previous.quads[0].order);

        let mut replayed = Scene::default();
        replayed.transform_clips.push(TransformedClip::default());
        replayed.replay(0..previous.len(), &previous);
        let state = replayed.spatial_states[replayed.quads[0].spatial_id.index().unwrap()];
        assert_eq!(
            state.matrix,
            previous.spatial_states[previous.quads[0].spatial_id.index().unwrap()].matrix
        );
        assert_eq!(state.clip_start, 1);
        assert_eq!(replayed.transform_clips[state.clip_start as usize], clip);
        assert!(replayed.quads[1].order > replayed.quads[0].order);
    }

    #[test]
    fn replay_preserves_overlap_order_inside_and_across_boundaries() {
        let fragment = [
            prim(0, 5.0, 5.0, 50.0, 50.0),
            prim(0, 10.0, 10.0, 50.0, 50.0),
            prim(0, 15.0, 15.0, 50.0, 50.0),
        ];

        let mut expected = Scene::default();
        expected.insert_primitive(prim(0, 0.0, 0.0, 60.0, 60.0));
        for primitive in &fragment {
            expected.insert_primitive(primitive.clone());
        }
        expected.insert_primitive(prim(0, 20.0, 20.0, 60.0, 60.0));
        expected.finish();

        let mut cached = Scene::default();
        for primitive in &fragment {
            cached.insert_primitive(primitive.clone());
        }

        let mut replayed = Scene::default();
        replayed.insert_primitive(prim(0, 0.0, 0.0, 60.0, 60.0));
        replayed.replay(0..cached.len(), &cached);
        replayed.insert_primitive(prim(0, 20.0, 20.0, 60.0, 60.0));
        replayed.finish();

        let expected_orders: Vec<DrawOrder> =
            expected.quads.iter().map(|quad| quad.order).collect();
        let replayed_orders: Vec<DrawOrder> =
            replayed.quads.iter().map(|quad| quad.order).collect();
        assert_eq!(expected_orders, replayed_orders);
        assert!(replayed_orders.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn replay_preserves_cross_kind_batch_order() {
        // An underline is painted first, then a quad on top. Collapsing the
        // fragment into one order would let the kind tie-break float the quad
        // below the underline, changing the batch sequence.
        let fragment = [
            underline(0, 0.0, 0.0, 10.0, 10.0),
            prim(0, 0.0, 0.0, 10.0, 10.0),
        ];

        let mut expected = Scene::default();
        for primitive in &fragment {
            expected.insert_primitive(primitive.clone());
        }
        expected.finish();

        let mut cached = Scene::default();
        for primitive in &fragment {
            cached.insert_primitive(primitive.clone());
        }

        let mut replayed = Scene::default();
        replayed.replay(0..cached.len(), &cached);
        replayed.finish();

        assert_eq!(batch_labels(&expected), batch_labels(&replayed));
        assert!(replayed.underlines[0].order < replayed.quads[0].order);
    }

    #[test]
    fn replay_preserves_layer_markers_and_relative_order() {
        let mut cached = Scene::default();
        cached
            .paint_operations
            .push(PaintOperation::Primitive(prim(1, 0.0, 0.0, 10.0, 10.0)));
        cached
            .paint_operations
            .push(PaintOperation::StartLayer(at(0.0, 0.0, 10.0, 10.0)));
        cached
            .paint_operations
            .push(PaintOperation::Primitive(prim(5, 0.0, 0.0, 10.0, 10.0)));
        cached.paint_operations.push(PaintOperation::EndLayer);

        let mut replayed = Scene::default();
        replayed.replay(0..cached.len(), &cached);

        assert_eq!(replayed.len(), cached.len());
        assert!(matches!(
            replayed.paint_operations[0],
            PaintOperation::Primitive(_)
        ));
        assert!(matches!(
            replayed.paint_operations[1],
            PaintOperation::StartLayer(_)
        ));
        assert!(matches!(
            replayed.paint_operations[2],
            PaintOperation::Primitive(_)
        ));
        assert!(matches!(
            replayed.paint_operations[3],
            PaintOperation::EndLayer
        ));
        assert!(replayed.quads[1].order > replayed.quads[0].order);
    }

    #[test]
    fn replay_inside_active_layer_inherits_current_layer_order() {
        let mut cached = Scene::default();
        cached.insert_primitive(prim(0, 0.0, 0.0, 5.0, 5.0));
        cached.insert_primitive(prim(0, 2.0, 2.0, 5.0, 5.0));
        assert!(cached.quads[0].order < cached.quads[1].order);

        let mut replayed = Scene::default();
        replayed.push_layer(at(0.0, 0.0, 100.0, 100.0));
        let layer_order = *replayed.layer_stack.last().unwrap();
        replayed.replay(0..cached.len(), &cached);
        replayed.pop_layer();

        // Both cached primitives must inherit the enclosing layer's order,
        // not their shifted internal orders.
        assert_eq!(replayed.quads[0].order, layer_order);
        assert_eq!(replayed.quads[1].order, layer_order);
        assert!(replayed.layer_stack.is_empty());
    }

    #[test]
    fn replay_leaves_enclosing_layer_open_for_later_primitives() {
        let mut cached = Scene::default();
        cached
            .paint_operations
            .push(PaintOperation::StartLayer(at(0.0, 0.0, 50.0, 50.0)));
        cached
            .paint_operations
            .push(PaintOperation::Primitive(prim(1, 0.0, 0.0, 5.0, 5.0)));

        let mut replayed = Scene::default();
        replayed.replay(0..cached.len(), &cached);
        let layer_order = *replayed.layer_stack.last().unwrap();

        // A live primitive after the range must inherit the open layer.
        replayed.insert_primitive(prim(0, 1.0, 1.0, 5.0, 5.0));
        assert_eq!(replayed.quads[1].order, layer_order);

        replayed.pop_layer();
        assert!(replayed.layer_stack.is_empty());
    }

    #[test]
    fn replay_registers_clipped_fragment_extent() {
        let mut cached = Scene::default();
        let Primitive::Quad(mut fragment) = prim(3, 0.0, 0.0, 100.0, 100.0) else {
            unreachable!()
        };
        fragment.content_mask = ContentMask {
            bounds: at(0.0, 0.0, 10.0, 10.0),
        };
        cached.insert_primitive(Primitive::Quad(fragment));

        let mut replayed = Scene::default();
        replayed.replay(0..cached.len(), &cached);

        // Overlaps the raw bounds but not the clipped extent, so it keeps the
        // fragment's order instead of being forced above it.
        replayed.insert_primitive(prim(0, 50.0, 50.0, 5.0, 5.0));
        assert_eq!(replayed.quads[0].order, replayed.quads[1].order);
    }
}
