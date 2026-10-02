//! The real UI with no window, no GPU and no event loop.
//!
//! [`Headless`] builds frames of the same [`App`] the window shows, delivers
//! input as pointer events at the controls' drawn positions — so a click here
//! goes through hit-testing, `enabled` scopes and all, exactly like a mouse —
//! and renders frames to PNG with libgui's CPU reference renderer.
//!
//! Pair it with [`libflasher::mock::MockPlatform`] and a whole flash runs
//! in a test or on a CI machine with no display and no USB port.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use libflasher::Platform;
use libgui::*;

use crate::App;

const FONT: &[u8] = include_bytes!("../../../assets/Inter.ttf");

pub struct Headless {
    pub app: App,
    pub ui: Ui,
    pub info: FrameInfo,
}

impl Headless {
    pub fn new(platform: Arc<dyn Platform>) -> Self {
        Self::with_app(App::new(platform))
    }

    /// Drive an `App` built some other way, e.g. with a short drive-poll interval.
    pub fn with_app(app: App) -> Self {
        let mut ui = Ui::new(Theme::dark(), FONT).expect("bundled font");
        libgui_keymap::Keymap::<u8>::for_current_platform().install(&mut ui);
        let info = FrameInfo {
            screen_size: Vec2::new(680.0, 640.0),
            scale: 1.0,
            dt: 1.0 / 60.0,
        };
        let mut h = Self { app, ui, info };
        // libgui hit-tests against the previous frame's rects: settle first.
        h.frames(3);
        h
    }

    /// Build one frame, as the window would on a redraw.
    pub fn frame(&mut self) {
        self.ui.begin_frame(self.info);
        self.app.ui(&mut self.ui);
        let _ = self.ui.end_frame();
    }

    pub fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.frame();
        }
    }

    /// Where a named control was drawn last frame (see `App::controls`).
    pub fn control(&self, name: &str) -> Option<Rect> {
        self.app
            .controls
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, r)| *r)
    }

    /// Click a control by name with a real move/press/release.
    pub fn click(&mut self, name: &str) -> Result<(), String> {
        self.frame();
        let r = self.control(name).ok_or_else(|| {
            let shown: Vec<_> = self.app.controls.iter().map(|(n, _)| *n).collect();
            format!("no control {name:?} on screen; showing {shown:?}")
        })?;
        let pos = Vec2::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
        self.ui.push(InputEvent::PointerMoved { pos });
        self.frame();
        self.ui.push(InputEvent::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
        });
        self.frame();
        self.ui.push(InputEvent::PointerButton {
            button: PointerButton::Primary,
            pressed: false,
        });
        self.frame();
        self.ui.push(InputEvent::PointerLeft);
        self.frame();
        // Responses carry last frame's rects: one more frame gives anything
        // the click revealed its real position.
        self.frame();
        Ok(())
    }

    /// Click the middle of `r`: for a part of a control, like one segment
    /// of a segmented picker.
    pub fn click_at(&mut self, r: Rect) {
        let pos = Vec2::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
        self.ui.push(InputEvent::PointerMoved { pos });
        self.frame();
        self.ui.push(InputEvent::PointerButton {
            button: PointerButton::Primary,
            pressed: true,
        });
        self.frame();
        self.ui.push(InputEvent::PointerButton {
            button: PointerButton::Primary,
            pressed: false,
        });
        self.frame();
        self.ui.push(InputEvent::PointerLeft);
        self.frame();
        // Responses carry last frame's rects: one more frame gives anything
        // the click revealed its real position.
        self.frame();
    }

    /// What dropping a file on the window does.
    pub fn drop_file(&mut self, path: &Path) {
        self.app.set_image(path);
        self.frame();
    }

    /// Keep building frames until no write is running, as the window's
    /// event loop does while it shows progress.
    pub fn wait_until_idle(&mut self, timeout: Duration) -> Result<(), String> {
        let end = Instant::now() + timeout;
        while self.app.busy() {
            if Instant::now() > end {
                return Err(format!("still busy after {timeout:?}"));
            }
            self.frame();
            std::thread::sleep(Duration::from_millis(5));
        }
        self.frame();
        Ok(())
    }

    /// Render the current UI to a PNG with the CPU reference renderer.
    pub fn snapshot(&mut self, path: &Path) -> std::io::Result<()> {
        self.frame();
        self.ui.begin_frame(self.info);
        self.app.ui(&mut self.ui);
        let out = self.ui.end_frame();
        let (w, h) = (
            (self.info.screen_size.x * self.info.scale) as u32,
            (self.info.screen_size.y * self.info.scale) as u32,
        );
        let img = libgui_soft::SoftRenderer::new().render_to_image(&out, w, h);
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut enc = png::Encoder::new(file, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&img.data)?;
        Ok(())
    }
}
