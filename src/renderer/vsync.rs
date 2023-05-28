use simple_moving_average::{NoSumSMA, SMA};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{spawn, JoinHandle},
    time::Instant,
};

use crate::profiling::tracy_zone;

use super::WindowedContext;
#[cfg(target_os = "linux")]
use std::env;

use super::vsync_opengl::VSyncOpengl;
#[cfg(target_os = "linux")]
use super::vsync_wayland::VSyncWayland;

#[cfg(target_os = "macos")]
type VSync = VSyncOpengl;

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
        match self {
            VSync::Opengl(vsync) => vsync.set_refresh_rate(desired_rate),
            VSync::Wayland(vsync) => vsync.set_refresh_rate(desired_rate),
        }
    }

    pub fn notify_frame_duration(&mut self, context: &WindowedContext, duration: f64) {
        match self {
            VSync::Opengl(vsync) => vsync.notify_frame_duration(context, duration),
            VSync::Wayland(vsync) => vsync.notify_frame_duration(context, duration),
        }
    }
}

pub struct ThreadedVSync {
    should_exit: Arc<AtomicBool>,
    vsync_thread: Option<JoinHandle<()>>,
    vsync_count: Arc<(Mutex<(usize, f64)>, Condvar)>,
    last_vsync: usize,
    dt: f64,
    interval: usize,
}

impl ThreadedVSync {
    // On Windows the fake vsync is always enabled
    // Everything else is very jerky
    pub fn new<F>(wait: F) -> Self
    where
        F: Fn() -> (),
        F: Send + 'static,
    {
        let should_exit = Arc::new(AtomicBool::new(false));
        let should_exit2 = should_exit.clone();
        let vsync_count = Arc::new((Mutex::new((0, 0.0)), Condvar::new()));
        let vsync_count2 = vsync_count.clone();

        // When using opengl on Windows, in windowed mode, swap_buffers does not seem to be
        // syncrhonized with the Desktop Window Manager. So work around that by waiting until the
        // DWM is flushed before swapping the buffers.
        // Using a separate thread simplifies things, since it avoids race conditions when the
        // starting the wait just before the next flush is starting to happen.
        let vsync_thread = Some(spawn(move || {
            let mut frame_dt_avg = NoSumSMA::<f64, f64, 10>::new();
            let mut prev_frame_start = Instant::now();

            let (lock, cvar) = &*vsync_count2;
            while !should_exit2.load(Ordering::SeqCst) {
                unsafe {
                    tracy_zone!("VSyncThread");
                    wait();
                    frame_dt_avg.add_sample(prev_frame_start.elapsed().as_secs_f64());
                    prev_frame_start = Instant::now();
                    {
                        let mut count_dt = lock.lock().unwrap();
                        count_dt.0 += 1;
                        count_dt.1 = frame_dt_avg.get_average();
                        cvar.notify_one();
                    }
                }
            }
        }));

        Self {
            should_exit,
            vsync_thread,
            vsync_count,
            last_vsync: 0,
            dt: 0.0,
            interval: 1,
        }
    }

    pub fn wait_for_vsync(&mut self) {
        let (lock, cvar) = &*self.vsync_count;
        let count_dt = cvar
            .wait_while(lock.lock().unwrap(), |count_dt| {
                count_dt.0 < self.last_vsync + self.interval
            })
            .unwrap();
        self.last_vsync = count_dt.0;
        self.dt = count_dt.1;
    }

    pub fn set_refresh_rate(&mut self, desired_rate: u64) {
        if self.dt > 0.0 {
            let rate = 1.0 / self.dt;
            let desired_rate = (desired_rate).max(30) as f64;
            self.interval = (rate / desired_rate).round().max(1.0) as usize;
        } else {
            self.interval = 1;
        }
    }

    pub fn notify_frame_duration(&mut self, _context: &WindowedContext, _duration: f64) {}
}

impl Drop for ThreadedVSync {
    fn drop(&mut self) {
        self.should_exit.store(true, Ordering::SeqCst);
        self.vsync_thread.take().unwrap().join().unwrap();
    }
}
