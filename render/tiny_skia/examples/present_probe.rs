//! Step 1 probe for `claude/tiny-skia-backend-plan.md`: what does it cost to get
//! a CPU pixmap onto the screen under cage, before any scene rasterisation?
//!
//! Not part of the player. Answers path (A) softbuffer/`wl_shm` — the frame
//! never touches Vulkan — and breaks the per-frame wall time into the phases
//! that path would pay: paint, RGBA→X8R8G8B8 convert+copy, `present()`.
//!
//! Modes:
//!   direct    pixmap at window size, copied into the shm buffer
//!   upscale   pixmap at the movie's 320×240 stage, scaled up on the way out
//!             (the "render at stage size" lever, chumby-pi development.md §6)
//!   zerocopy  tiny-skia paints straight into the shm buffer, no copy at all
//!             (R/B come out swapped — this measures the copy's cost, it is
//!             not a correct present path)
//!
//! `present()` itself looks free in the phase breakdown because handing over a
//! `wl_shm` buffer is asynchronous — the compositing and the scanout/SPI upload
//! happen in cage's process. So the probe also accounts CPU from `/proc`: its
//! own (utime+stime) and the whole box's busy time, whose difference is what
//! presenting costs *outside* this process.
//!
//! Usage: present_probe [direct|upscale|zerocopy] [frames] [nearest|bilinear] [fps]
//!        fps 0 (default) runs unthrottled — the max-rate case; a real target
//!        rate (12 = the panel's ceiling) measures cost at the shipped load.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Linux clock ticks per second for `/proc` CPU fields (`_SC_CLK_TCK`), 100 on
/// every kernel this runs on.
const TICKS_PER_SEC: f64 = 100.0;

/// This process's CPU time (user+system) from `/proc/self/stat`.
fn self_cpu() -> Duration {
    let stat = std::fs::read_to_string("/proc/self/stat").expect("/proc/self/stat");
    // Fields after the comm field, which may itself contain spaces.
    let tail = &stat[stat.rfind(") ").expect("comm end") + 2..];
    let mut fields = tail.split_whitespace().skip(11); // utime is field 14
    let utime: f64 = fields.next().expect("utime").parse().expect("utime");
    let stime: f64 = fields.next().expect("stime").parse().expect("stime");
    Duration::from_secs_f64((utime + stime) / TICKS_PER_SEC)
}

/// Busy CPU time across all cores from `/proc/stat` (total minus idle+iowait) —
/// this is what catches the compositor's share of a present.
fn system_busy_cpu() -> Duration {
    let stat = std::fs::read_to_string("/proc/stat").expect("/proc/stat");
    let line = stat.lines().next().expect("cpu line");
    let v: Vec<f64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|f| f.parse().ok())
        .collect();
    let total: f64 = v.iter().sum();
    let idle = v.get(3).copied().unwrap_or(0.0) + v.get(4).copied().unwrap_or(0.0);
    Duration::from_secs_f64((total - idle) / TICKS_PER_SEC)
}

use tiny_skia::{
    Color, FilterQuality, Paint, Pixmap, PixmapMut, PixmapPaint, PixmapRef, Rect, Transform,
};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowId};

/// The movie's own stage size — what `upscale` rasterises at.
const STAGE: (u32, u32) = (320, 240);

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Direct,
    Upscale,
    ZeroCopy,
}

#[derive(Default)]
struct Phases {
    paint: Duration,
    copy: Duration,
    present: Duration,
    frames: u32,
}

struct App {
    mode: Mode,
    filter: FilterQuality,
    budget: u32,
    /// Target rate; `None` runs the loop unthrottled.
    period: Option<Duration>,
    stage: Pixmap,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    scratch: Option<Pixmap>,
    phases: Phases,
    started: Option<Instant>,
    deadline: Option<Instant>,
    cpu_at_start: (Duration, Duration),
    size: (u32, u32),
}

