//! CPU-only `tiny-skia` render backend — a spike to test whether a direct 2D
//! rasterizer beats wgpu-on-lavapipe on CPU load (see
//! `ruffle/claude/tiny-skia-render-backend-plan.md`).
//!
//! Non-goals, all stubbed unsupported: masks, blend-mode compositing, filters,
//! offscreen bitmap caching, PixelBender, Context3D. Bitmap *fills* render flat
//! for now; photographic content still goes through `render_bitmap`.

#![allow(clippy::arc_with_non_send_sync)]

use std::any::Any;
use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt::{self, Debug};
use std::num::NonZeroU32;
use std::sync::Arc;

use ruffle_render::backend::{
    BitmapCacheEntry, Context3D, Context3DProfile, PixelBenderOutput, PixelBenderTarget,
    RenderBackend, ShapeHandle, ShapeHandleImpl, ViewportDimensions,
};
use ruffle_render::bitmap::{
    Bitmap, BitmapHandle, BitmapHandleImpl, BitmapSource, PixelRegion, PixelSnapping, RgbaBufRead,
    SyncHandle,
};
use ruffle_render::commands::{CommandHandler, CommandList, RenderBlendMode};
use ruffle_render::error::Error;
use ruffle_render::lines::emulate_line_rect;
use ruffle_render::matrix::Matrix;
use ruffle_render::pixel_bender::{PixelBenderShader, PixelBenderShaderHandle};
use ruffle_render::pixel_bender_support::PixelBenderShaderArgument;
use ruffle_render::quality::StageQuality;
use ruffle_render::shape_utils::{DistilledShape, DrawCommand, DrawPath, FillRule};
use swf::{Color, ColorTransform, FillStyle, Gradient, GradientSpread};
use tiny_skia::{
    FillRule as SkFillRule, FilterQuality, GradientStop, IntRect, IntSize, LinearGradient, Paint,
    Path, Mask, PathBuilder, Pattern, Pixmap, PixmapPaint, PixmapRef, Point, RadialGradient, Shader,
    SpreadMode, Stroke,
    Transform as SkTransform,
};

/// Twips-to-pixel scale. Paths and matrices are kept in twips (as `canvas` does)
/// and scaled to device pixels at paint time.
const TWIPS_TO_PIXELS: f32 = 0.05;

/// The gradient square SWF gradients are defined in: x/y span ±16384 twips.
const GRADIENT_HALF: f32 = 16384.0;

#[derive(Default, Debug)]
pub struct SpikeStats {
    pub shapes: u32,
    pub bitmaps: u32,
    pub rects: u32,
    pub lines: u32,
    pub masks: u32,
    pub alpha_masks: u32,
    pub blends: u32,
    pub offscreen: u32,
    pub bitmap_fills: u32,
}

pub struct TinySkiaRenderBackend {
    pub stats: SpikeStats,
    frame: Pixmap,
    masks: MaskStack,
    dimensions: ViewportDimensions,
}

impl TinySkiaRenderBackend {
    pub fn new(width: u32, height: u32) -> Self {
        let frame = new_pixmap(width, height);
        Self {
            stats: SpikeStats::default(),
            frame,
            masks: MaskStack::default(),
            dimensions: ViewportDimensions {
                width: width.max(1),
                height: height.max(1),
                scale_factor: 1.0,
            },
        }
    }

    /// The rendered frame, for a harness to read back (the trait has no present).
    pub fn frame(&self) -> &Pixmap {
        &self.frame
    }

    /// Bitmap fills must be resolved while the shape's `BitmapSource` is at
    /// hand; everything else becomes a paint straight away.
    fn shape_paint(&mut self, style: &FillStyle, source: &dyn BitmapSource) -> SkPaint {
        if let FillStyle::Bitmap {
            id,
            matrix,
            is_smoothed,
            is_repeating,
        } = style
        {
            match source.bitmap_handle(*id, self) {
                Some(handle) => {
                    return SkPaint::Bitmap(SkBitmapFill {
                        handle,
                        matrix: *matrix,
                        smoothed: *is_smoothed,
                        repeating: *is_repeating,
                    });
                }
                None => log::warn!("shape fills with unknown bitmap {id}"),
            }
        }
        SkPaint::Solid(Box::new(fill_paint(style, &ColorTransform::IDENTITY)))
    }
}

/// Ruffle's mask protocol: `push_mask`, the mask's own geometry,
/// `activate_mask`, the maskee, `deactivate_mask`, the same geometry once more
/// (wgpu replays it to clear its stencil), `pop_mask`. A mask can also be
/// pushed and popped again without ever being activated, so each level carries
/// its own state rather than sharing one depth counter.
enum MaskLevel {
    /// Geometry submitted now *defines* this mask instead of being drawn.
    Building(BoundedMask),
    /// This mask clips every draw until it is popped.
    Active(BoundedMask),
}

/// A full-frame mask that remembers which pixels it has touched. Ruffle
/// pushes a mask per text field (`edit_text.rs`), so a busy screen builds
/// dozens per frame; clearing and intersecting the whole frame for each of
/// them was two thirds of the Space Theme's render time on the Pi. Only the
/// box is ever written, cleared or multiplied; outside it the mask is zero.
struct BoundedMask {
    mask: Mask,
    /// Pixels that may be non-zero. `None` means the mask is all zero, i.e.
    /// an empty clip.
    dirty: Option<IntRect>,
}

