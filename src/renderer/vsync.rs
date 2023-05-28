use super::WindowedContext;
#[cfg(target_os = "linux")]
use std::env;

use super::vsync_opengl::VSyncOpengl;
#[cfg(target_os = "linux")]
use super::vsync_wayland::VSyncWayland;

#[cfg(target_os = "macos")]
pub type VSync = VSyncOpengl;

#[cfg(target_os = "linux")]
pub enum VSync {
    Opengl(VSyncOpengl),
    Wayland(VSyncWayland),
}

#[cfg(target_os = "linux")]
impl VSync {
    pub fn new(vsync_enabled: bool, context: &WindowedContext) -> Self {
        if env::var("WAYLAND_DISPLAY").is_ok() {
            VSync::Wayland(VSyncWayland::new(vsync_enabled, context))
        } else {
            VSync::Opengl(VSyncOpengl::new(vsync_enabled, context))
        }
    }

    pub fn wait_for_vsync(&mut self) {
        match self {
            VSync::Opengl(vsync) => vsync.wait_for_vsync(),
            VSync::Wayland(vsync) => vsync.wait_for_vsync(),
        }
    }

    pub fn set_refresh_rate(&mut self, desired_rate: u64) {
        if let VSync::Opengl(vsync) = self {
            vsync.set_refresh_rate(desired_rate);
        }
    }

    pub fn notify_frame_duration(&mut self, context: &WindowedContext, duration: f64) {
        if let VSync::Opengl(vsync) = self {
            vsync.notify_frame_duration(context, duration);
        }
    }
}