impl App {
    fn new(mode: Mode, filter: FilterQuality, budget: u32, fps: f64) -> Self {
        Self {
            mode,
            filter,
            budget,
            period: (fps > 0.0).then(|| Duration::from_secs_f64(1.0 / fps)),
            stage: Pixmap::new(STAGE.0, STAGE.1).expect("stage pixmap"),
            window: None,
            surface: None,
            scratch: None,
            phases: Phases::default(),
            started: None,
            deadline: None,
            cpu_at_start: (Duration::ZERO, Duration::ZERO),
            size: (0, 0),
        }
    }

    /// A full-buffer write with a moving marker: cheap, but neither the
    /// compositor nor the panel can treat two frames as identical (damage
    /// tracking would otherwise flatter the numbers).
    fn paint(pixmap: &mut PixmapMut, frame: u32) {
        let phase = (frame % 60) as f32 / 60.0;
        pixmap.fill(Color::from_rgba8(20, 20, (40.0 + 180.0 * phase) as u8, 255));
        let w = pixmap.width() as f32;
        let h = pixmap.height() as f32;
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(240, 180, 40, 255));
        if let Some(rect) = Rect::from_xywh(phase * (w - w / 8.0), h / 3.0, w / 8.0, h / 3.0) {
            pixmap.fill_rect(rect, &paint, Transform::identity(), None);
        }
    }

    /// RGBA8 premultiplied → softbuffer's 0x00RRGGBB, the copy path (A) pays.
    fn blit(src: PixmapRef, dst: &mut [u32]) {
        for (out, px) in dst.iter_mut().zip(src.pixels()) {
            *out = (px.red() as u32) << 16 | (px.green() as u32) << 8 | px.blue() as u32;
        }
    }

    fn frame(&mut self) {
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let (w, h) = self.size;
        let n = self.phases.frames;

        let mut buffer = surface.buffer_mut().expect("shm buffer");

        match self.mode {
            Mode::ZeroCopy => {
                let t = Instant::now();
                let bytes: &mut [u8] = bytemuck::cast_slice_mut(&mut buffer);
                let mut pixmap = PixmapMut::from_bytes(bytes, w, h).expect("wrap shm buffer");
                Self::paint(&mut pixmap, n);
                self.phases.paint += t.elapsed();
            }
            Mode::Direct => {
                let scratch = self.scratch.as_mut().expect("scratch pixmap");
                let t = Instant::now();
                Self::paint(&mut scratch.as_mut(), n);
                self.phases.paint += t.elapsed();

                let t = Instant::now();
                Self::blit(scratch.as_ref(), &mut buffer);
                self.phases.copy += t.elapsed();
            }
            Mode::Upscale => {
                let t = Instant::now();
                Self::paint(&mut self.stage.as_mut(), n);
                self.phases.paint += t.elapsed();

                let scratch = self.scratch.as_mut().expect("scratch pixmap");
                let t = Instant::now();
                let scale = Transform::from_scale(
                    w as f32 / STAGE.0 as f32,
                    h as f32 / STAGE.1 as f32,
                );
                scratch.fill(Color::TRANSPARENT);
                scratch.draw_pixmap(
                    0,
                    0,
                    self.stage.as_ref(),
                    &PixmapPaint {
                        quality: self.filter,
                        ..Default::default()
                    },
                    scale,
                    None,
                );
                Self::blit(scratch.as_ref(), &mut buffer);
                self.phases.copy += t.elapsed();
            }
        }

        let t = Instant::now();
        buffer.present().expect("present");
        self.phases.present += t.elapsed();

        self.phases.frames += 1;
    }

    fn report(&self) {
        let p = &self.phases;
        let n = p.frames.max(1) as f64;
        let wall = self.started.map(|s| s.elapsed()).unwrap_or_default();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0 / n;
        let mode = match self.mode {
            Mode::Direct => "direct",
            Mode::Upscale => "upscale",
            Mode::ZeroCopy => "zerocopy",
        };
        println!(
            "mode={mode} window={}x{} frames={} wall={:.2}s => {:.1} presents/s",
            self.size.0,
            self.size.1,
            p.frames,
            wall.as_secs_f64(),
            n / wall.as_secs_f64().max(f64::EPSILON)
        );
        println!(
            "  per frame: paint {:.3} ms | copy {:.3} ms | present {:.3} ms | total {:.3} ms",
            ms(p.paint),
            ms(p.copy),
            ms(p.present),
            ms(p.paint + p.copy + p.present)
        );

        let own = self_cpu().saturating_sub(self.cpu_at_start.0);
        let system = system_busy_cpu().saturating_sub(self.cpu_at_start.1);
        let elsewhere = system.saturating_sub(own);
        println!(
            "  CPU per frame: this process {:.3} ms | whole box {:.3} ms | \
             elsewhere (compositor+kernel) {:.3} ms",
            ms(own),
            ms(system),
            ms(elsewhere)
        );
        println!(
            "  CPU load: this process {:.0}% of a core | whole box {:.0}% of a core",
            own.as_secs_f64() / wall.as_secs_f64().max(f64::EPSILON) * 100.0,
            system.as_secs_f64() / wall.as_secs_f64().max(f64::EPSILON) * 100.0
        );
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // Fullscreen is the measurement case (cage gives the panel's own size);
        // PROBE_WINDOWED exists so a smoke test on a dev desktop stays polite.
        let mut attrs = Window::default_attributes().with_title("present_probe");
        attrs = if std::env::var_os("PROBE_WINDOWED").is_some() {
            attrs.with_inner_size(winit::dpi::PhysicalSize::new(480, 320))
        } else {
            attrs.with_fullscreen(Some(Fullscreen::Borderless(None)))
        };
        let window = Rc::new(event_loop.create_window(attrs).expect("window"));
        let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let mut surface =
            softbuffer::Surface::new(&context, window.clone()).expect("softbuffer surface");

        let inner = window.inner_size();
        let (w, h) = (inner.width.max(1), inner.height.max(1));
        surface
            .resize(NonZeroU32::new(w).unwrap(), NonZeroU32::new(h).unwrap())
            .expect("resize");
        self.size = (w, h);
        self.scratch = Some(Pixmap::new(w, h).expect("scratch pixmap"));
        // The context must outlive the surface; the surface owns its window
        // handle, so leaking the context is fine for a probe.
        std::mem::forget(context);
        self.surface = Some(surface);
        self.window = Some(window);
        self.started = Some(Instant::now());
        self.cpu_at_start = (self_cpu(), system_busy_cpu());
        event_loop.set_control_flow(ControlFlow::Poll);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                let (w, h) = (size.width.max(1), size.height.max(1));
                if (w, h) != self.size {
                    if let Some(surface) = self.surface.as_mut() {
                        surface
                            .resize(NonZeroU32::new(w).unwrap(), NonZeroU32::new(h).unwrap())
                            .expect("resize");
                    }
                    self.size = (w, h);
                    self.scratch = Some(Pixmap::new(w, h).expect("scratch pixmap"));
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_none() {
            return;
        }
        // Paced by sleeping, not by ControlFlow::WaitUntil: buffer-completion
        // events cancel the wait immediately and the loop runs flat out.
        if let Some(period) = self.period {
            let deadline = *self.deadline.get_or_insert_with(Instant::now);
            let now = Instant::now();
            if now < deadline {
                std::thread::sleep(deadline - now);
            }
            self.deadline = Some(deadline + period);
        }

        self.frame();
        if self.phases.frames >= self.budget {
            self.report();
            event_loop.exit();
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = match args.next().as_deref() {
        None | Some("direct") => Mode::Direct,
        Some("upscale") => Mode::Upscale,
        Some("zerocopy") => Mode::ZeroCopy,
        Some(other) => {
            eprintln!("unknown mode {other:?} (direct|upscale|zerocopy)");
            std::process::exit(2);
        }
    };
    let budget = args
        .next()
        .map(|s| s.parse().expect("frame count"))
        .unwrap_or(300);
    let filter = match args.next().as_deref() {
        None | Some("nearest") => FilterQuality::Nearest,
        Some("bilinear") => FilterQuality::Bilinear,
        Some(other) => {
            eprintln!("unknown filter {other:?} (nearest|bilinear)");
            std::process::exit(2);
        }
    };

    let fps = args
        .next()
        .map(|s| s.parse().expect("target fps"))
        .unwrap_or(0.0);

    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App::new(mode, filter, budget, fps);
    event_loop.run_app(&mut app).expect("run");
}