impl BoundedMask {
    fn new(width: u32, height: u32) -> Option<Self> {
        Mask::new(width, height).map(|mask| Self { mask, dirty: None })
    }

    fn fill(&mut self, path: &Path, rule: SkFillRule, transform: SkTransform) {
        let Some(bounds) = pixel_bounds(path, transform, self.mask.width(), self.mask.height())
        else {
            return;
        };
        self.mask.fill_path(path, rule, true, transform);
        self.dirty = Some(match self.dirty {
            Some(d) => union(d, bounds),
            None => bounds,
        });
    }

    /// Zero the touched box; the rest never changed.
    fn clear_dirty(&mut self) {
        let Some(d) = self.dirty.take() else { return };
        let width = self.mask.width() as usize;
        let data = self.mask.data_mut();
        for y in d.top() as usize..d.bottom() as usize {
            data[y * width + d.left() as usize..y * width + d.right() as usize].fill(0);
        }
    }
}

/// Device-pixel box a path covers under `transform`, one pixel wider for
/// anti-aliasing, clamped to the mask. Degenerate paths give `None` — they
/// fill nothing, and tiny-skia would only log a warning for them.
fn pixel_bounds(path: &Path, transform: SkTransform, width: u32, height: u32) -> Option<IntRect> {
    let b = path.bounds();
    // tiny-skia's own SCALAR_NEARLY_ZERO (1/4096), the threshold its fill_path warns at.
    if b.width() < 1.0 / 4096.0 || b.height() < 1.0 / 4096.0 {
        return None;
    }
    let mut corners = [
        Point::from_xy(b.left(), b.top()),
        Point::from_xy(b.right(), b.top()),
        Point::from_xy(b.left(), b.bottom()),
        Point::from_xy(b.right(), b.bottom()),
    ];
    transform.map_points(&mut corners);
    let (mut l, mut t, mut r, mut bt) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for c in corners {
        l = l.min(c.x);
        t = t.min(c.y);
        r = r.max(c.x);
        bt = bt.max(c.y);
    }
    if !(l.is_finite() && t.is_finite() && r.is_finite() && bt.is_finite()) {
        return None;
    }
    let l = (l.floor() - 1.0).max(0.0) as i32;
    let t = (t.floor() - 1.0).max(0.0) as i32;
    let r = (r.ceil() + 1.0).min(width as f32) as i32;
    let bt = (bt.ceil() + 1.0).min(height as f32) as i32;
    if l >= r || t >= bt {
        return None;
    }
    IntRect::from_ltrb(l, t, r, bt)
}

fn union(a: IntRect, b: IntRect) -> IntRect {
    IntRect::from_ltrb(
        a.left().min(b.left()),
        a.top().min(b.top()),
        a.right().max(b.right()),
        a.bottom().max(b.bottom()),
    )
    .unwrap_or(a)
}

fn intersection(a: IntRect, b: IntRect) -> Option<IntRect> {
    let l = a.left().max(b.left());
    let t = a.top().max(b.top());
    let r = a.right().min(b.right());
    let bt = a.bottom().min(b.bottom());
    if l >= r || t >= bt {
        return None;
    }
    IntRect::from_ltrb(l, t, r, bt)
}

#[derive(Default)]
struct MaskStack {
    levels: Vec<MaskLevel>,
    /// Retired masks, kept to avoid re-allocating ~w*h bytes every frame.
    spare: Vec<BoundedMask>,
}

impl MaskStack {
    fn defining(&self) -> bool {
        matches!(self.levels.last(), Some(MaskLevel::Building(_)))
    }

    /// The innermost mask that is actually clipping.
    fn clip(&self) -> Option<&Mask> {
        self.active().map(|m| &m.mask)
    }

    /// The innermost active clip lets nothing through: skip the draw.
    fn clip_is_empty(&self) -> bool {
        matches!(self.active(), Some(m) if m.dirty.is_none())
    }

    fn active(&self) -> Option<&BoundedMask> {
        self.levels.iter().rev().find_map(|level| match level {
            MaskLevel::Active(mask) => Some(mask),
            MaskLevel::Building(_) => None,
        })
    }

    /// Geometry for the mask being defined; ignored when none is.
    fn fill(&mut self, path: &Path, rule: SkFillRule, transform: SkTransform) {
        if let Some(MaskLevel::Building(mask)) = self.levels.last_mut() {
            mask.fill(path, rule, transform);
        }
    }

    fn take(&mut self, width: u32, height: u32) -> Option<BoundedMask> {
        while let Some(mut mask) = self.spare.pop() {
            if mask.mask.width() == width && mask.mask.height() == height {
                mask.clear_dirty();
                return Some(mask);
            }
        }
        BoundedMask::new(width, height)
    }

    fn push(&mut self, width: u32, height: u32) {
        if let Some(mask) = self.take(width, height) {
            self.levels.push(MaskLevel::Building(mask));
        }
    }

    /// The mask is complete: intersect it with any enclosing clip and apply it.
    fn activate(&mut self) {
        if !self.defining() {
            return;
        }
        let Some(MaskLevel::Building(mut mask)) = self.levels.pop() else {
            return;
        };
        if let Some(outer) = self.active() {
            intersect(&mut mask, outer);
        }
        self.levels.push(MaskLevel::Active(mask));
    }

