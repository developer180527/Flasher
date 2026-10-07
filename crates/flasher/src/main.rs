#![cfg_attr(windows, windows_subsystem = "windows")]
//! The window: one winit window, one wgpu surface, one libgui `Ui`.
//! Adapted from libgui's `libgui_pad` host.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::ExitCode;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use flasher::App;
use libgui::*;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Theme as OsTheme, Window, WindowId};

const FONT: &[u8] = include_bytes!("../../../assets/Inter.ttf");

/// The app's id: the Linux desktop file and icon are named after it, and a
/// window carrying it is matched to them by docks and task switchers.
#[cfg(all(unix, not(target_os = "macos")))]
const APP_ID: &str = "io.github.developer180527.flasher";

struct Live {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: libgui_wgpu::Renderer,
    ui: Ui,
    platform: libgui_winit::PlatformState,
    last: Instant,
    /// Time since the last frame that was actually built.
    idle: f32,
    batches: Vec<Batch>,
    clear: Color,
    /// From the last built frame: when the UI next wants one (a caret blink,
    /// an animation), so the event loop can sleep until then.
    repaint_after: Option<f32>,
}

struct Host {
    live: Option<Live>,
    app: App,
    clipboard: Option<arboard::Clipboard>,
    /// The open Browse… dialog: what it will pick, polled between events.
    browse: Option<Pin<Box<dyn Future<Output = Option<PathBuf>> + Send>>>,
    /// Wakes the event loop when the dialog finishes.
    waker: Waker,
}

/// A `Waker` that wakes the event loop, so a finished dialog is noticed
/// even while the loop sleeps waiting for input.
struct WakeLoop(EventLoopProxy<()>);

impl Wake for WakeLoop {
    fn wake(self: Arc<Self>) {
        let _ = self.0.send_event(());
    }
}

