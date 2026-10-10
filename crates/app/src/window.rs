//! The settings window and its OpenGL context.
//!
//! The window is created when the user asks for it and destroyed when they close it, so
//! that a running EyeSaver is a tray icon and a sleeping event loop - nothing is laid out,
//! rasterised or swapped while the window does not exist.

use std::num::NonZeroU32;
use std::sync::Arc;

use egui_glow::glow::{self, HasContext as _};
use eye_saver_core::settings::Settings;
use glutin::config::{Config, ConfigTemplateBuilder, GlConfig as _};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, NotCurrentGlContext as _, PossiblyCurrentContext,
    PossiblyCurrentGlContext as _,
};
use glutin::display::{Display, GetGlDisplay as _, GlDisplay as _};
use glutin::surface::{GlSurface as _, Surface, SurfaceAttributesBuilder, WindowSurface};
use glutin_winit::{ApiPreference, DisplayBuilder};
use raw_window_handle::HasWindowHandle as _;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::app::UserEvent;
use crate::ui;

/// The settings window: a winit window, a GL surface and an egui context.
pub struct SettingsWindow {
    window: Window,
    gl_context: PossiblyCurrentContext,
    gl_surface: Surface<WindowSurface>,
    gl: Arc<glow::Context>,
    egui: egui_glow::EguiGlow,
    /// Edited copy of the settings; only applied when the user saves.
    draft: Settings,
}

impl SettingsWindow {
    /// Creates the window and makes its GL context current.
    ///
    /// # Errors
    ///
    /// If no usable OpenGL configuration exists, or the window cannot be created.
    pub fn open(
        event_loop: &ActiveEventLoop,
        settings: Settings,
        proxy: EventLoopProxy<UserEvent>,
    ) -> Result<Self, String> {
        let attributes = WindowAttributes::default()
            .with_title("EyeSaver Settings")
            .with_inner_size(LogicalSize::new(460.0, 400.0))
            .with_min_inner_size(LogicalSize::new(380.0, 320.0))
            .with_resizable(true)
            // Shown once something has been painted, so it never flashes empty.
            .with_visible(false);

        let template = ConfigTemplateBuilder::new()
            .prefer_hardware_accelerated(None)
            .with_depth_size(0)
            .with_stencil_size(0)
            .with_transparency(false);

        let (window, gl_config) = DisplayBuilder::new()
            .with_preference(ApiPreference::FallbackEgl)
            .with_window_attributes(Some(attributes.clone()))
            .build(event_loop, template, pick_config)
            .map_err(|e| format!("no usable OpenGL configuration: {e}"))?;

        let window = match window {
            Some(window) => window,
            None => glutin_winit::finalize_window(event_loop, attributes, &gl_config)
                .map_err(|e| format!("creating the settings window: {e}"))?,
        };

        let display = gl_config.display();
        let handle = window
            .window_handle()
            .map_err(|e| format!("the settings window has no handle: {e}"))?
            .as_raw();

        let context_attributes = ContextAttributesBuilder::new().build(Some(handle));
        // A core context is preferred; some drivers - and most virtual machines - only
        // offer OpenGL ES, which egui is equally happy with.
        let fallback = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(None))
            .build(Some(handle));

        // SAFETY: the handle comes from the window created just above, which outlives the
        // context because both are owned by this struct.
        let not_current = unsafe {
            display
                .create_context(&gl_config, &context_attributes)
                .or_else(|first| {
                    log::debug!("no core GL context ({first}); trying OpenGL ES");
                    display.create_context(&gl_config, &fallback)
                })
        }
        .map_err(|e| format!("creating an OpenGL context: {e}"))?;

