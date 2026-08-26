//! Backlight brightness: map the panel's writes onto a real display.
//!
//! Two modes, selected by `player.toml` (config.rs):
//!
//! - **Default (kernel backlight).** `hardware_version` stays "3.8", so the
//!   panel shows the day/night sliders (`altBrightness`, DS1748) and
//!   `ScreenManager.setRawBrightness` writes 0–65535 to
//!   `/proc/sys/sense1/brightness` via `_putFile` (F2:9119). `RootFs`
//!   intercepts that write and `Backlight` scales it onto the device's own
//!   `0–max_brightness` — the range varies per driver (255 on the RPi
//!   displays, thousands on Intel panels, 1 on binary ones), so it is read,
//!   never assumed.
//! - **`brightness_ctl = "<executable>"`.** The host answers
//!   `chumby_version -h` with "3.7", the panel shows the bright/dim radio
//!   view and `ScreenManager.setDim` calls `_setLCDMute(level)` with the
//!   older models' discrete levels (LCD_ON 0 / LCD_DIM 1 / LCD_OFF 2,
//!   F2:9021); the executable runs with the level as its only argument.
//!
//! Neither available → the `settings-brightness` ui-policy rule keeps the
//! settings button disabled (`only_without_brightness`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Backlight {
    brightness_file: PathBuf,
    max: u32,
    /// Sliders write many times a second while dragged; warn once, not per write.
    write_failed: AtomicBool,
}

impl Backlight {
    pub fn detect() -> Option<Backlight> {
        Self::detect_in(Path::new("/sys/class/backlight"))
    }

    /// `rpi_backlight` (the original RPi 7" touch display) first; otherwise
    /// a lone device is unambiguous — backlight names are not stable across
    /// panels (the Touch Display 2 registers under its DSI panel's name).
    /// Several devices with no `rpi_backlight` is ambiguous: none is chosen.
    /// `pub(crate)` so fixture.rs tests can build one on a temp dir.
    pub(crate) fn detect_in(class_dir: &Path) -> Option<Backlight> {
        let preferred = class_dir.join("rpi_backlight");
        let dir = if preferred.is_dir() {
            preferred
        } else {
            let mut devices: Vec<PathBuf> = std::fs::read_dir(class_dir)
                .ok()?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .collect();
            match devices.len() {
                0 => return None,
                1 => devices.remove(0),
                _ => {
                    tracing::warn!(target: "chumby_host",
                        "{} backlight devices in {} and none is rpi_backlight — \
                         brightness stays unwired",
                        devices.len(), class_dir.display());
                    return None;
                }
            }
        };
        let max: u32 = std::fs::read_to_string(dir.join("max_brightness"))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        if max == 0 {
            return None;
        }
        if max == 1 {
            tracing::warn!(target: "chumby_host",
                "backlight {} has max_brightness=1 — it can only switch, not dim",
                dir.display());
        }
        tracing::info!(target: "chumby_host",
            "backlight {} (max_brightness={max})", dir.display());
        Some(Backlight {
            brightness_file: dir.join("brightness"),
            max,
            write_failed: AtomicBool::new(false),
        })
    }

    /// Panel space 0–65535 → device space 0–max. Linear; 0 stays 0 (the
    /// panel's screen-off state installs its own touch-to-restore handler),
    /// but any non-zero input lands at ≥1 so "dimmest" cannot round to "off".
    pub fn set_raw(&self, value: f64) {
        let value = value.clamp(0.0, 65535.0);
        let mut scaled = (value / 65535.0 * self.max as f64).round() as u32;
        if value > 0.0 {
            scaled = scaled.max(1);
        }
        match std::fs::write(&self.brightness_file, scaled.to_string()) {
            Ok(()) => {
                self.write_failed.store(false, Ordering::Relaxed);
            }
            Err(e) => {
                if !self.write_failed.swap(true, Ordering::Relaxed) {
                    tracing::warn!(target: "chumby_host",
                        "backlight write {} failed: {e} (permissions? see udev \
                         rule in the appliance repo)",
                        self.brightness_file.display());
                }
            }
        }
    }
}

/// `brightness_ctl` mode: run the configured executable with the discrete
/// level (0 on / 1 dim / 2 off) as its only argument. Spawned, not waited
/// for inline — `_setLCDMute` is called mid-frame — but reaped on a thread
/// so no zombies accumulate (the audio.rs lesson).
pub fn run_ctl(program: &Path, level: f64) {
    let level = (level as i64).clamp(0, 2);
    match std::process::Command::new(program).arg(level.to_string()).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!(target: "chumby_host",
            "brightness_ctl {} {level} failed to start: {e}", program.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_device(class_dir: &Path, name: &str, max: &str) {
        let dir = class_dir.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("max_brightness"), max).unwrap();
        std::fs::write(dir.join("brightness"), "0").unwrap();
    }

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("chumby-backlight-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Discovery: rpi_backlight preferred, a lone other device accepted
    /// (the TD2 registers under its panel's name), several is ambiguous.
    #[test]
    fn test_detection_prefers_rpi_then_lone_device() {
        let dir = tmp("detect");
        assert!(Backlight::detect_in(&dir).is_none(), "empty class dir");

        fake_device(&dir, "10-0045", "255");
        let b = Backlight::detect_in(&dir).expect("lone device");
        assert!(b.brightness_file.ends_with("10-0045/brightness"));
        assert_eq!(b.max, 255);

        fake_device(&dir, "rpi_backlight", "255");
        let b = Backlight::detect_in(&dir).expect("rpi_backlight");
        assert!(b.brightness_file.ends_with("rpi_backlight/brightness"));

        std::fs::remove_dir_all(dir.join("rpi_backlight")).unwrap();
        fake_device(&dir, "acpi_video0", "15");
        assert!(Backlight::detect_in(&dir).is_none(), "two devices, neither rpi");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The scale comes from the device's max_brightness — 255 is common but
    /// not universal (ACPI 15, Intel thousands). 0 stays 0; anything the
    /// panel means as "on" must not round down to "off".
    #[test]
    fn test_set_raw_scales_to_device_max() {
        let dir = tmp("scale");
        fake_device(&dir, "rpi_backlight", "255");
        let b = Backlight::detect_in(&dir).unwrap();
        let read = || -> u32 {
            std::fs::read_to_string(dir.join("rpi_backlight/brightness"))
                .unwrap()
                .parse()
                .unwrap()
        };

        b.set_raw(65535.0); // panel 100%: value ×655.35 (F2:9119)
        assert_eq!(read(), 255);
        b.set_raw(13107.0); // night default 20
        assert_eq!(read(), 51);
        b.set_raw(1.0); // dimmest non-zero must stay on
        assert_eq!(read(), 1);
        b.set_raw(0.0); // screen off (setDim FULL_OFF)
        assert_eq!(read(), 0);
        b.set_raw(1e9); // out of range clamps
        assert_eq!(read(), 255);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_bad_max_brightness_rejected() {
        let dir = tmp("badmax");
        fake_device(&dir, "rpi_backlight", "0");
        assert!(Backlight::detect_in(&dir).is_none());
        std::fs::write(dir.join("rpi_backlight/max_brightness"), "junk").unwrap();
        assert!(Backlight::detect_in(&dir).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
