use std::num::NonZeroU32;
use std::ffi::{c_void, CStr};
use crate::cmd_line::CmdLineSettings;
use winit::{
    event_loop::EventLoop,
    dpi::PhysicalSize,
    window::{Window, WindowBuilder},
};
use glutin::{
    config::{Config, ConfigTemplateBuilder},
};
use super::clamp_render_buffer_size;
use surfman::{
    SurfaceAccess, SurfaceType, Connection, ContextAttributes, ContextAttributeFlags,
    GLVersion, Adapter, Device, GLApi, Surface, Error,

};
use skia_safe::{
    gpu::{gl::FramebufferInfo, BackendRenderTarget, DirectContext, SurfaceOrigin},
    Canvas, ColorType,
};
use euclid::default::Size2D;
use raw_window_handle::HasRawWindowHandle;
use mozangle::egl::ffi::{
    SwapInterval,
    GetCurrentDisplay,
};


/*
impl GlConfig for Config {
    fn color_buffer_type(&self) -> Option<ColorBufferType> {
        gl_api_dispatch!(self; Self(config) => config.color_buffer_type())
    }

    fn float_pixels(&self) -> bool {
        gl_api_dispatch!(self; Self(config) => config.float_pixels())
    }

    fn alpha_size(&self) -> u8 {
        gl_api_dispatch!(self; Self(config) => config.alpha_size())
    }

    fn depth_size(&self) -> u8 {
        gl_api_dispatch!(self; Self(config) => config.depth_size())
    }

    fn stencil_size(&self) -> u8 {
        gl_api_dispatch!(self; Self(config) => config.stencil_size())
    }

    fn num_samples(&self) -> u8 {
        gl_api_dispatch!(self; Self(config) => config.num_samples())
    }

    fn srgb_capable(&self) -> bool {
        gl_api_dispatch!(self; Self(config) => config.srgb_capable())
    }

    fn config_surface_types(&self) -> ConfigSurfaceTypes {
        gl_api_dispatch!(self; Self(config) => config.config_surface_types())
    }

    fn hardware_accelerated(&self) -> bool {
        gl_api_dispatch!(self; Self(config) => config.hardware_accelerated())
    }

    fn supports_transparency(&self) -> Option<bool> {
        gl_api_dispatch!(self; Self(config) => config.supports_transparency())
    }

    fn api(&self) -> Api {
        gl_api_dispatch!(self; Self(config) => config.api())
    }
}
*/


pub struct Context {
    window: Window,
    connection: Connection,
    adapter: Adapter,
    device: Device,
    context: surfman::Context,
    //surface: Surface,
    
}

impl Context {
    pub fn window(&self) -> &Window {
        &self.window
    }

    /*
    pub fn resize(&self, width: NonZeroU32, height: NonZeroU32) {
        //let size = Size2D::new(width as i32, height as i32);
        //self.device.resize_surface(&self.context, 
    }
    */

    pub fn swap_buffers(&mut self) -> Result<(), Error> {
        let mut surface = self.device.unbind_surface_from_context(&mut self.context).unwrap().unwrap();
        let res = self.device.present_surface(&self.context, &mut surface);
        self.device.bind_surface_to_context(&mut self.context, surface);
        res
    }

    pub fn get_proc_address(&self, addr: &CStr) -> *const c_void {
        let addr_str = addr.to_str().unwrap();
        self.device.get_proc_address(&self.context, &addr_str)
    }

    pub fn create_surface(
        &mut self,
        gr_context: &mut DirectContext,
        fb_info: FramebufferInfo,
    ) -> skia_safe::Surface {
        unsafe {
            let display = GetCurrentDisplay();
            let res = SwapInterval(display, 0);
            assert!(res==1);
        }
        let size = clamp_render_buffer_size(self.window.inner_size());
        let size = Size2D::new(size.width as i32, size.height as i32);
        let mut surface = self.device.unbind_surface_from_context(&mut self.context).unwrap().unwrap();
        self.device.resize_surface(&self.context, &mut surface, size);
        self.device.bind_surface_to_context(&mut self.context, surface);
        let sample_count = 1;
        let stencil_bits = 8;
        let backend_render_target = BackendRenderTarget::new_gl((size.width, size.height), sample_count, stencil_bits, fb_info);
        skia_safe::Surface::from_backend_render_target(
            gr_context,
            &backend_render_target,
            SurfaceOrigin::BottomLeft,
            ColorType::RGBA8888,
            None,
            None,
        ).unwrap()

    }
}

pub fn build_context<TE>(
    cmd_line_settings: &CmdLineSettings,
    winit_window_builder: WindowBuilder,
    event_loop: &EventLoop<TE>,
) -> Context {
    let window = winit_window_builder
        .build(&event_loop).expect("Failed to create window");
    /*
    let raw_window_handle = window.raw_window_handle();
    let template_builder = ConfigTemplateBuilder::new()
        .with_stencil_size(8)
        .with_transparency(true)
        .compatible_with_native_window(raw_window_handle);
    let template = template_builder.build();
    */


    let connection = Connection::new().unwrap();
    let adapter = connection.create_adapter().unwrap();
    let mut device = connection.create_device(&adapter).unwrap();
    let version = if device.gl_api() == GLApi::GL {
        GLVersion::new(3, 3)
    } else {
        GLVersion::new(2, 0)
    };
    let attributes = ContextAttributes {
        version,
        flags: ContextAttributeFlags::ALPHA /*| ContextAttributeFlags::DEPTH*/ | ContextAttributeFlags::STENCIL,
    };
    let descriptor = device.create_context_descriptor(&attributes).unwrap();
    let mut context = device.create_context(&descriptor, None).unwrap();
    device.make_context_current(&context).expect("Could not make the context current");
    let native_widget = connection.create_native_widget_from_winit_window(&window).unwrap();
    let surface = device.create_surface(&context, SurfaceAccess::GPUCPU, SurfaceType::Widget{native_widget}).unwrap();
    device.bind_surface_to_context(&mut context, surface);



    Context {
        window,
        connection,
        adapter,
        device,
        context,
    }
}
