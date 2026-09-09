//! Step 2b harness: render an SWF through the tiny-skia backend to PNGs, for
//! eyeballing against the wgpu reference. Bypasses the wgpu-based `Exporter`
//! (no GPU adapter needed) and reads pixels straight from the backend's frame.
//!
//! Usage: `tiny_skia_export <in.swf> <out_dir> [frames] [scale]`

use std::any::Any;
use std::path::Path;
use std::time::Instant;

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

    // `out_dir` of "-" times rendering only (no PNGs), for the timing gate.
    let save = out_dir != "-";
    if save {
        std::fs::create_dir_all(&out_dir).expect("create out_dir");
    }

    let mut render_ms: Vec<f64> = Vec::with_capacity(frames as usize);
    let mut frame_ms: Vec<f64> = Vec::with_capacity(frames as usize);
    for i in 0..frames {
        let mut p = player.lock().unwrap();
        p.preload(&mut ExecutionLimit::none());
        let t0 = Instant::now();
        p.run_frame();
        frame_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        let t0 = Instant::now();
        p.render();
        render_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        if save {
            let renderer = <dyn Any>::downcast_mut::<TinySkiaRenderBackend>(p.renderer_mut())
                .expect("renderer must be tiny-skia");
            let path = format!("{out_dir}/frame_{i:04}.png");
            renderer.frame().save_png(&path).expect("save_png");
        }
    }

    let n = render_ms.len().max(1) as f64;
    let total: f64 = render_ms.iter().sum();
    let mean = total / n;
    let mut sorted = render_ms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[sorted.len() / 2];
    let min = sorted.first().copied().unwrap_or(0.0);
    let max = sorted.last().copied().unwrap_or(0.0);
    println!(
        "render(): {frames} frames  mean {mean:.2} ms  median {median:.2} ms  \
         min {min:.2}  max {max:.2}  (= {:.1} fps at mean)",
        1000.0 / mean
    );
    // Script and display-list work per frame, timed apart from render():
    // on the appliance both run on one thread, so their sum is the floor.
    let frame_mean: f64 = frame_ms.iter().sum::<f64>() / n;
    println!("run_frame(): mean {frame_mean:.2} ms");
    if save {
        println!("wrote {frames} frame(s) to {out_dir}/");
    }
}
