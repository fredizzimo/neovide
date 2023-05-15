use std::ffi::{c_void, CStr, CString};
use std::num::NonZeroU32;

use crate::cmd_line::CmdLineSettings;

use gl::MAX_RENDERBUFFER_SIZE;
use glutin::surface::{SwapInterval, PbufferSurface};
use glutin::{
    config::{Config, ConfigTemplateBuilder},
    context::{ContextAttributesBuilder, ContextAttributes, GlProfile, PossiblyCurrentContext, NotCurrentContext},
    display::GetGlDisplay,
    prelude::*,
    surface::{Surface, SurfaceAttributesBuilder, WindowSurface},
};
use glutin_winit::DisplayBuilder;
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
use winit::dpi::PhysicalSize;
use winit::{
    event_loop::EventLoop,
    window::{Window, WindowBuilder},
};

pub struct Context {
    surface: Surface<WindowSurface>,
    context: PossiblyCurrentContext,
    window: Window,
    config: Config,
}

pub fn clamp_render_buffer_size(size: PhysicalSize<u32>) -> PhysicalSize<u32> {
    PhysicalSize::new(
        size.width.clamp(1, MAX_RENDERBUFFER_SIZE),
        size.height.clamp(1, MAX_RENDERBUFFER_SIZE),
    )
}

impl Context {
    pub fn window(&self) -> &Window {
        &self.window
    }
    pub fn resize(&self, width: NonZeroU32, height: NonZeroU32) {
        GlSurface::resize(&self.surface, &self.context, width, height)
    }
    pub fn swap_buffers(&self) -> glutin::error::Result<()> {
        GlSurface::swap_buffers(&self.surface, &self.context)
    }
    pub fn get_proc_address(&self, addr: &CStr) -> *const c_void {
        GlDisplay::get_proc_address(&self.surface.display(), addr)
    }
    pub fn get_config(&self) -> &Config {
        &self.config
    }

    pub fn get_render_target_size(&self) -> PhysicalSize<u32> {
        clamp_render_buffer_size(self.window.inner_size())
    }

    pub fn new_shared(&self) {
        
    }
}

pub struct OffscreenContext {
    surface: Surface<PbufferSurface>,
    context: PossiblyCurrentContext,
}

impl OffscreenContext {
    pub fn get_proc_address(&self, addr: &str) -> *const c_void {
        let addr = CString::new(addr).unwrap();
        GlDisplay::get_proc_address(&self.surface.display(), addr.as_c_str())
    }

}

fn gen_config(mut config_iterator: Box<dyn Iterator<Item = Config> + '_>) -> Config {
    config_iterator.next().unwrap()
}

pub fn build_window<TE>(
    winit_window_builder: WindowBuilder,
    event_loop: &EventLoop<TE>,
) -> (Window, Config) {
    let template_builder = ConfigTemplateBuilder::new()
        .with_stencil_size(8)
        .with_transparency(true);
    let (window, config) = DisplayBuilder::new()
        .with_window_builder(Some(winit_window_builder))
        .build(event_loop, template_builder, gen_config)
        .expect("Failed to create Window");
    (window.expect("Could not create Window"), config)
}

pub fn build_context(
    window: Window,
    config: Config,
    cmd_line_settings: &CmdLineSettings,
) -> Context {
    let gl_display = config.display();
    let raw_window_handle = window.raw_window_handle();

    let size = clamp_render_buffer_size(window.inner_size());

    let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new()
        .with_srgb(Some(cmd_line_settings.srgb))
        .build(
            raw_window_handle,
            NonZeroU32::new(size.width).unwrap(),
            NonZeroU32::new(size.height).unwrap(),
        );
    let surface = unsafe { gl_display.create_window_surface(&config, &surface_attributes) }
        .expect("Failed to create Windows Surface");

    let context_attributes = ContextAttributesBuilder::new()
        .with_profile(GlProfile::Core)
        .build(Some(raw_window_handle));
    let context = unsafe { gl_display.create_context(&config, &context_attributes) }
        .expect("Failed to create OpenGL context")
        .make_current(&surface)
        .unwrap();

    // NOTE: We don't care if these fails, the driver can override the SwapInterval in any case, so it needs to work in all cases
    let _ = if cmd_line_settings.vsync {
        surface.set_swap_interval(&context, SwapInterval::Wait(NonZeroU32::new(1).unwrap()))
    } else {
        surface.set_swap_interval(&context, SwapInterval::DontWait)
    };

    Context {
        surface,
        context,
        window,
        config,
    }
}

pub fn build_offscreen_context(
    config: Config,
) -> OffscreenContext {
    let gl_display = config.display();

    let surface_attributes = SurfaceAttributesBuilder::<PbufferSurface>::new()
        .build(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        );
    let surface = unsafe { gl_display.create_pbuffer_surface(&config, &surface_attributes) }
        .expect("Failed to create Pbuffer Surface");
    let context_attributes = ContextAttributesBuilder::new()
        .with_profile(GlProfile::Core)
        .build(None);
    let context = unsafe { gl_display.create_context(&config, &context_attributes) }
        .expect("Failed to create OpenGL context")
        .make_current(&surface)
        .unwrap();

    OffscreenContext {
        surface,
        context,
    }
}
