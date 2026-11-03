//! Integration tests: drive the backend through its public API and assert on
//! the rendered pixels. Colors are opaque so premultiplied == straight and the
//! raw `data()` bytes can be compared directly.

use ruffle_render::backend::null::NullBitmapSource;
use ruffle_render::backend::{Context3DProfile, RenderBackend, ViewportDimensions};
use ruffle_render::bitmap::{
    Bitmap, BitmapFormat, BitmapHandle, BitmapSize, BitmapSource, PixelRegion, PixelSnapping,
};
use ruffle_render::commands::{CommandHandler, CommandList};
use ruffle_render::matrix::Matrix;
use ruffle_render::shape_utils::{DistilledShape, DrawCommand, DrawPath, FillRule};
use ruffle_render::transform::Transform;
use ruffle_render_tiny_skia::TinySkiaRenderBackend;
use swf::{
    Color, ColorTransform, Fixed8, Fixed16, FillStyle, Gradient, GradientInterpolation,
    GradientRecord, GradientSpread, Matrix as SwfMatrix, Point, Rectangle, Twips,
};
use tiny_skia::Pixmap;

fn twip_pt(x: f64, y: f64) -> Point<Twips> {
    Point::new(Twips::from_pixels(x), Twips::from_pixels(y))
}

fn rect_cmds(x: f64, y: f64, w: f64, h: f64) -> Vec<DrawCommand> {
    vec![
        DrawCommand::MoveTo(twip_pt(x, y)),
        DrawCommand::LineTo(twip_pt(x + w, y)),
        DrawCommand::LineTo(twip_pt(x + w, y + h)),
        DrawCommand::LineTo(twip_pt(x, y + h)),
    ]
}

fn stage_bounds(w: f64, h: f64) -> Rectangle<Twips> {
    Rectangle {
        x_min: Twips::from_pixels(0.0),
        x_max: Twips::from_pixels(w),
        y_min: Twips::from_pixels(0.0),
        y_max: Twips::from_pixels(h),
    }
}

fn identity() -> Transform {
    Transform {
        matrix: Matrix::IDENTITY,
        ..Default::default()
    }
}

fn translate(x: f64, y: f64) -> Transform {
    let mut m = Matrix::IDENTITY;
    m.tx = Twips::from_pixels(x);
    m.ty = Twips::from_pixels(y);
    Transform {
        matrix: m,
        ..Default::default()
    }
}

/// A uniform scale, expressed in the pre-twips-scale space the backend paints
/// in: `a = pixels_per_bitmap_pixel / TWIPS_TO_PIXELS = factor * 20`.
fn pixel_scale(factor: f32) -> Transform {
    let mut m = Matrix::IDENTITY;
    m.a = factor * 20.0;
    m.d = factor * 20.0;
    Transform {
        matrix: m,
        ..Default::default()
    }
}

/// Raw RGBA of one pixel.
fn px(pixmap: &Pixmap, x: u32, y: u32) -> [u8; 4] {
    let data = pixmap.data();
    let i = ((y * pixmap.width() + x) * 4) as usize;
    [data[i], data[i + 1], data[i + 2], data[i + 3]]
}

#[test]
fn clear_fills_frame() {
    let mut backend = TinySkiaRenderBackend::new(8, 8);
    backend.submit_frame(Color::from_rgb(0x102030, 255), CommandList::new(), Vec::new());
    assert_eq!(px(backend.frame(), 4, 4), [0x10, 0x20, 0x30, 0xFF]);
}

#[test]
fn solid_fill_covers_its_region_only() {
    let red = FillStyle::Color(Color::from_rgb(0xFF0000, 255));
    let shape = DistilledShape {
        paths: vec![DrawPath::Fill {
            style: &red,
            commands: rect_cmds(2.0, 2.0, 4.0, 4.0),
            winding_rule: FillRule::NonZero,
        }],
        shape_bounds: stage_bounds(8.0, 8.0),
        edge_bounds: stage_bounds(8.0, 8.0),
        id: 1,
    };

    let mut backend = TinySkiaRenderBackend::new(8, 8);
    let handle = backend.register_shape(shape, &NullBitmapSource);
    let mut commands = CommandList::new();
    commands.render_shape(handle, identity());
    backend.submit_frame(Color::WHITE, commands, Vec::new());

    let frame = backend.frame();
    let inside = px(frame, 4, 4);
    assert!(inside[0] > 200 && inside[1] < 60 && inside[2] < 60, "inside={inside:?}");
    assert_eq!(px(frame, 0, 0), [255, 255, 255, 255], "outside should stay clear");
}