    /// The maskee is done; what follows is the mask's geometry again, which
    /// only wgpu's stencil needs. Collect it somewhere it can be thrown away.
    fn deactivate(&mut self, width: u32, height: u32) {
        if matches!(self.levels.last(), Some(MaskLevel::Active(_)))
            && let Some(MaskLevel::Active(mask)) = self.levels.pop()
        {
            self.spare.push(mask);
        }
        self.push(width, height);
    }

    fn pop(&mut self) {
        if let Some(level) = self.levels.pop() {
            self.spare.push(match level {
                MaskLevel::Building(mask) | MaskLevel::Active(mask) => mask,
            });
        }
    }

    /// A frame must not inherit a clip from an unbalanced previous one.
    fn reset(&mut self) {
        for level in self.levels.drain(..) {
            self.spare.push(match level {
                MaskLevel::Building(mask) | MaskLevel::Active(mask) => mask,
            });
        }
    }
}

/// tiny-skia can intersect a mask with a *path* but not with another mask.
/// Only the inner box needs the multiply: outside it the inner mask is
/// already zero, and outside the outer's box the product becomes zero.
fn intersect(mask: &mut BoundedMask, other: &BoundedMask) {
    if mask.mask.width() != other.mask.width() || mask.mask.height() != other.mask.height() {
        return;
    }
    let Some(inner) = mask.dirty else { return };
    let width = mask.mask.width() as usize;
    let data = mask.mask.data_mut();
    let other_data = other.mask.data();
    for y in inner.top() as usize..inner.bottom() as usize {
        let row = y * width + inner.left() as usize..y * width + inner.right() as usize;
        for (a, b) in data[row.clone()].iter_mut().zip(&other_data[row]) {
            *a = ((*a as u16 * *b as u16 + 127) / 255) as u8;
        }
    }
    mask.dirty = other.dirty.and_then(|o| intersection(inner, o));
}

fn new_pixmap(width: u32, height: u32) -> Pixmap {
    Pixmap::new(width.max(1), height.max(1)).expect("non-zero pixmap dimensions")
}

// --- Handle types -----------------------------------------------------------

enum SkDraw {
    Fill {
        path: Path,
        style: FillStyle,
        paint: SkPaint,
        rule: SkFillRule,
    },
    Stroke {
        path: Path,
        style: FillStyle,
        paint: SkPaint,
        width: f32,
    },
}

enum SkPaint {
    /// Colour or gradient: independent of any bitmap, so built once.
    Solid(Box<Paint<'static>>),
    /// A bitmap fill's `Pattern` borrows the bitmap, which lives behind a
    /// `RefCell`, so its paint can only exist inside the borrow at draw time.
    Bitmap(SkBitmapFill),
}

struct SkBitmapFill {
    handle: BitmapHandle,
    /// Maps the bitmap's pixel grid into the shape's twips space.
    matrix: swf::Matrix,
    smoothed: bool,
    repeating: bool,
}

struct SkShape(Vec<SkDraw>);

impl ShapeHandleImpl for SkShape {}

impl Debug for SkShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SkShape({} draws)", self.0.len())
    }
}

struct SkBitmap {
    pixmap: RefCell<Pixmap>,
}

impl BitmapHandleImpl for SkBitmap {}

impl Debug for SkBitmap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pixmap = self.pixmap.borrow();
        write!(f, "SkBitmap({}x{})", pixmap.width(), pixmap.height())
    }
}

fn as_sk_shape(handle: &ShapeHandle) -> &SkShape {
    <dyn Any>::downcast_ref(&*handle.0).expect("shape handle must be a tiny-skia SkShape")
}

fn as_sk_bitmap(handle: &BitmapHandle) -> &SkBitmap {
    <dyn Any>::downcast_ref(&*handle.0).expect("bitmap handle must be a tiny-skia SkBitmap")
}

// --- Conversions ------------------------------------------------------------

fn sk_color(color: &Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(color.r, color.g, color.b, color.a)
}

/// A ruffle matrix (twips translation) composed with an extra uniform scale,
/// as a tiny-skia transform. `scale` folds in the twips-to-pixel conversion.
fn sk_transform(matrix: &Matrix, scale: f32) -> SkTransform {
    SkTransform::from_row(
        scale * matrix.a,
        scale * matrix.b,
        scale * matrix.c,
        scale * matrix.d,
        scale * matrix.tx.get() as f32,
        scale * matrix.ty.get() as f32,
    )
}

fn sk_spread(spread: GradientSpread) -> SpreadMode {
    match spread {
        GradientSpread::Pad => SpreadMode::Pad,
        GradientSpread::Reflect => SpreadMode::Reflect,
        GradientSpread::Repeat => SpreadMode::Repeat,
    }
}

/// A colour with the render-time colour transform applied.
fn transform_color(color: &Color, ctx: &ColorTransform) -> tiny_skia::Color {
    sk_color(&(ctx * *color))
}

fn gradient_stops(gradient: &Gradient, ctx: &ColorTransform) -> Vec<GradientStop> {
    gradient
        .records
        .iter()
        .map(|record| {
            GradientStop::new(f32::from(record.ratio) / 255.0, transform_color(&record.color, ctx))
        })
        .collect()
}

