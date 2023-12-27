use std::{
    convert::TryInto,
    env,
    env::consts::OS,
    ffi::{c_void, CStr, CString},
    num::NonZeroU32,
};

use gl::{types::*, MAX_RENDERBUFFER_SIZE};
use glutin::surface::SwapInterval;
use glutin::{
    config::{Config, ConfigTemplateBuilder},
    context::{ContextAttributesBuilder, GlProfile, PossiblyCurrentContext},
    display::GetGlDisplay,
    prelude::*,
    surface::{Surface, SurfaceAttributesBuilder, WindowSurface},
};
use glutin_winit::DisplayBuilder;
use raw_window_handle::HasRawWindowHandle;
use winit::dpi::PhysicalSize;
use winit::{
    event_loop::EventLoop,
    window::{Window, WindowBuilder},
};

pub struct GlWindow {
    pub window: Window,
    config: Config,
}

pub struct SkiaRenderer {
    // NOTE: The destruction order is important, so don't re-arrange
    // If possible keep it the reverse of the initialization order
    skia_surface: skia_safe::Surface,
    fb_info: FramebufferInfo,
    pub gr_context: DirectContext,
    context: PossiblyCurrentContext,
    window_surface: Surface<WindowSurface>,
    config: Config,
    window: Window,
}

use skia_safe::{
    gpu::{
        backend_render_targets::make_gl, gl::FramebufferInfo, surfaces::wrap_backend_render_target,
        DirectContext, SurfaceOrigin,
    },
    Canvas, ColorType,
};

fn clamp_render_buffer_size(size: &PhysicalSize<u32>) -> PhysicalSize<u32> {
    PhysicalSize::new(
        size.width.clamp(1, MAX_RENDERBUFFER_SIZE),
        size.height.clamp(1, MAX_RENDERBUFFER_SIZE),
    )
}

fn get_proc_address(surface: &Surface<WindowSurface>, addr: &CStr) -> *const c_void {
    GlDisplay::get_proc_address(&surface.display(), addr)
}

impl SkiaRenderer {
    pub fn new(window: GlWindow, srgb: bool, vsync: bool) -> Self {
        let config = window.config;
        let window = window.window;
        let gl_display = config.display();
        let raw_window_handle = window.raw_window_handle();

        let size = clamp_render_buffer_size(&window.inner_size());

        let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new()
            .with_srgb(Some(srgb))
            .build(
                raw_window_handle,
                NonZeroU32::new(size.width).unwrap(),
                NonZeroU32::new(size.height).unwrap(),
            );
        let window_surface =
            unsafe { gl_display.create_window_surface(&config, &surface_attributes) }
                .expect("Failed to create Windows Surface");

        let context_attributes = ContextAttributesBuilder::new()
            .with_profile(GlProfile::Core)
            .build(Some(raw_window_handle));
        let context = unsafe { gl_display.create_context(&config, &context_attributes) }
            .expect("Failed to create OpenGL context")
            .make_current(&window_surface)
            .unwrap();

        // NOTE: We don't care if these fails, the driver can override the SwapInterval in any case, so it needs to work in all cases
        // The OpenGL VSync is always disabled on Wayland and Windows, since they have their own
        // implementation
        let _ = if vsync && env::var("WAYLAND_DISPLAY").is_err() && OS != "windows" && OS != "macos"
        {
            window_surface
                .set_swap_interval(&context, SwapInterval::Wait(NonZeroU32::new(1).unwrap()))
        } else {
            window_surface.set_swap_interval(&context, SwapInterval::DontWait)
        };

        gl::load_with(|s| get_proc_address(&window_surface, CString::new(s).unwrap().as_c_str()));

        let interface = skia_safe::gpu::gl::Interface::new_load_with(|name| {
            if name == "eglGetCurrentDisplay" {
                return std::ptr::null();
            }
            get_proc_address(&window_surface, CString::new(name).unwrap().as_c_str())
        })
        .expect("Could not create interface");

        let mut gr_context = skia_safe::gpu::DirectContext::new_gl(Some(interface), None)
            .expect("Could not create direct context");
        let fb_info = {
            let mut fboid: GLint = 0;
            unsafe { gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut fboid) };

            FramebufferInfo {
                fboid: fboid.try_into().expect("Could not create frame buffer id"),
                format: skia_safe::gpu::gl::Format::RGBA8.into(),
                ..Default::default()
            }
        };
        let skia_surface = create_surface(
            &config,
            &window.inner_size(),
            &context,
            &window_surface,
            &mut gr_context,
            &fb_info,
        );

        Self {
            window_surface,
            context,
            window,
            config,
            gr_context,
            fb_info,
            skia_surface,
        }
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn swap_buffers(&self) {
        let _ = GlSurface::swap_buffers(&self.window_surface, &self.context);
    }

    pub fn canvas(&mut self) -> &Canvas {
        self.skia_surface.canvas()
    }

    pub fn resize(&mut self) {
        self.skia_surface = create_surface(
            &self.config,
            &self.window.inner_size(),
            &self.context,
            &self.window_surface,
            &mut self.gr_context,
            &self.fb_info,
        );
    }
}

fn gen_config(mut config_iterator: Box<dyn Iterator<Item = Config> + '_>) -> Config {
    config_iterator.next().unwrap()
}

pub fn build_window<TE>(
    winit_window_builder: WindowBuilder,
    event_loop: &EventLoop<TE>,
) -> GlWindow {
    let template_builder = ConfigTemplateBuilder::new()
        .with_stencil_size(8)
        .with_transparency(true);
    let (window, config) = DisplayBuilder::new()
        .with_window_builder(Some(winit_window_builder))
        .build(event_loop, template_builder, gen_config)
        .expect("Failed to create Window");
    let window = window.expect("Could not create Window");
    GlWindow { window, config }
}

fn create_surface(
    pixel_format: &Config,
    size: &PhysicalSize<u32>,
    context: &PossiblyCurrentContext,
    window_surface: &Surface<WindowSurface>,
    gr_context: &mut DirectContext,
    fb_info: &FramebufferInfo,
) -> skia_safe::Surface {
    let size = clamp_render_buffer_size(size);
    let backend_render_target = make_gl(
        size.into(),
        Some(pixel_format.num_samples().into()),
        pixel_format.stencil_size().into(),
        *fb_info,
    );

    let width = NonZeroU32::new(size.width).unwrap();
    let height = NonZeroU32::new(size.height).unwrap();
    GlSurface::resize(window_surface, context, width, height);
    wrap_backend_render_target(
        gr_context,
        &backend_render_target,
        SurfaceOrigin::BottomLeft,
        ColorType::RGBA8888,
        None,
        None,
    )
    .expect("Could not create skia surface")
}
