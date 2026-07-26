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
use ruffle_render::matrix::Matrix;
use ruffle_render::pixel_bender::{PixelBenderShader, PixelBenderShaderHandle};
use ruffle_render::pixel_bender_support::PixelBenderShaderArgument;
use ruffle_render::quality::StageQuality;
use ruffle_render::shape_utils::{DistilledShape, DrawCommand, DrawPath, FillRule};
use swf::{Color, FillStyle, Gradient, GradientSpread, LineStyle};
use tiny_skia::{
    FillRule as SkFillRule, FilterQuality, GradientStop, IntSize, LinearGradient, Paint, Path,
    PathBuilder, Pixmap, PixmapPaint, Point, RadialGradient, Shader, SpreadMode, Stroke,
    Transform as SkTransform,
};

/// Twips-to-pixel scale. Paths and matrices are kept in twips (as `canvas` does)
/// and scaled to device pixels at paint time.
const TWIPS_TO_PIXELS: f32 = 0.05;

/// The gradient square SWF gradients are defined in: x/y span ±16384 twips.
const GRADIENT_HALF: f32 = 16384.0;

pub struct TinySkiaRenderBackend {
    frame: Pixmap,
    dimensions: ViewportDimensions,
}

impl TinySkiaRenderBackend {
    pub fn new(width: u32, height: u32) -> Self {
        let frame = new_pixmap(width, height);
        Self {
            frame,
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
}

fn new_pixmap(width: u32, height: u32) -> Pixmap {
    Pixmap::new(width.max(1), height.max(1)).expect("non-zero pixmap dimensions")
}

// --- Handle types -----------------------------------------------------------

enum SkDraw {
    Fill {
        path: Path,
        paint: Paint<'static>,
        rule: SkFillRule,
    },
    Stroke {
        path: Path,
        paint: Paint<'static>,
        width: f32,
    },
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

fn gradient_stops(gradient: &Gradient) -> Vec<GradientStop> {
    gradient
        .records
        .iter()
        .map(|record| GradientStop::new(f32::from(record.ratio) / 255.0, sk_color(&record.color)))
        .collect()
}

/// A `swf::Matrix` (Fixed16 scale/skew, twips translation) as a tiny-skia
/// transform, kept in twips. Used as a gradient's baked local matrix; the
/// twips-to-pixel scale is applied by the fill transform at paint time.
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

fn fill_shader(style: &FillStyle) -> Shader<'static> {
    match style {
        FillStyle::Color(color) => Shader::SolidColor(sk_color(color)),
        FillStyle::LinearGradient(gradient) => LinearGradient::new(
            Point::from_xy(-GRADIENT_HALF, 0.0),
            Point::from_xy(GRADIENT_HALF, 0.0),
            gradient_stops(gradient),
            sk_spread(gradient.spread),
            sk_transform_swf(&gradient.matrix),
        )
        .unwrap_or_else(|| solid_fallback(gradient)),
        FillStyle::RadialGradient(gradient) | FillStyle::FocalGradient { gradient, .. } => {
            // Focal offset is ignored for the spike — rendered as a plain radial.
            RadialGradient::new(
                Point::from_xy(0.0, 0.0),
                Point::from_xy(0.0, 0.0),
                GRADIENT_HALF,
                gradient_stops(gradient),
                sk_spread(gradient.spread),
                sk_transform_swf(&gradient.matrix),
            )
            .unwrap_or_else(|| solid_fallback(gradient))
        }
        // Spike: bitmap-filled vector shapes render flat. Photographic content
        // arrives via `render_bitmap`, which is implemented.
        FillStyle::Bitmap { .. } => Shader::SolidColor(tiny_skia::Color::from_rgba8(128, 128, 128, 255)),
    }
}

fn solid_fallback(gradient: &Gradient) -> Shader<'static> {
    let color = gradient
        .records
        .first()
        .map(|record| sk_color(&record.color))
        .unwrap_or(tiny_skia::Color::BLACK);
    Shader::SolidColor(color)
}

fn fill_paint(style: &FillStyle) -> Paint<'static> {
    let mut paint = Paint {
        shader: fill_shader(style),
        ..Default::default()
    };
    paint.anti_alias = true;
    paint
}