impl Host {
    fn start(&mut self, el: &ActiveEventLoop) {
        let attrs = Window::default_attributes()
            .with_title("Flasher")
            .with_inner_size(LogicalSize::new(680.0, 640.0))
            .with_min_inner_size(LogicalSize::new(520.0, 540.0));
        // macOS takes the icon from the app bundle; elsewhere the window
        // carries its own.
        #[cfg(not(target_os = "macos"))]
        let attrs = attrs.with_window_icon(window_icon());
        #[cfg(all(unix, not(target_os = "macos")))]
        let attrs = {
            use winit::platform::{
                wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11,
            };
            let attrs = WindowAttributesExtWayland::with_name(attrs, APP_ID, "flasher");
            WindowAttributesExtX11::with_name(attrs, APP_ID, "flasher")
        };
        let window = Arc::new(el.create_window(attrs).expect("window"));

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(el.owned_display_handle()),
        ));
        let surface = instance.create_surface(window.clone()).expect("surface");
        // The integrated GPU first, then any GPU, then the OS's software
        // renderer (WARP on Windows), so a PC with a missing or broken
        // graphics driver still gets a window rather than a crash.
        let adapter = [
            (wgpu::PowerPreference::LowPower, false),
            (wgpu::PowerPreference::HighPerformance, false),
            (wgpu::PowerPreference::None, true),
        ]
        .into_iter()
        .find_map(|(power_preference, force_fallback_adapter)| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference,
                force_fallback_adapter,
                compatible_surface: Some(&surface),
                ..Default::default()
            }))
            .ok()
        })
        .expect("no graphics adapter could draw the window, not even a software one");
        // The UI needs little: ask for no more than older GPUs give, at the
        // largest texture size this one supports.
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .expect("the graphics adapter refused to start");

        let px = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, px.width.max(1), px.height.max(1))
            .expect("config");
        let caps = surface.get_capabilities(&adapter);
        config.format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);
        let renderer = libgui_wgpu::Renderer::new(&device, &queue, config.format);

        let mut ui = Ui::new(Theme::dark(), FONT).expect("bundled font");
        libgui_keymap::Keymap::<u8>::for_current_platform().install(&mut ui);
        let clear = ui.theme.palette.bg_app;
        // Unknown counts as dark, which is libgui's default look.
        self.app
            .set_system_dark(window.theme() != Some(OsTheme::Light));

        self.clipboard = arboard::Clipboard::new().ok();
        self.live = Some(Live {
            window,
            surface,
            config,
            device,
            queue,
            renderer,
            ui,
            platform: libgui_winit::PlatformState::default(),
            last: Instant::now(),
            idle: 0.0,
            batches: Vec::new(),
            clear,
            repaint_after: None,
        });
    }

    fn draw(&mut self) {
        let Some(w) = self.live.as_mut() else { return };

        let px = w.window.inner_size();
        if px.width.max(1) != w.config.width || px.height.max(1) != w.config.height {
            w.config.width = px.width.max(1);
            w.config.height = px.height.max(1);
            w.surface.configure(&w.device, &w.config);
        }

        let now = Instant::now();
        w.idle += (now - w.last).as_secs_f32().min(0.1);
        w.last = now;

        let scale = w.window.scale_factor() as f32;
        let info = FrameInfo {
            screen_size: Vec2::new(
                w.config.width as f32 / scale,
                w.config.height as f32 / scale,
            ),
            scale,
            dt: w.idle,
        };

        let mut platform = PlatformOutput::default();
        if self.app.busy() || w.ui.needs_frame_for(&info, w.idle) {
            w.idle = 0.0;
            w.ui.begin_frame(info);
            self.app.ui(&mut w.ui);
            let out = w.ui.end_frame();
            platform = out.platform.clone();
            w.repaint_after = platform.repaint_after;
            w.clear = out.clear_color;
            w.renderer.prepare(&out);
            w.batches.clear();
            w.batches.extend_from_slice(&out.draw.batches);
        }

        let frame = match w.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => {
                w.surface.configure(&w.device, &w.config);
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = w.device.create_command_encoder(&Default::default());
        {
            let c = w.clear;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: c.r as f64,
                            g: c.g as f64,
                            b: c.b as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            w.renderer.render_batches(&mut pass, &w.batches);
        }
        w.queue.submit([encoder.finish()]);
        w.window.pre_present_notify();
        w.queue.present(frame);
        // Let wgpu free what finished frames used (upload staging and the
        // like). Without it that memory is reclaimed only now and then, and a
        // window redrawn for minutes on end grew by gigabytes.
        let _ = w.device.poll(wgpu::PollType::Poll);

        w.platform.apply(&w.window, &platform);
        if let Some(text) = platform.copied_text {
            if let Some(c) = self.clipboard.as_mut() {
                let _ = c.set_text(text);
            }
        }
        if platform.paste_requested {
            if let Some(text) = self.clipboard.as_mut().and_then(|c| c.get_text().ok()) {
                w.ui.push(InputEvent::Paste(text));
            }
        }

        // Native dialogs are the host's job. Never a blocking one here: this
        // runs inside winit's event handler, and on macOS a modal dialog's
        // run loop delivers winit events into it, which winit aborts on. The
        // asynchronous dialog (a sheet on macOS) leaves the loop running;
        // `about_to_wait` collects the answer.
        if let Some(text) = self.app.want_copy.take() {
            if let Some(c) = self.clipboard.as_mut() {
                let _ = c.set_text(text);
            }
        }
        if std::mem::take(&mut self.app.want_browse) && self.browse.is_none() {
            let mut dialog = rfd::AsyncFileDialog::new();
            if let Some(dir) = self.app.last_image_dir() {
                dialog = dialog.set_directory(dir);
            }
            let picked = dialog
                .add_filter(
                    "Disk images",
                    &["img", "iso", "raw", "bin", "dmg", "gz", "xz", "zst", "bz2"],
                )
                .add_filter("All files", &["*"])
                .set_parent(&*w.window)
                .pick_file();
            self.browse = Some(Box::pin(async move {
                picked.await.map(|f| f.path().to_path_buf())
            }));
        }
    }
}

/// The window app has no terminal to print a panic to: keep it in
/// `crash.log` beside the settings, and on Windows say so in a dialog,
/// instead of the window just vanishing.
fn report_crashes() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let log = flasher::prefs::PrefsStore::default_location()
            .map(|s| s.path().with_file_name("crash.log"));
        let mut text = format!("Flasher {} stopped: {info}", env!("CARGO_PKG_VERSION"));
        flasher_cli::journal::line(format_args!("crashed: {info}"));
        if let Some(log) = &log {
            if let Some(dir) = log.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if std::fs::write(log, &text).is_ok() {
                text += &format!("\n\nSaved in {}", log.display());
            }
        }
        text += "\n\nPlease report it: https://github.com/developer180527/Flasher/issues";
        // A worker thread's panic is reported by the app itself; only the
        // window's own thread ending means the app is gone.
        #[cfg(windows)]
        if std::thread::current().name() == Some("main") {
            windows::error_box("Flasher", &text);
        }
        #[cfg(not(windows))]
        let _ = text;
    }));
}

/// Flasher's icon for the title bar and taskbar, from the PNG in assets.
#[cfg(not(target_os = "macos"))]
fn window_icon() -> Option<winit::window::Icon> {
    let png = include_bytes!("../../../assets/icons/flasher-256.png");
    let mut reader = png::Decoder::new(std::io::Cursor::new(&png[..]))
        .read_info()
        .ok()?;
    let mut rgba = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut rgba).ok()?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return None;
    }
    rgba.truncate(frame.buffer_size());
    winit::window::Icon::from_rgba(rgba, frame.width, frame.height).ok()
}