/// `draw_rect` and `draw_line` supply a *unit* square or line, and
/// `Matrix::create_box` puts the size in the linear part **in pixels** while the
/// translation stays in twips. Only the translation may be converted here —
/// scaling the whole matrix (as shape paths, which are in twips, require)
/// shrinks these by 20x. Text fields with a `scrollRect` are masked this way,
/// so the bug hid a mask down to a few pixels and clipped the text away
/// entirely.
fn sk_transform_unit(matrix: &Matrix) -> SkTransform {
    SkTransform::from_row(
        matrix.a,
        matrix.b,
        matrix.c,
        matrix.d,
        matrix.tx.get() as f32 * TWIPS_TO_PIXELS,
        matrix.ty.get() as f32 * TWIPS_TO_PIXELS,
    )
}

/// A `swf::Matrix` (Fixed16 scale/skew, twips translation) as a tiny-skia
/// transform, kept in twips. A gradient's or bitmap fill's baked local matrix;
/// the twips-to-pixel scale is applied by the fill transform at paint time.
fn sk_transform_swf(matrix: &swf::Matrix) -> SkTransform {
    SkTransform::from_row(
        matrix.a.to_f32(),
        matrix.b.to_f32(),
        matrix.c.to_f32(),
        matrix.d.to_f32(),
        matrix.tx.get() as f32,
        matrix.ty.get() as f32,
    )
}

fn fill_shader(style: &FillStyle, ctx: &ColorTransform) -> Shader<'static> {
    match style {
        FillStyle::Color(color) => Shader::SolidColor(transform_color(color, ctx)),
        FillStyle::LinearGradient(gradient) => LinearGradient::new(
            Point::from_xy(-GRADIENT_HALF, 0.0),
            Point::from_xy(GRADIENT_HALF, 0.0),
            gradient_stops(gradient, ctx),
            sk_spread(gradient.spread),
            sk_transform_swf(&gradient.matrix),
        )
        .unwrap_or_else(|| solid_fallback(gradient, ctx)),
        // A focal gradient is the same circle with the first stop moved along
        // the gradient square's x axis; tiny-skia's two-point conical takes
        // that as its start point. Flash clamps the offset just short of the
        // edge, where the cone degenerates.
        FillStyle::RadialGradient(gradient) => focal_gradient(gradient, 0.0, ctx),
        FillStyle::FocalGradient {
            gradient,
            focal_point,
        } => focal_gradient(gradient, focal_point.to_f32(), ctx),
        // Handled by the caller, which owns the bitmap borrow; a shape that
        // reaches here referenced a bitmap that could not be resolved.
        FillStyle::Bitmap { .. } => {
            Shader::SolidColor(transform_color(&Color::from_rgb(0x808080, 255), ctx))
        }
    }
}

fn focal_gradient(gradient: &Gradient, focal_point: f32, ctx: &ColorTransform) -> Shader<'static> {
    RadialGradient::new(
        Point::from_xy(focal_point.clamp(-0.98, 0.98) * GRADIENT_HALF, 0.0),
        Point::from_xy(0.0, 0.0),
        GRADIENT_HALF,
        gradient_stops(gradient, ctx),
        sk_spread(gradient.spread),
        sk_transform_swf(&gradient.matrix),
    )
    .unwrap_or_else(|| solid_fallback(gradient, ctx))
}

/// A bitmap fill's paint, built inside the borrow of the bitmap it samples.
fn bitmap_paint<'a>(fill: &SkBitmapFill, pixmap: PixmapRef<'a>, ctx: &ColorTransform) -> Paint<'a> {
    Paint {
        shader: Pattern::new(
            pixmap,
            if fill.repeating {
                SpreadMode::Repeat
            } else {
                // Flash clamps a non-repeating bitmap fill at its edges.
                SpreadMode::Pad
            },
            if fill.smoothed {
                FilterQuality::Bilinear
            } else {
                FilterQuality::Nearest
            },
            // tiny-skia patterns carry an opacity but no colour transform, so
            // fades apply and RGB tinting does not.
            ctx.a_multiply.to_f32().clamp(0.0, 1.0),
            sk_transform_swf(&fill.matrix),
        ),
        anti_alias: true,
        ..Default::default()
    }
}

fn solid_fallback(gradient: &Gradient, ctx: &ColorTransform) -> Shader<'static> {
    let color = gradient
        .records
        .first()
        .map(|record| transform_color(&record.color, ctx))
        .unwrap_or(tiny_skia::Color::BLACK);
    Shader::SolidColor(color)
}

fn fill_paint(style: &FillStyle, ctx: &ColorTransform) -> Paint<'static> {
    let mut paint = Paint {
        shader: fill_shader(style, ctx),
        ..Default::default()
    };
    paint.anti_alias = true;
    paint
}

fn sk_fill_rule(rule: FillRule) -> SkFillRule {
    match rule {
        FillRule::EvenOdd => SkFillRule::EvenOdd,
        FillRule::NonZero => SkFillRule::Winding,
    }
}