#[test]
fn transform_positions_the_shape() {
    let red = FillStyle::Color(Color::from_rgb(0xFF0000, 255));
    let shape = DistilledShape {
        paths: vec![DrawPath::Fill {
            style: &red,
            commands: rect_cmds(0.0, 0.0, 4.0, 4.0),
            winding_rule: FillRule::NonZero,
        }],
        shape_bounds: stage_bounds(8.0, 8.0),
        edge_bounds: stage_bounds(8.0, 8.0),
        id: 1,
    };

    let mut backend = TinySkiaRenderBackend::new(8, 8);
    let handle = backend.register_shape(shape, &NullBitmapSource);
    let mut commands = CommandList::new();
    commands.render_shape(handle, translate(4.0, 4.0));
    backend.submit_frame(Color::WHITE, commands, Vec::new());

    let frame = backend.frame();
    // Shape drawn at 0..4 but translated by 4 → occupies 4..8.
    let moved = px(frame, 6, 6);
    assert!(moved[0] > 200 && moved[1] < 60, "moved={moved:?}");
    assert_eq!(px(frame, 1, 1), [255, 255, 255, 255], "origin should be empty");
}

#[test]
fn linear_gradient_runs_cyan_to_magenta() {
    let w = 100.0;
    let gradient = FillStyle::LinearGradient(Gradient {
        matrix: SwfMatrix {
            a: Fixed16::from_f32((w * 20.0 / 32768.0) as f32),
            b: Fixed16::from_f32(0.0),
            c: Fixed16::from_f32(0.0),
            d: Fixed16::from_f32(1.0),
            tx: Twips::from_pixels(w / 2.0),
            ty: Twips::from_pixels(4.0),
        },
        spread: GradientSpread::Pad,
        interpolation: GradientInterpolation::Rgb,
        records: vec![
            GradientRecord {
                ratio: 0,
                color: Color::from_rgb(0x00FFFF, 255),
            },
            GradientRecord {
                ratio: 255,
                color: Color::from_rgb(0xFF00FF, 255),
            },
        ],
    });
    let shape = DistilledShape {
        paths: vec![DrawPath::Fill {
            style: &gradient,
            commands: rect_cmds(0.0, 0.0, w, 8.0),
            winding_rule: FillRule::NonZero,
        }],
        shape_bounds: stage_bounds(w, 8.0),
        edge_bounds: stage_bounds(w, 8.0),
        id: 1,
    };

    let mut backend = TinySkiaRenderBackend::new(100, 8);
    let handle = backend.register_shape(shape, &NullBitmapSource);
    let mut commands = CommandList::new();
    commands.render_shape(handle, identity());
    backend.submit_frame(Color::BLACK, commands, Vec::new());

    let frame = backend.frame();
    let left = px(frame, 3, 4); // near cyan (0,255,255)
    let right = px(frame, 96, 4); // near magenta (255,0,255)
    assert!(left[0] < 90 && left[1] > 170, "left={left:?}");
    assert!(right[0] > 170 && right[1] < 90, "right={right:?}");
}

#[test]
fn bitmap_renders_by_quadrant() {
    // 2x2: red, green / blue, white.
    #[rustfmt::skip]
    let data = vec![
        255, 0, 0, 255,   0, 255, 0, 255,
        0, 0, 255, 255,   255, 255, 255, 255,
    ];
    let bitmap = Bitmap::new(2, 2, BitmapFormat::Rgba, data);

    let mut backend = TinySkiaRenderBackend::new(20, 20);
    let handle = backend.register_bitmap(bitmap).expect("register_bitmap");
    let mut commands = CommandList::new();
    // 2 bitmap px → 20 device px, so factor 10; nearest sampling.
    commands.render_bitmap(handle, pixel_scale(10.0), false, PixelSnapping::Never);
    backend.submit_frame(Color::BLACK, commands, Vec::new());

    let frame = backend.frame();
    let red = px(frame, 5, 5);
    let green = px(frame, 15, 5);
    let blue = px(frame, 5, 15);
    let white = px(frame, 15, 15);
    assert!(red[0] > 200 && red[1] < 60 && red[2] < 60, "red={red:?}");
    assert!(green[1] > 200 && green[0] < 60 && green[2] < 60, "green={green:?}");
    assert!(blue[2] > 200 && blue[0] < 60 && blue[1] < 60, "blue={blue:?}");
    assert!(white[0] > 200 && white[1] > 200 && white[2] > 200, "white={white:?}");
}