impl ApplicationHandler for Host {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.live.is_none() {
            self.start(el);
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(w) = self.live.as_mut() else { return };
        libgui_winit::push_window_event(&mut w.ui, &event, w.window.scale_factor());
        match event {
            // Not mid-write: the app first stops the job, and closes when it
            // has (see `about_to_wait`).
            WindowEvent::CloseRequested => {
                if self.app.request_close() {
                    el.exit();
                } else {
                    w.ui.request_repaint();
                    w.window.request_redraw();
                }
            }
            WindowEvent::ThemeChanged(t) => {
                self.app.set_system_dark(t == OsTheme::Dark);
                w.ui.request_repaint();
                w.window.request_redraw();
            }
            WindowEvent::DroppedFile(path) => {
                self.app.set_image(&path);
                w.ui.request_repaint();
                w.window.request_redraw();
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                w.window.request_redraw()
            }
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let Some(w) = self.live.as_mut() else { return };
        // Drives plugged in or removed, or a job's progress, arrive from
        // background threads with no input event to wake us: look for them.
        if self.app.tick() {
            w.ui.request_repaint();
        }
        if let Some(dialog) = self.browse.as_mut() {
            if let Poll::Ready(picked) = dialog.as_mut().poll(&mut Context::from_waker(&self.waker))
            {
                self.browse = None;
                if let Some(path) = picked {
                    self.app.set_image(&path);
                }
                w.ui.request_repaint();
                w.window.request_redraw();
            }
        }
        // ⌘Q, the Dock's Quit, logging out: a close request like any other.
        #[cfg(target_os = "macos")]
        {
            macos::set_busy(self.app.busy());
            if macos::take_quit_request() && self.app.request_close() {
                el.exit();
                return;
            }
        }
        if self.app.should_close() {
            el.exit();
            return;
        }
        let elapsed = w.idle + w.last.elapsed().as_secs_f32();
        // A running job redraws for its progress, but at JOB_FPS, not the
        // display's refresh rate: a status line needs no 120 frames a second.
        let since = w.last.elapsed().as_secs_f32();
        let job_due = self.app.busy() && since >= 1.0 / JOB_FPS;
        if job_due || w.ui.needs_frame(elapsed) {
            w.window.request_redraw();
        }
        // Sleep until input, waking early for a running write's progress or
        // for whatever the UI asked to animate.
        let mut wake = w.repaint_after.map(|t| (t - elapsed).max(0.0));
        if self.app.busy() || self.app.pending() {
            // A write's progress, or an image or drive list being read in
            // the background, arrives with no input event: look again soon.
            let next = (1.0 / JOB_FPS - since).clamp(0.0, 0.05);
            wake = Some(wake.map_or(next, |t| t.min(next)));
        } else if self.app.settings.auto_refresh {
            // Often enough that a new drive appears promptly; this is a
            // channel check, not a disk listing.
            wake = Some(wake.map_or(0.5, |t| t.min(0.5)));
        }
        el.set_control_flow(match wake {
            Some(t) => {
                ControlFlow::WaitUntil(Instant::now() + Duration::from_secs_f32(t.max(0.005)))
            }
            None => ControlFlow::Wait,
        });
    }
}

/// How often the window redraws while a job runs and nothing else changes.
const JOB_FPS: f32 = 10.0;

fn main() -> ExitCode {
    // A command means the terminal; none means the window. (Old macOS adds a
    // `-psn_…` argument when an app is opened from the Finder: not a command.)
    // Mutable only on Windows, which strips the relaunch marker below.
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with("-psn_"))
        .collect();
    #[cfg(windows)]
    let relaunched = {
        let before = args.len();
        args.retain(|a| a != windows::RELAUNCHED);
        args.len() != before
    };
    if !args.is_empty() {
        #[cfg(windows)]
        windows::attach_console();
        return flasher_cli::run(args);
    }
    #[cfg(windows)]
    if !relaunched && !windows::is_elevated() && windows::relaunch_elevated() {
        return ExitCode::SUCCESS;
    }
    flasher_cli::journal::open_default();
    report_crashes();
    let el = EventLoop::new().expect("event loop");
    #[cfg(target_os = "macos")]
    if !macos::guard_quit() {
        eprintln!("flasher: could not guard Quit; quitting during a write will stop it");
    }
    let waker = Waker::from(Arc::new(WakeLoop(el.create_proxy())));
    let mut host = Host {
        browse: None,
        waker,
        live: None,
        app: match flasher::prefs::PrefsStore::default_location() {
            Some(store) => App::with_store(libflasher::current_platform(), store),
            None => App::new(libflasher::current_platform()),
        },
        clipboard: None,
    };
    el.run_app(&mut host).expect("run");
    ExitCode::SUCCESS
}
