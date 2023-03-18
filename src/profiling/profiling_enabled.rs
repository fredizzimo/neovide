use std::cell::RefCell;
use std::ptr::null;

use once_cell::unsync::OnceCell;

use tracy_client_sys::{
    ___tracy_c_zone_context, ___tracy_connected, ___tracy_emit_frame_mark,
    ___tracy_emit_zone_begin, ___tracy_emit_zone_end, ___tracy_source_location_data,
    ___tracy_startup_profiler,
};

use crate::renderer::SkiaRenderer;

pub struct _LocationData {
    pub data: ___tracy_source_location_data,
}

unsafe impl Send for _LocationData {}
unsafe impl Sync for _LocationData {}

#[allow(unconditional_panic)]
const fn illegal_null_in_string() {
    [][0]
}

#[doc(hidden)]
pub const fn validate_cstr_contents(bytes: &[u8]) {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\0' {
            illegal_null_in_string();
        }
        i += 1;
    }
}

macro_rules! cstr {
    ( $s:literal ) => {{
        $crate::profiling::validate_cstr_contents($s.as_bytes());
        unsafe { std::mem::transmute::<_, &std::ffi::CStr>(concat!($s, "\0")) }
    }};
}

macro_rules! file_cstr {
    ( ) => {{
        unsafe { std::mem::transmute::<_, &std::ffi::CStr>(concat!(std::file!(), "\0")) }
    }};
}

pub const fn _create_location_data(
    name: &std::ffi::CStr,
    function: &std::ffi::CStr,
    file: &std::ffi::CStr,
    line: u32,
    color: u32,
) -> _LocationData {
    _LocationData {
        data: ___tracy_source_location_data {
            name: name.as_ptr(),
            function: function.as_ptr(),
            file: file.as_ptr(),
            line,
            color,
        },
    }
}

#[allow(dead_code)]
fn is_connected() -> bool {
    unsafe { ___tracy_connected() > 0 }
}

#[cfg(feature = "gpu_profiling")]
fn gpu_enabled() -> bool {
    is_connected()
}

#[cfg(not(feature = "gpu_profiling"))]
fn gpu_enabled() -> bool {
    false
}

pub struct _Zone {
    context: ___tracy_c_zone_context,
    gpu: bool,
}

impl _Zone {
    pub fn new(loc_data: &___tracy_source_location_data, gpu: bool) -> Self {
        let context = unsafe { ___tracy_emit_zone_begin(loc_data, 1) };
        let gpu = gpu && gpu_enabled();
        if gpu {
            gpu_begin(loc_data);
        }
        _Zone { context, gpu }
    }
}

impl Drop for _Zone {
    fn drop(&mut self) {
        if self.gpu && gpu_enabled() {
            gpu_end();
        }
        unsafe {
            ___tracy_emit_zone_end(self.context);
        }
    }
}

pub trait GpuCtx {
    fn gpu_collect(&mut self);
    fn gpu_begin(&mut self, loc_data: &___tracy_source_location_data);
    fn gpu_end(&mut self);
}

thread_local! {
    static GPUCTX: OnceCell<RefCell<Box<dyn GpuCtx>>> = OnceCell::new();
}

pub fn startup_profiler() {
    unsafe {
        ___tracy_startup_profiler();
    }
}

#[cfg(not(feature = "gpu_profiling"))]
pub fn tracy_create_gpu_context(_name: &str, _skia_renderer: &dyn SkiaRenderer) {}

#[cfg(feature = "gpu_profiling")]
pub fn tracy_create_gpu_context(name: &str, skia_renderer: &dyn SkiaRenderer) {
    let context = skia_renderer.tracy_create_gpu_context(name);
    GPUCTX.with(|ctx| {
        ctx.set(RefCell::new(context)).unwrap_or_else(|_| {
            panic!("tracy_create_gpu_context can only be called once per thread")
        });
    });
}

pub fn tracy_gpu_collect() {
    tracy_zone!("collect gpu info");
    GPUCTX.with(|ctx| {
        ctx.get()
            .expect("Profiling context not initialized for current thread")
            .borrow_mut()
            .gpu_collect();
    });
}

fn gpu_begin(loc_data: &___tracy_source_location_data) {
    GPUCTX.with(|ctx| {
        ctx.get()
            .expect("Profiling context not initialized for current thread")
            .borrow_mut()
            .gpu_begin(loc_data);
    });
}

fn gpu_end() {
    GPUCTX.with(|ctx| {
        ctx.get()
            .expect("Profiling context not initialized for current thread")
            .borrow_mut()
            .gpu_end();
    });
}

#[inline(always)]
pub fn emit_frame_mark() {
    unsafe {
        ___tracy_emit_frame_mark(null());
    }
}

#[macro_export]
macro_rules! location_data {
    ($name: expr, $color: expr) => {{
        static LOC: $crate::profiling::_LocationData = $crate::profiling::_create_location_data(
            $crate::profiling::cstr!($name),
            // There does not seem to be any way of getting a c string to the current
            // function until this is implemented
            // https://github.com/rust-lang/rust/issues/63084
            // So use Unknown for now
            $crate::profiling::cstr!("Unknown"),
            $crate::profiling::file_cstr!(),
            std::line!(),
            $color,
        );
        &LOC.data
    }};
}

#[macro_export]
macro_rules! tracy_zone {
    ($name: expr, $color: expr) => {
        let _tracy_zone =
            $crate::profiling::_Zone::new($crate::profiling::location_data!($name, $color), false);
    };
    ($name: expr) => {
        $crate::profiling::tracy_zone!($name, 0)
    };
}

#[macro_export]
macro_rules! tracy_gpu_zone {
    ($name: expr, $color: expr) => {
        let _tracy_zone =
            $crate::profiling::_Zone::new($crate::profiling::location_data!($name, $color), true);
    };
    ($name: expr) => {
        $crate::profiling::tracy_gpu_zone!($name, 0)
    };
}

pub(crate) use cstr;
pub(crate) use file_cstr;
pub(crate) use location_data;
pub(crate) use tracy_gpu_zone;
pub(crate) use tracy_zone;