#[test]
fn update_texture_replaces_pixels() {
    let red = Bitmap::new(2, 2, BitmapFormat::Rgba, vec![255, 0, 0, 255].repeat(4));
    let mut backend = TinySkiaRenderBackend::new(20, 20);
    let handle = backend.register_bitmap(red).expect("register_bitmap");

    let green = Bitmap::new(2, 2, BitmapFormat::Rgba, vec![0, 255, 0, 255].repeat(4));
    backend
        .update_texture(
            &handle,
            green,
            PixelRegion {
                x_min: 0,
                y_min: 0,
                x_max: 2,
                y_max: 2,
            },
        )
        .expect("update_texture");

    let mut commands = CommandList::new();
    commands.render_bitmap(handle, pixel_scale(10.0), false, PixelSnapping::Never);
    backend.submit_frame(Color::BLACK, commands, Vec::new());

    let center = px(backend.frame(), 10, 10);
    assert!(center[1] > 200 && center[0] < 60, "center={center:?}");
}

#[test]
fn color_transform_alpha_blends() {
    // A green fill at 50% alpha over a red clear should blend to a mid tone,
    // proving the render-time colour transform reaches the shader.
    let green = FillStyle::Color(Color::from_rgb(0x00FF00, 255));
    let shape = DistilledShape {
        paths: vec![DrawPath::Fill {
            style: &green,
            commands: rect_cmds(0.0, 0.0, 8.0, 8.0),
            winding_rule: FillRule::NonZero,
        }],
        shape_bounds: stage_bounds(8.0, 8.0),
        edge_bounds: stage_bounds(8.0, 8.0),
        id: 1,
    };

    let mut backend = TinySkiaRenderBackend::new(8, 8);
    let handle = backend.register_shape(shape, &NullBitmapSource);
    let mut commands = CommandList::new();
    commands.render_shape(
        handle,
        Transform {
            matrix: Matrix::IDENTITY,
            color_transform: ColorTransform {
                a_multiply: Fixed8::from_f32(0.5),
                ..ColorTransform::IDENTITY
            },
            ..Default::default()
        },
    );
    backend.submit_frame(Color::from_rgb(0xFF0000, 255), commands, Vec::new());

    let c = px(backend.frame(), 4, 4);
    assert!(c[0] > 90 && c[0] < 180, "red channel not blended: {c:?}");
    assert!(c[1] > 90 && c[1] < 180, "green channel not blended: {c:?}");
    assert!(c[2] < 40, "blue should stay low: {c:?}");
}

#[test]
fn metadata_and_stubs() {
    let mut backend = TinySkiaRenderBackend::new(4, 4);
    assert_eq!(backend.name(), "tiny-skia");
    assert!(!backend.is_offscreen_supported());

    backend.set_viewport_dimensions(ViewportDimensions {
        width: 32,
        height: 24,
        scale_factor: 1.0,
    });
    assert_eq!(backend.viewport_dimensions().width, 32);
    assert_eq!(backend.frame().width(), 32, "frame should reallocate");
    assert_eq!(backend.frame().height(), 24);

    assert!(backend.create_context3d(Context3DProfile::Baseline).is_err());
}

/// A `BitmapSource` that answers every id with one already-registered bitmap,
/// which is all a bitmap *fill* needs to be resolved.
struct OneBitmap(BitmapHandle);

impl BitmapSource for OneBitmap {
    fn bitmap_size(&self, _id: u16) -> Option<BitmapSize> {
        Some(BitmapSize {
            width: 2,
            height: 2,
        })
    }

    fn bitmap_handle(&self, _id: u16, _renderer: &mut dyn RenderBackend) -> Option<BitmapHandle> {
        Some(self.0.clone())
    }
}

/// A bitmap-filled vector shape samples the bitmap. The fill matrix maps the
/// bitmap's pixel grid into twips, so one bitmap pixel spans `20 * factor`.
#[test]
fn bitmap_fill_samples_the_bitmap() {
    #[rustfmt::skip]
    let data = vec![
        255, 0, 0, 255,   0, 255, 0, 255,
        0, 0, 255, 255,   255, 255, 255, 255,
    ];
    let mut backend = TinySkiaRenderBackend::new(20, 20);
    let handle = backend
        .register_bitmap(Bitmap::new(2, 2, BitmapFormat::Rgba, data))
        .expect("register_bitmap");

    let fill = FillStyle::Bitmap {
        id: 1,
        matrix: SwfMatrix {
            a: Fixed16::from_f32(200.0),
            b: Fixed16::from_f32(0.0),
            c: Fixed16::from_f32(0.0),
            d: Fixed16::from_f32(200.0),
            tx: Twips::ZERO,
            ty: Twips::ZERO,
        },
        is_smoothed: false,
        is_repeating: false,
    };
    let shape = DistilledShape {
        paths: vec![DrawPath::Fill {
            style: &fill,
            commands: rect_cmds(0.0, 0.0, 20.0, 20.0),
            winding_rule: FillRule::NonZero,
        }],
        shape_bounds: stage_bounds(20.0, 20.0),
        edge_bounds: stage_bounds(20.0, 20.0),
        id: 1,
    };

    let shape_handle = backend.register_shape(shape, &OneBitmap(handle));
    let mut commands = CommandList::new();
    commands.render_shape(shape_handle, identity());
    backend.submit_frame(Color::BLACK, commands, Vec::new());

    let frame = backend.frame();
    let red = px(frame, 5, 5);
    let green = px(frame, 15, 5);
    let blue = px(frame, 5, 15);
    assert!(red[0] > 200 && red[1] < 60, "red={red:?}");
    assert!(green[1] > 200 && green[0] < 60, "green={green:?}");
    assert!(blue[2] > 200 && blue[0] < 60, "blue={blue:?}");
}