        let size = window.inner_size();
        let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            handle,
            NonZeroU32::new(size.width).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(size.height).unwrap_or(NonZeroU32::MIN),
        );
        // SAFETY: as above - the window outlives the surface.
        let gl_surface = unsafe { display.create_window_surface(&gl_config, &surface_attributes) }
            .map_err(|e| format!("creating the window surface: {e}"))?;

        let gl_context = not_current
            .make_current(&gl_surface)
            .map_err(|e| format!("making the OpenGL context current: {e}"))?;

        // Wait for vsync: the window repaints only on input, so this costs nothing and
        // avoids tearing.
        let _ = gl_surface.set_swap_interval(
            &gl_context,
            glutin::surface::SwapInterval::Wait(NonZeroU32::MIN),
        );

        let gl = Arc::new(load_gl(&display));
        let egui = egui_glow::EguiGlow::new(event_loop, Arc::clone(&gl), None, None, true);

        // egui is reactive: it asks for the next frame only when something has changed, and
        // says how long it can wait. Forwarding that into the event loop is what keeps an
        // open-but-idle window from repainting at all.
        let repaint_proxy = egui::mutex::Mutex::new(proxy);
        egui.egui_ctx.set_request_repaint_callback(move |info| {
            let _ = repaint_proxy
                .lock()
                .send_event(UserEvent::Repaint(info.delay));
        });

        Ok(Self {
            window,
            gl_context,
            gl_surface,
            gl,
            egui,
            draft: settings,
        })
    }

    /// The window's id, for matching winit events.
    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    /// Brings the window to the front.
    pub fn focus(&self) {
        self.window.set_visible(true);
        self.window.focus_window();
    }

    /// Replaces the draft, e.g. when the settings changed underneath the window.
    pub fn set_draft(&mut self, settings: Settings) {
        self.draft = settings;
    }

    /// Feeds a window event to egui. Returns whether egui wants a repaint.
    pub fn on_window_event(&mut self, event: &WindowEvent) -> bool {
        let response = self.egui.on_window_event(&self.window, event);
        if let WindowEvent::Resized(size) = event {
            self.resize(*size);
        }
        response.repaint
    }

    fn resize(&self, size: PhysicalSize<u32>) {
        if let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        {
            self.gl_surface.resize(&self.gl_context, width, height);
        }
    }

    /// Lays out and paints one frame, returning what the user asked for.
    pub fn draw(&mut self, saved: &Settings) -> ui::Action {
        let mut action = ui::Action::default();
        let draft = &mut self.draft;
        let mut background = egui::Color32::BLACK;
        self.egui.run(&self.window, |ui| {
            background = ui.visuals().panel_fill;
            egui::Frame::new()
                .fill(background)
                .inner_margin(egui::Margin {
                    left: 18,
                    right: 14,
                    top: 14,
                    bottom: 12,
                })
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        action = ui::settings_page(ui, draft, saved);
                    });
                });
        });

        if self.gl_context.make_current(&self.gl_surface).is_err() {
            log::warn!("could not make the settings window's GL context current");
            return action;
        }
        // SAFETY: the context above was just made current on this thread.
        unsafe {
            self.gl.clear_color(
                f32::from(background.r()) / 255.0,
                f32::from(background.g()) / 255.0,
                f32::from(background.b()) / 255.0,
                1.0,
            );
            self.gl.clear(glow::COLOR_BUFFER_BIT);
        }
        self.egui.paint(&self.window);
        if let Err(e) = self.gl_surface.swap_buffers(&self.gl_context) {
            log::warn!("could not present the settings window: {e}");
        }
        self.window.set_visible(true);
        action
    }

    /// The settings as currently edited.
    pub fn draft(&self) -> Settings {
        self.draft
    }

    /// Asks for another frame.
    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// Releases the GPU resources. Called before the window is dropped.
    pub fn close(mut self) {
        self.egui.destroy();
    }
}

/// Prefers the configuration with the most samples, which in practice means the plain
/// hardware-accelerated one.
fn pick_config(configs: Box<dyn Iterator<Item = Config> + '_>) -> Config {
    configs
        .reduce(|best, candidate| {
            if candidate.num_samples() > best.num_samples() {
                candidate
            } else {
                best
            }
        })
        .expect("glutin offered no configurations at all")
}

/// Binds the GL function pointers egui needs.
fn load_gl(display: &Display) -> egui_glow::glow::Context {
    // SAFETY: `get_proc_address` returns valid pointers for the current context, which was
    // made current just above, and glow only calls them while that context is current.
    unsafe {
        egui_glow::glow::Context::from_loader_function_cstr(|symbol| {
            display.get_proc_address(symbol).cast()
        })
    }
}
