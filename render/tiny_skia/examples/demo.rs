//! Small visual smoke-test for the tiny-skia backend: renders a few solid
//! shapes and a linear gradient to a PNG. Not part of the shipped backend.
//!
//! Run: `cargo run -p ruffle_render_tiny_skia --example demo -- out.png`

use ruffle_render::backend::RenderBackend;
use ruffle_render::backend::null::NullBitmapSource;
use ruffle_render::commands::{CommandHandler, CommandList};
use ruffle_render::matrix::Matrix as RenderMatrix;
use ruffle_render::shape_utils::{DistilledShape, DrawCommand, DrawPath, FillRule};
use ruffle_render::transform::Transform;
use ruffle_render_tiny_skia::TinySkiaRenderBackend;
use swf::{
    Color, Fixed16, FillStyle, Gradient, GradientInterpolation, GradientRecord, GradientSpread,
    Matrix as SwfMatrix, Point, Rectangle, Twips,
};

const WIDTH: u32 = 400;
const HEIGHT: u32 = 260;

fn pt(x: f64, y: f64) -> Point<Twips> {
    Point::new(Twips::from_pixels(x), Twips::from_pixels(y))
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Vec<DrawCommand> {
    vec![
        DrawCommand::MoveTo(pt(x, y)),
        DrawCommand::LineTo(pt(x + w, y)),
        DrawCommand::LineTo(pt(x + w, y + h)),
        DrawCommand::LineTo(pt(x, y + h)),
    ]
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "demo.png".into());

    // A horizontal cyan→magenta gradient spanning a bar at x=30..370 (twips).
    let bar_x = 30.0;
    let bar_w = 340.0;
    let gradient = FillStyle::LinearGradient(Gradient {
        matrix: SwfMatrix {
            a: Fixed16::from_f32((bar_w * 20.0 / 32768.0) as f32),
            b: Fixed16::from_f32(0.0),
            c: Fixed16::from_f32(0.0),
            d: Fixed16::from_f32(1.0),
            tx: Twips::from_pixels(bar_x + bar_w / 2.0),
            ty: Twips::from_pixels(180.0),
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
    let blue = FillStyle::Color(Color::from_rgb(0x3366FF, 255));
    let orange = FillStyle::Color(Color::from_rgb(0xFF8800, 255));

    let paths = vec![
        DrawPath::Fill {
            style: &blue,
            commands: rect(30.0, 30.0, 120.0, 80.0),
            winding_rule: FillRule::NonZero,
        },
        DrawPath::Fill {
            style: &orange,
            commands: vec![
                DrawCommand::MoveTo(pt(200.0, 30.0)),
                DrawCommand::LineTo(pt(320.0, 30.0)),
                DrawCommand::LineTo(pt(260.0, 110.0)),
            ],
            winding_rule: FillRule::NonZero,
        },
        DrawPath::Fill {
            style: &gradient,
            commands: rect(bar_x, 140.0, bar_w, 80.0),
            winding_rule: FillRule::NonZero,
        },
    ];

    let shape = DistilledShape {
        paths,
        shape_bounds: Rectangle {
            x_min: Twips::from_pixels(0.0),
            x_max: Twips::from_pixels(WIDTH as f64),
            y_min: Twips::from_pixels(0.0),
            y_max: Twips::from_pixels(HEIGHT as f64),
        },
        edge_bounds: Rectangle {
            x_min: Twips::from_pixels(0.0),
            x_max: Twips::from_pixels(WIDTH as f64),
            y_min: Twips::from_pixels(0.0),
            y_max: Twips::from_pixels(HEIGHT as f64),
        },
        id: 1,
    };

    let mut backend = TinySkiaRenderBackend::new(WIDTH, HEIGHT);
    let handle = backend.register_shape(shape, &NullBitmapSource);

    let mut commands = CommandList::new();
    commands.render_shape(
        handle,
        Transform {
            matrix: RenderMatrix::IDENTITY,
            ..Default::default()
        },
    );

    backend.submit_frame(Color::from_rgb(0x202830, 255), commands, Vec::new());

    backend
        .frame()
        .save_png(&out)
        .expect("save_png should succeed");
    println!("wrote {out}");
}
