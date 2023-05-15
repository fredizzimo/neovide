use crate::profiling::tracy_zone;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{spawn, JoinHandle, sleep},
    time::{Instant, Duration}
};
use winit::window::Window;
use raw_window_handle::HasRawWindowHandle;
use glutin::{
    config::Config,
    context::{ContextAttributesBuilder, GlProfile, PossiblyCurrentContext, NotCurrentContext},
};

use crate::cmd_line::CmdLineSettings;
use crate::renderer::build_offscreen_context;

#[cfg(target_os = "windows")]
use winapi::um::dwmapi::DwmFlush;

use neovide_gl_bindings::glx::WaitVideoSyncSGI;


use simple_moving_average::{NoSumSMA, SMA};

use super::opengl::OffscreenContext;

pub struct VSync {
    should_exit: Arc<AtomicBool>,
    vsync_thread: Option<JoinHandle<()>>,
    vsync_count: Arc<(Mutex<(usize, f64)>, Condvar)>,
    last_vsync: usize,
    dt: f64,
    interval: usize,
}

#[cfg(target_os = "windows")]
fn wait() {
    unsafe {
        DwmFlush();
    }
}


#[cfg(target_os = "linux")]
fn wait() {
    //sleep(Duration::from_secs_f64(1.0 / 60.0));
    let mut count = 0;
    unsafe {
        WaitVideoSyncSGI(1, 0, &mut count);
    }
}

impl VSync {
    pub fn new(window: &Window, config: Config, cmd_line_settings: &CmdLineSettings) -> Self {
        let should_exit = Arc::new(AtomicBool::new(false));
        let should_exit2 = should_exit.clone();
        let vsync_count = Arc::new((Mutex::new((0, 0.0)), Condvar::new()));
        let vsync_count2 = vsync_count.clone();

        let vsync_thread = Some(spawn(move || {
            let context = build_offscreen_context(config);
            WaitVideoSyncSGI::load_with(|s| context.get_proc_address(s) as *const _);
            let mut frame_dt_avg = NoSumSMA::<f64, f64, 10>::new();
            let mut prev_frame_start = Instant::now();

            let (lock, cvar) = &*vsync_count2;
            while !should_exit2.load(Ordering::SeqCst) {
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
        }));

        VSync {
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
}

impl Drop for VSync {
    fn drop(&mut self) {
        self.should_exit.store(true, Ordering::SeqCst);
        self.vsync_thread.take().unwrap().join().unwrap();
    }
}