fn build_path(commands: &[DrawCommand], is_closed: bool) -> Option<Path> {
    let mut builder = PathBuilder::new();
    for command in commands {
        match command {
            DrawCommand::MoveTo(point) => {
                builder.move_to(point.x.get() as f32, point.y.get() as f32);
            }
            DrawCommand::LineTo(point) => {
                builder.line_to(point.x.get() as f32, point.y.get() as f32);
            }
            DrawCommand::QuadraticCurveTo { control, anchor } => {
                builder.quad_to(
                    control.x.get() as f32,
                    control.y.get() as f32,
                    anchor.x.get() as f32,
                    anchor.y.get() as f32,
                );
            }
            DrawCommand::CubicCurveTo {
                control_a,
                control_b,
                anchor,
            } => {
                builder.cubic_to(
                    control_a.x.get() as f32,
                    control_a.y.get() as f32,
                    control_b.x.get() as f32,
                    control_b.y.get() as f32,
                    anchor.x.get() as f32,
                    anchor.y.get() as f32,
                );
            }
        }
    }
    if is_closed {
        builder.close();
    }
    builder.finish()
}

/// A unit box (0,0)-(1,1) for `draw_rect`.
fn unit_rect() -> Option<Path> {
    let mut builder = PathBuilder::new();
    builder.move_to(0.0, 0.0);
    builder.line_to(1.0, 0.0);
    builder.line_to(1.0, 1.0);
    builder.line_to(0.0, 1.0);
    builder.close();
    builder.finish()
}

// --- RenderBackend ----------------------------------------------------------

impl RenderBackend for TinySkiaRenderBackend {
    fn viewport_dimensions(&self) -> ViewportDimensions {
        self.dimensions
    }

    fn set_viewport_dimensions(&mut self, dimensions: ViewportDimensions) {
        if dimensions.width != self.dimensions.width || dimensions.height != self.dimensions.height
        {
            self.frame = new_pixmap(dimensions.width, dimensions.height);
        }
        self.dimensions = dimensions;
    }

    fn register_shape(
        &mut self,
        shape: DistilledShape,
        bitmap_source: &dyn BitmapSource,
    ) -> ShapeHandle {
        let mut draws = Vec::new();
        for path in &shape.paths {
            match path {
                DrawPath::Fill {
                    style,
                    commands,
                    winding_rule,
                } => {
                    if let Some(path) = build_path(commands, true) {
                        let style = (*style).clone();
                        let paint = self.shape_paint(&style, bitmap_source);
                        draws.push(SkDraw::Fill {
                            path,
                            style,
                            paint,
                            rule: sk_fill_rule(*winding_rule),
                        });
                    }
                }
                DrawPath::Stroke {
                    style,
                    is_closed,
                    commands,
                } => {
                    if let Some(path) = build_path(commands, *is_closed) {
                        let fill_style = style.fill_style().clone();
                        let paint = self.shape_paint(&fill_style, bitmap_source);
                        draws.push(SkDraw::Stroke {
                            path,
                            style: fill_style,
                            paint,
                            width: style.width().get() as f32,
                        });
                    }
                }
            }
        }
        ShapeHandle(Arc::new(SkShape(draws)))
    }

    fn render_offscreen(
        &mut self,
        _handle: BitmapHandle,
        _commands: CommandList,
        _quality: StageQuality,
        _bounds: PixelRegion,
    ) -> Option<Box<dyn SyncHandle>> {
        self.stats.offscreen += 1;
        None
    }

    fn submit_frame(
        &mut self,
        clear: Color,
        commands: CommandList,
        _cache_entries: Vec<BitmapCacheEntry>,
    ) {
        self.frame.fill(sk_color(&clear));
        self.masks.reset();
        self.stats = SpikeStats::default();
        commands.execute(self);
        if std::env::var_os("CHUMBY_TS_STATS").is_some() {
            log::warn!("tiny-skia frame stats: {:?}", self.stats);
        }
    }

    fn create_empty_texture(
        &mut self,
        width: NonZeroU32,
        height: NonZeroU32,
    ) -> Result<BitmapHandle, Error> {
        let pixmap = Pixmap::new(width.get(), height.get()).ok_or(Error::TooLarge)?;
        Ok(BitmapHandle(Arc::new(SkBitmap {
            pixmap: RefCell::new(pixmap),
        })))
    }

    fn register_bitmap(&mut self, bitmap: Bitmap<'_>) -> Result<BitmapHandle, Error> {
        let bitmap = bitmap.to_rgba();
        let size = IntSize::from_wh(bitmap.width(), bitmap.height()).ok_or(Error::TooLarge)?;
        // `BitmapFormat::Rgba` is premultiplied, which is what tiny-skia stores.
        let pixmap = Pixmap::from_vec(bitmap.data().to_vec(), size).ok_or(Error::TooLarge)?;
        Ok(BitmapHandle(Arc::new(SkBitmap {
            pixmap: RefCell::new(pixmap),
        })))
    }

