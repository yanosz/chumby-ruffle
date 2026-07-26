//! Step 2b harness: render an SWF through the tiny-skia backend to PNGs, for
//! eyeballing against the wgpu reference. Bypasses the wgpu-based `Exporter`
//! (no GPU adapter needed) and reads pixels straight from the backend's frame.
//!
//! Usage: `tiny_skia_export <in.swf> <out_dir> [frames] [scale]`

use std::any::Any;
use std::path::Path;

use ruffle_core::PlayerBuilder;
use ruffle_core::limits::ExecutionLimit;
use ruffle_core::tag_utils::movie_from_path;
use ruffle_render_tiny_skia::TinySkiaRenderBackend;

fn main() {
    let mut args = std::env::args().skip(1);
    let swf_path = args.next().expect("usage: tiny_skia_export <in.swf> <out_dir> [frames] [scale]");
    let out_dir = args.next().unwrap_or_else(|| ".".into());
    let frames: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let scale: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1.0);

    let movie = movie_from_path(Path::new(&swf_path), None).expect("load movie");
    let width = ((movie.width().to_pixels()) * scale).round() as u32;
    let height = ((movie.height().to_pixels()) * scale).round() as u32;
    println!("{swf_path}: {width}x{height}, {frames} frame(s)");

    let backend = TinySkiaRenderBackend::new(width, height);
    let player = PlayerBuilder::new()
        .with_renderer(backend)
        .with_movie(movie)
        .with_viewport_dimensions(width, height, scale)
        .with_autoplay(true)
        .build();

    std::fs::create_dir_all(&out_dir).expect("create out_dir");

    for i in 0..frames {
        let mut p = player.lock().unwrap();
        p.preload(&mut ExecutionLimit::none());
        p.run_frame();
        p.render();
        let renderer = <dyn Any>::downcast_mut::<TinySkiaRenderBackend>(p.renderer_mut())
            .expect("renderer must be tiny-skia");
        let path = format!("{out_dir}/frame_{i:04}.png");
        renderer.frame().save_png(&path).expect("save_png");
    }
    println!("wrote {frames} frame(s) to {out_dir}/");
}