/// A focal gradient is a radial one whose bright centre has slid along the
/// gradient square's x axis.
#[test]
fn focal_offset_moves_the_gradient_centre() {
    let w = 100.0;
    let gradient = Gradient {
        matrix: SwfMatrix {
            a: Fixed16::from_f32((w * 20.0 / 32768.0) as f32),
            b: Fixed16::from_f32(0.0),
            c: Fixed16::from_f32(0.0),
            d: Fixed16::from_f32((w * 20.0 / 32768.0) as f32),
            tx: Twips::from_pixels(w / 2.0),
            ty: Twips::from_pixels(w / 2.0),
        },
        spread: GradientSpread::Pad,
        interpolation: GradientInterpolation::Rgb,
        records: vec![
            GradientRecord {
                ratio: 0,
                color: Color::WHITE,
            },
            GradientRecord {
                ratio: 255,
                color: Color::BLACK,
            },
        ],
    };

    let brightest_column = |style: FillStyle| {
        let shape = DistilledShape {
            paths: vec![DrawPath::Fill {
                style: &style,
                commands: rect_cmds(0.0, 0.0, w, w),
                winding_rule: FillRule::NonZero,
            }],
            shape_bounds: stage_bounds(w, w),
            edge_bounds: stage_bounds(w, w),
            id: 1,
        };
        let mut backend = TinySkiaRenderBackend::new(w as u32, w as u32);
        let handle = backend.register_shape(shape, &NullBitmapSource);
        let mut commands = CommandList::new();
        commands.render_shape(handle, identity());
        backend.submit_frame(Color::BLACK, commands, Vec::new());
        let frame = backend.frame();
        (0..w as u32)
            .max_by_key(|x| px(frame, *x, w as u32 / 2)[0])
            .expect("non-empty row")
    };

    let plain = brightest_column(FillStyle::RadialGradient(gradient.clone()));
    let focal = brightest_column(FillStyle::FocalGradient {
        gradient,
        focal_point: Fixed8::from_f32(0.8),
    });
    assert!(
        focal > plain + 10,
        "focal centre should sit right of the plain one: focal={focal} plain={plain}"
    );
}

/// An `EditText` border (`edit_text.rs` `draw_text_box`) arrives as a unit
/// square whose matrix carries the box size. Stroking that path scales the
/// 1px border by the box dimensions, which filled the alarm wizard's name
/// field with solid black.
#[test]
fn line_rect_border_stays_one_pixel_thick() {
    let mut backend = TinySkiaRenderBackend::new(64, 32);
    let mut commands = CommandList::new();
    commands.draw_line_rect(
        Color::BLACK,
        Matrix::create_box(48.0, 12.0, Twips::from_pixels(4.0), Twips::from_pixels(8.0)),
    );
    backend.submit_frame(Color::WHITE, commands, Vec::new());

    let frame = backend.frame();
    assert_eq!(px(frame, 30, 8), [0, 0, 0, 255], "top edge should be drawn");
    assert_eq!(px(frame, 30, 20), [0, 0, 0, 255], "bottom edge should be drawn");
    assert_eq!(px(frame, 4, 14), [0, 0, 0, 255], "left edge should be drawn");
    assert_eq!(px(frame, 30, 14), [255, 255, 255, 255], "interior must stay clear");
    assert_eq!(px(frame, 5, 14), [255, 255, 255, 255], "interior must stay clear");
    assert_eq!(px(frame, 30, 6), [255, 255, 255, 255], "above the box must stay clear");
    assert_eq!(px(frame, 60, 14), [255, 255, 255, 255], "right of the box must stay clear");
}
