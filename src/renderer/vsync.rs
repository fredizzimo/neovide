use crate::profiling::tracy_zone;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{spawn, JoinHandle},
};
use winapi::um::dwmapi::DwmFlush;

pub struct VSync {
    should_exit: Arc<AtomicBool>,
    vsync_thread: Option<JoinHandle<()>>,
    vsync_count: Arc<(Mutex<usize>, Condvar)>,
    last_vsync: usize,
}

impl VSync {
    pub fn new() -> Self {
        let should_exit = Arc::new(AtomicBool::new(false));
        let should_exit2 = should_exit.clone();
        let vsync_count = Arc::new((Mutex::new(0), Condvar::new()));
        let vsync_count2 = vsync_count.clone();

        let vsync_thread = Some(spawn(move || {
            let (lock, cvar) = &*vsync_count2;
            while should_exit2.load(Ordering::SeqCst) == false {
                unsafe {
                    tracy_zone!("VSyncThread");
                    DwmFlush();
                    {
                        let mut count = lock.lock().unwrap();
                        *count += 1;
                        cvar.notify_one();
                    }
                }
            }
        }));

        VSync {
            should_exit,
            vsync_thread,
            vsync_count,
            last_vsync: 0,
        }
    }

    pub fn wait_for_vsync(&mut self) {
        let (lock, cvar) = &*self.vsync_count;
        let count = cvar
            .wait_while(lock.lock().unwrap(), |count| *count < self.last_vsync + 2)
            .unwrap();
        self.last_vsync = *count;
    }
}

impl Drop for VSync {
    fn drop(&mut self) {
        self.should_exit.store(true, Ordering::SeqCst);
        self.vsync_thread.take().unwrap().join().unwrap();
    }
}