    fn update_texture(
        &mut self,
        handle: &BitmapHandle,
        bitmap: Bitmap<'_>,
        region: PixelRegion,
    ) -> Result<(), Error> {
        let bitmap = bitmap.to_rgba();
        let sk = as_sk_bitmap(handle);
        let mut pixmap = sk.pixmap.borrow_mut();
        let dst_width = pixmap.width();
        let dst = pixmap.data_mut();
        let src = bitmap.data();
        let row_bytes = (region.width() * 4) as usize;
        for row in 0..region.height() {
            let src_off = row as usize * row_bytes;
            let dst_off = (((region.y_min + row) * dst_width + region.x_min) * 4) as usize;
            let (Some(src_row), Some(dst_row)) = (
                src.get(src_off..src_off + row_bytes),
                dst.get_mut(dst_off..dst_off + row_bytes),
            ) else {
                break;
            };
            dst_row.copy_from_slice(src_row);
        }
        Ok(())
    }

    fn create_context3d(
        &mut self,
        _profile: Context3DProfile,
    ) -> Result<Box<dyn Context3D>, Error> {
        Err(Error::Unimplemented("createContext3D".into()))
    }

    fn debug_info(&self) -> Cow<'static, str> {
        Cow::Borrowed("Renderer: tiny-skia (spike)")
    }

    fn name(&self) -> &'static str {
        "tiny-skia"
    }

    fn set_quality(&mut self, _quality: StageQuality) {}

    fn compile_pixelbender_shader(
        &mut self,
        _shader: PixelBenderShader,
    ) -> Result<PixelBenderShaderHandle, Error> {
        Err(Error::Unimplemented("Pixel bender shader compilation".into()))
    }

    fn run_pixelbender_shader(
        &mut self,
        _handle: PixelBenderShaderHandle,
        _arguments: &[PixelBenderShaderArgument],
        _target: &PixelBenderTarget,
    ) -> Result<PixelBenderOutput, Error> {
        Err(Error::Unimplemented("Pixel bender shader".into()))
    }

    fn resolve_sync_handle(
        &mut self,
        _handle: Box<dyn SyncHandle>,
        _with_rgba: RgbaBufRead,
    ) -> Result<(), Error> {
        Err(Error::Unimplemented("Sync handle resolution".into()))
    }
}

// --- CommandHandler ---------------------------------------------------------

impl CommandHandler for TinySkiaRenderBackend {
    fn render_bitmap(
        &mut self,
        bitmap: BitmapHandle,
        transform: ruffle_render::transform::Transform,
        smoothing: bool,
        _pixel_snapping: PixelSnapping,
    ) {
        self.stats.bitmaps += 1;
        if self.masks.defining() {
            // A bitmap as mask geometry: take its whole rectangle as coverage.
            let sk = as_sk_bitmap(&bitmap);
            let (w, h) = {
                let pixmap = sk.pixmap.borrow();
                (pixmap.width() as f32, pixmap.height() as f32)
            };
            let rect = tiny_skia::Rect::from_xywh(0.0, 0.0, w, h).map(PathBuilder::from_rect);
            if let Some(rect) = rect {
                self.masks.fill(
                    &rect,
                    SkFillRule::Winding,
                    sk_transform(&transform.matrix, TWIPS_TO_PIXELS),
                );
            }
            return;
        }
        if self.masks.clip_is_empty() {
            return;
        }
        let sk = as_sk_bitmap(&bitmap);
        let pixmap = sk.pixmap.borrow();
        // tiny-skia bitmaps only carry an opacity, not a full colour transform;
        // apply the alpha multiply (fades) and leave RGB tinting for later.
        let opacity = transform.color_transform.a_multiply.to_f32().clamp(0.0, 1.0);
        let paint = PixmapPaint {
            opacity,
            quality: if smoothing {
                FilterQuality::Bilinear
            } else {
                FilterQuality::Nearest
            },
            ..Default::default()
        };
        let transform = sk_transform(&transform.matrix, TWIPS_TO_PIXELS);
        let _ = self
            .frame
            .as_mut()
            .draw_pixmap(0, 0, pixmap.as_ref(), &paint, transform, self.masks.clip());
    }

    fn render_stage3d(
        &mut self,
        _bitmap: BitmapHandle,
        _transform: ruffle_render::transform::Transform,
    ) {
        // Unsupported (non-goal).
    }