fn stroke_paint(style: &LineStyle) -> Paint<'static> {
    let mut paint = Paint {
        shader: fill_shader(style.fill_style()),
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

/// A unit box (0,0)-(1,1) for `draw_rect` / `draw_line_rect`.
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
        _bitmap_source: &dyn BitmapSource,
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
                        draws.push(SkDraw::Fill {
                            path,
                            paint: fill_paint(style),
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
                        draws.push(SkDraw::Stroke {
                            path,
                            paint: stroke_paint(style),
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
        None
    }

    fn submit_frame(
        &mut self,
        clear: Color,
        commands: CommandList,
        _cache_entries: Vec<BitmapCacheEntry>,
    ) {
        self.frame.fill(sk_color(&clear));
        commands.execute(self);
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
        let sk = as_sk_bitmap(&bitmap);
        let pixmap = sk.pixmap.borrow();
        let paint = PixmapPaint {
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
            .draw_pixmap(0, 0, pixmap.as_ref(), &paint, transform, None);
    }

    fn render_stage3d(
        &mut self,
        _bitmap: BitmapHandle,
        _transform: ruffle_render::transform::Transform,
    ) {
        // Unsupported (non-goal).
    }

    fn render_shape(&mut self, shape: ShapeHandle, transform: ruffle_render::transform::Transform) {
        let sk = as_sk_shape(&shape);
        let matrix = sk_transform(&transform.matrix, TWIPS_TO_PIXELS);
        let mut pixmap = self.frame.as_mut();
        for draw in &sk.0 {
            match draw {
                SkDraw::Fill { path, paint, rule } => {
                    let _ = pixmap.fill_path(path, paint, *rule, matrix, None);
                }
                SkDraw::Stroke { path, paint, width } => {
                    let stroke = Stroke {
                        width: *width,
                        ..Default::default()
                    };
                    let _ = pixmap.stroke_path(path, paint, &stroke, matrix, None);
                }
            }
        }
    }

    fn render_alpha_mask(&mut self, maskee_commands: CommandList, _mask_commands: CommandList) {
        // Spike: no clipping — draw the maskee unclipped.
        maskee_commands.execute(self);
    }

    fn draw_rect(&mut self, color: Color, matrix: Matrix) {
        let Some(path) = unit_rect() else { return };
        let paint = Paint {
            shader: Shader::SolidColor(sk_color(&color)),
            anti_alias: true,
            ..Default::default()
        };
        let transform = sk_transform(&matrix, TWIPS_TO_PIXELS);
        let _ = self
            .frame
            .as_mut()
            .fill_path(&path, &paint, SkFillRule::Winding, transform, None);
    }

    fn draw_line(&mut self, color: Color, matrix: Matrix) {
        let mut builder = PathBuilder::new();
        builder.move_to(0.0, 0.0);
        builder.line_to(1.0, 0.0);
        let Some(path) = builder.finish() else { return };
        let paint = Paint {
            shader: Shader::SolidColor(sk_color(&color)),
            anti_alias: true,
            ..Default::default()
        };
        let transform = sk_transform(&matrix, TWIPS_TO_PIXELS);
        let _ = self
            .frame
            .as_mut()
            .stroke_path(&path, &paint, &Stroke::default(), transform, None);
    }

    fn draw_line_rect(&mut self, color: Color, matrix: Matrix) {
        let Some(path) = unit_rect() else { return };
        let paint = Paint {
            shader: Shader::SolidColor(sk_color(&color)),
            anti_alias: true,
            ..Default::default()
        };
        let transform = sk_transform(&matrix, TWIPS_TO_PIXELS);
        let _ = self
            .frame
            .as_mut()
            .stroke_path(&path, &paint, &Stroke::default(), transform, None);
    }

    fn push_mask(&mut self) {}
    fn activate_mask(&mut self) {}
    fn deactivate_mask(&mut self) {}
    fn pop_mask(&mut self) {}

    fn blend(&mut self, commands: CommandList, _blend_mode: RenderBlendMode) {
        // Spike: blend modes unsupported — draw the inner commands normally.
        commands.execute(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swf::{GradientSpread, Twips};

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
