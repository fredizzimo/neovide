use crate::cmd_line::CmdLineSettings;
use std::convert::TryInto;

use gl::types::*;
use glutin::{ContextBuilder, GlProfile, NotCurrent, WindowedContext};
use skia_safe::{
    gpu::{gl::FramebufferInfo, BackendRenderTarget, DirectContext, SurfaceOrigin},
    Canvas, ColorType, Surface,
};
use winit::{event_loop::EventLoop, window::WindowBuilder};

pub type Context = WindowedContext<glutin::PossiblyCurrent>;

pub fn build_context<TE>(
    cmd_line_settings: &CmdLineSettings,
    winit_window_builder: WindowBuilder,
    event_loop: &EventLoop<TE>,
) -> WindowedContext<NotCurrent> {
    let builder = ContextBuilder::new()
        .with_pixel_format(24, 8)
        .with_stencil_buffer(8)
        .with_gl_profile(GlProfile::Core)
        .with_srgb(cmd_line_settings.srgb)
        .with_vsync(cmd_line_settings.vsync);

    match builder
        .clone()
        .build_windowed(winit_window_builder.clone(), event_loop)
    {
        Ok(ctx) => ctx,
        Err(err) => {
            // haven't found any sane way to actually match on the pattern rabbithole CreationError
            // provides, so here goes nothing
            if err.to_string().contains("vsync") {
                builder
                    .with_vsync(false)
                    .build_windowed(winit_window_builder, event_loop)
                    .unwrap()
            } else {
                panic!("{}", err);
            }
        }
    }
}

fn create_surface(
    windowed_context: &Context,
    gr_context: &mut DirectContext,
    fb_info: FramebufferInfo,
) -> Surface {
    let pixel_format = windowed_context.get_pixel_format();
    let size = windowed_context.window().inner_size();
    let size = (
        size.width.try_into().expect("Could not convert width"),
        size.height.try_into().expect("Could not convert height"),
    );
    let backend_render_target = BackendRenderTarget::new_gl(
        size,
        pixel_format
            .multisampling
            .map(|s| s.try_into().expect("Could not convert multisampling")),
        pixel_format
            .stencil_bits
            .try_into()
            .expect("Could not convert stencil"),
        fb_info,
    );
    windowed_context.resize(size.into());
    Surface::from_backend_render_target(
        gr_context,
        &backend_render_target,
        SurfaceOrigin::BottomLeft,
        ColorType::RGBA8888,
        None,
        None,
    )
    .expect("Could not create skia surface")
}

pub struct SkiaRenderer {
    pub gr_context: DirectContext,
    fb_info: FramebufferInfo,
    surface: Surface,
}

impl SkiaRenderer {
    pub fn new(windowed_context: &Context) -> SkiaRenderer {
        gl::load_with(|s| windowed_context.get_proc_address(s));

        let interface = skia_safe::gpu::gl::Interface::new_load_with(|name| {
            if name == "eglGetCurrentDisplay" {
                return std::ptr::null();
            }
            windowed_context.get_proc_address(name)
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
            }
        };
        let surface = create_surface(windowed_context, &mut gr_context, fb_info);

        SkiaRenderer {
            gr_context,
            fb_info,
            surface,
        }
    }

    pub fn canvas(&mut self) -> &mut Canvas {
        self.surface.canvas()
    }

    pub fn resize(&mut self, windowed_context: &Context) {
        self.surface = create_surface(windowed_context, &mut self.gr_context, self.fb_info);
    }
}