    fn render_shape(&mut self, shape: ShapeHandle, transform: ruffle_render::transform::Transform) {
        self.stats.shapes += 1;
        let sk = as_sk_shape(&shape);
        let matrix = sk_transform(&transform.matrix, TWIPS_TO_PIXELS);
        if self.masks.defining() {
            // Only the shape's coverage matters for a mask, not its paint.
            for draw in &sk.0 {
                match draw {
                    SkDraw::Fill { path, rule, .. } => self.masks.fill(path, *rule, matrix),
                    SkDraw::Stroke { path, .. } => {
                        self.masks.fill(path, SkFillRule::Winding, matrix)
                    }
                }
            }
            return;
        }
        if self.masks.clip_is_empty() {
            return;
        }
        // Cached paints hold the untransformed colours; only rebuild when a
        // real colour transform (tint / alpha fade) is in play.
        let ctx = transform.color_transform;
        let identity = ctx == ColorTransform::IDENTITY;
        let clip = self.masks.clip();
        let stats = &mut self.stats;
        let mut pixmap = self.frame.as_mut();
        for draw in &sk.0 {
            match draw {
                SkDraw::Fill {
                    path,
                    style,
                    paint,
                    rule,
                } => match paint {
                    SkPaint::Solid(cached) => {
                        let rebuilt;
                        let paint = if identity {
                            cached.as_ref()
                        } else {
                            rebuilt = fill_paint(style, &ctx);
                            &rebuilt
                        };
                        let _ = pixmap.fill_path(path, paint, *rule, matrix, clip);
                    }
                    SkPaint::Bitmap(fill) => {
                        stats.bitmap_fills += 1;
                        let bitmap = as_sk_bitmap(&fill.handle).pixmap.borrow();
                        let paint = bitmap_paint(fill, bitmap.as_ref(), &ctx);
                        let _ = pixmap.fill_path(path, &paint, *rule, matrix, clip);
                    }
                },
                SkDraw::Stroke {
                    path,
                    style,
                    paint,
                    width,
                } => {
                    let stroke = Stroke {
                        width: *width,
                        ..Default::default()
                    };
                    match paint {
                        SkPaint::Solid(cached) => {
                            let rebuilt;
                            let paint = if identity {
                                cached.as_ref()
                            } else {
                                rebuilt = fill_paint(style, &ctx);
                                &rebuilt
                            };
                            let _ = pixmap.stroke_path(path, paint, &stroke, matrix, clip);
                        }
                        SkPaint::Bitmap(fill) => {
                            let bitmap = as_sk_bitmap(&fill.handle).pixmap.borrow();
                            let paint = bitmap_paint(fill, bitmap.as_ref(), &ctx);
                            let _ = pixmap.stroke_path(path, &paint, &stroke, matrix, clip);
                        }
                    }
                }
            }
        }
    }

    fn render_alpha_mask(&mut self, maskee_commands: CommandList, _mask_commands: CommandList) {
        self.stats.alpha_masks += 1;
        // Spike: no clipping — draw the maskee unclipped.
        maskee_commands.execute(self);
    }

    fn draw_rect(&mut self, color: Color, matrix: Matrix) {
        self.stats.rects += 1;
        let Some(path) = unit_rect() else { return };
        let transform = sk_transform_unit(&matrix);
        if self.masks.defining() {
            self.masks.fill(&path, SkFillRule::Winding, transform);
            return;
        }
        if self.masks.clip_is_empty() {
            return;
        }
        let paint = Paint {
            shader: Shader::SolidColor(sk_color(&color)),
            anti_alias: true,
            ..Default::default()
        };
        let _ = self.frame.as_mut().fill_path(
            &path,
            &paint,
            SkFillRule::Winding,
            transform,
            self.masks.clip(),
        );
    }

    fn draw_line(&mut self, color: Color, matrix: Matrix) {
        self.stats.lines += 1;
        if self.masks.defining() || self.masks.clip_is_empty() {
            return;
        }
        let mut builder = PathBuilder::new();
        builder.move_to(0.0, 0.0);
        builder.line_to(1.0, 0.0);
        let Some(path) = builder.finish() else { return };
        let paint = Paint {
            shader: Shader::SolidColor(sk_color(&color)),
            anti_alias: true,
            ..Default::default()
        };
        let transform = sk_transform_unit(&matrix);
        let _ = self.frame.as_mut().stroke_path(
            &path,
            &paint,
            &Stroke::default(),
            transform,
            self.masks.clip(),
        );
    }

    /// A border is 1 device pixel thick and must not be transformed
    /// (`edit_text.rs`: "always 1px regardless of zoom and transform"), but
    /// tiny-skia strokes in path space and transforms afterwards, and this
    /// matrix carries the box's *size* in its linear part. Stroking the unit
    /// square therefore scaled the border by the box dimensions — the alarm
    /// wizard's name field came out as a filled black band. `emulate_line_rect`
    /// builds the four 1px rects from already-transformed corners, as wgpu does
    /// when its adapter has no line primitive.
    fn draw_line_rect(&mut self, color: Color, matrix: Matrix) {
        self.stats.lines += 1;
        emulate_line_rect(self, color, matrix);
    }

    fn push_mask(&mut self) {
        self.stats.masks += 1;
        let (width, height) = (self.frame.width(), self.frame.height());
        self.masks.push(width, height);
    }

    fn activate_mask(&mut self) {
        self.masks.activate();
    }

    fn deactivate_mask(&mut self) {
        let (width, height) = (self.frame.width(), self.frame.height());
        self.masks.deactivate(width, height);
    }

    fn pop_mask(&mut self) {
        self.masks.pop();
    }

    fn blend(&mut self, commands: CommandList, _blend_mode: RenderBlendMode) {
        self.stats.blends += 1;
        // Spike: blend modes unsupported — draw the inner commands normally.
        commands.execute(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swf::{GradientSpread, Twips};

    /// `Matrix::create_box` (core's `create_box_from_rectangle`) carries the
    /// size in pixels and the position in twips. Scaling the whole matrix, as
    /// twips-space shape paths need, shrank every `draw_rect` 20x — which on
    /// the panel reduced a text field's `scrollRect` mask to a few pixels and
    /// clipped the text away.
    #[test]
    fn unit_geometry_keeps_pixel_scale() {
        let matrix = Matrix::create_box(
            177.0,
            132.0,
            Twips::from_pixels(24.0),
            Twips::from_pixels(-5.0),
        );
        let t = sk_transform_unit(&matrix);
        assert!(approx(t.sx, 177.0));
        assert!(approx(t.sy, 132.0));
        assert!(approx(t.tx, 24.0));
        assert!(approx(t.ty, -5.0));
    }

    /// A mask can be pushed and popped again without ever being activated;
    /// that must not disturb the clip of an enclosing mask.
    #[test]
    fn mask_popped_without_activation_keeps_outer_clip() {
        let mut stack = MaskStack::default();
        stack.push(4, 4);
        stack.activate();
        assert!(stack.clip().is_some());

        stack.push(4, 4);
        stack.pop();
        assert!(stack.clip().is_some(), "outer clip must survive");

        stack.pop();
        assert!(stack.clip().is_none());
    }

    /// While a mask is being defined, geometry defines it instead of being
    /// drawn; once activated, drawing resumes and is clipped.
    #[test]
    fn mask_states_follow_the_protocol() {
        let mut stack = MaskStack::default();
        assert!(!stack.defining());
        stack.push(4, 4);
        assert!(stack.defining());
        stack.activate();
        assert!(!stack.defining());
        stack.deactivate(4, 4);
        assert!(stack.defining(), "the replayed geometry is not content");
        stack.pop();
        assert!(!stack.defining());
    }

    fn rect_path(l: f32, t: f32, r: f32, b: f32) -> Path {
        PathBuilder::from_rect(tiny_skia::Rect::from_ltrb(l, t, r, b).unwrap())
    }

    /// A reused mask must be all zero again, whatever was drawn into it.
    #[test]
    fn reused_mask_is_clean() {
        let mut stack = MaskStack::default();
        stack.push(8, 8);
        stack.fill(&rect_path(2.0, 2.0, 6.0, 6.0), SkFillRule::Winding, SkTransform::identity());
        stack.activate();
        assert!(stack.clip().unwrap().data().iter().any(|&v| v > 0));
        stack.pop();
        stack.push(8, 8);
        stack.activate();
        assert!(stack.clip().unwrap().data().iter().all(|&v| v == 0));
        assert!(stack.clip_is_empty());
    }

    /// An inner mask keeps only what the outer clip lets through, and its
    /// box shrinks to the overlap.
    #[test]
    fn activate_intersects_within_the_outer_box() {
        let mut stack = MaskStack::default();
        stack.push(8, 8);
        stack.fill(&rect_path(0.0, 0.0, 4.0, 8.0), SkFillRule::Winding, SkTransform::identity());
        stack.activate();
        stack.push(8, 8);
        stack.fill(&rect_path(0.0, 0.0, 8.0, 8.0), SkFillRule::Winding, SkTransform::identity());
        stack.activate();
        let inner = stack.active().unwrap();
        let data = inner.mask.data();
        assert_eq!(data[8 + 1], 255, "inside both");
        assert_eq!(data[8 + 6], 0, "outside the outer clip");
        let d = inner.dirty.unwrap();
        assert!(d.right() <= 5 && d.left() == 0, "box shrank to the overlap: {d:?}");
        assert!(!stack.clip_is_empty());
    }

    /// Geometry with no area defines nothing and costs nothing.
    #[test]
    fn degenerate_geometry_leaves_the_mask_empty() {
        let mut stack = MaskStack::default();
        stack.push(8, 8);
        stack.fill(&rect_path(1.0, 1.0, 1.0, 5.0), SkFillRule::Winding, SkTransform::identity());
        stack.activate();
        assert!(stack.clip_is_empty());
    }

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn color_maps_channels() {
        let c = sk_color(&Color::from_rgb(0xFF0000, 255));
        assert!(approx(c.red(), 1.0));
        assert!(approx(c.green(), 0.0));
        assert!(approx(c.blue(), 0.0));
        assert!(approx(c.alpha(), 1.0));
    }

    #[test]
    fn fill_rule_maps() {
        assert!(matches!(sk_fill_rule(FillRule::EvenOdd), SkFillRule::EvenOdd));
        assert!(matches!(sk_fill_rule(FillRule::NonZero), SkFillRule::Winding));
    }

    #[test]
    fn spread_maps() {
        assert!(matches!(sk_spread(GradientSpread::Pad), SpreadMode::Pad));
        assert!(matches!(sk_spread(GradientSpread::Reflect), SpreadMode::Reflect));
        assert!(matches!(sk_spread(GradientSpread::Repeat), SpreadMode::Repeat));
    }

    #[test]
    fn transform_folds_scale_and_twips() {
        let mut m = Matrix::IDENTITY;
        m.tx = Twips::from_pixels(10.0); // 200 twips
        let t = sk_transform(&m, TWIPS_TO_PIXELS);
        assert!(approx(t.sx, 0.05));
        assert!(approx(t.sy, 0.05));
        // 0.05 * 200 twips = 10 device px.
        assert!(approx(t.tx, 10.0));
    }

    #[test]
    fn build_path_some_and_none() {
        let commands = vec![
            DrawCommand::MoveTo(swf::Point::new(Twips::new(0), Twips::new(0))),
            DrawCommand::LineTo(swf::Point::new(Twips::new(100), Twips::new(0))),
            DrawCommand::LineTo(swf::Point::new(Twips::new(100), Twips::new(100))),
        ];
        assert!(build_path(&commands, true).is_some());
        assert!(build_path(&[], true).is_none());
    }
}
