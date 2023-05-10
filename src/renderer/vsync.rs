use crate::profiling::tracy_zone;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{spawn, JoinHandle},
    time::{Duration, Instant},
};
use winapi::{
    shared::{
        dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput},
        guiddef::REFIID,
        minwindef::FALSE,
        ntdef::NULL,
        windef::HWND,
        winerror::SUCCEEDED,
    },
    um::dwmapi::{DwmFlush, DwmGetCompositionTimingInfo, DwmIsCompositionEnabled, DWM_TIMING_INFO},
    Interface,
};

fn get_composition_timing_info() -> Option<DWM_TIMING_INFO> {
    let mut composition_enabled = FALSE;
    unsafe {
        DwmIsCompositionEnabled(&mut composition_enabled);
    }
    let composition_enabled = composition_enabled != FALSE;
    if composition_enabled {
        let mut timing_info = DWM_TIMING_INFO {
            cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
            ..Default::default()
        };
        let res = unsafe { DwmGetCompositionTimingInfo(NULL as HWND, &mut timing_info) };
        if SUCCEEDED(res) {
            Some(timing_info)
        } else {
            None
        }
    } else {
        None
    }
}

fn get_vsync_interval() -> Duration {
    if let Some(timing_info) = get_composition_timing_info() {
        let rate = timing_info.rateRefresh;
        let refresh_rate = rate.uiDenominator as f64 / rate.uiNumerator as f64;
        Duration::from_secs_f64(refresh_rate)
    } else {
        Duration::from_secs_f64(1.0 / 60.0)
    }
}

fn create_dxgifactory() -> Option<*mut IDXGIFactory1> {
    let riid: REFIID = &IDXGIFactory1::uuidof();

    let mut factory: *mut IDXGIFactory1 = std::ptr::null_mut();

    let hr = unsafe { CreateDXGIFactory1(riid, &mut factory as *mut _ as *mut *mut _) };

    if SUCCEEDED(hr) {
        unsafe { Some(factory) }
    } else {
        None
    }
}

fn get_primary_adapter(factory: &Option<*mut IDXGIFactory1>) -> Option<*mut IDXGIAdapter1> {
    if let Some(factory) = *factory {
        let mut adapter: *mut IDXGIAdapter1 = std::ptr::null_mut();
        let hr = unsafe { (*factory).EnumAdapters1(0, &mut adapter) };
        if SUCCEEDED(hr) {
            unsafe { return Some(adapter) }
        }
    }
    None
}

fn get_primary_output(factory: &Option<*mut IDXGIFactory1>) -> Option<*mut IDXGIOutput> {
    let adapter = get_primary_adapter(&factory);
    if let Some(adapter) = adapter {
        let mut output: *mut IDXGIOutput = std::ptr::null_mut();
        let hr = unsafe {
            let hr = (*adapter).EnumOutputs(0, &mut output);
            (*adapter).Release();
            hr
        };
        if SUCCEEDED(hr) {
            unsafe { return Some(output) }
        }
    }
    None
}

/*
  void UpdateVBlankOutput() {
    HMONITOR primary_monitor =
        MonitorFromWindow(nullptr, MONITOR_DEFAULTTOPRIMARY);
    if (primary_monitor == mWaitVBlankMonitor && mWaitVBlankOutput) {
      return;
    }

    mWaitVBlankMonitor = primary_monitor;

    RefPtr<IDXGIOutput> output = nullptr;
    if (DeviceManagerDx* dx = DeviceManagerDx::Get()) {
      if (dx->GetOutputFromMonitor(mWaitVBlankMonitor, &output)) {
        mWaitVBlankOutput = output;
        return;
      }
    }

    // failed to convert a monitor to an output so keep trying
    mWaitVBlankOutput = nullptr;
  }
*/

pub struct VSync {
    //last_vsync: Instant,
    pub interval: Duration,
    last_refresh: usize,
    should_exit: Arc<AtomicBool>,
    vsync_thread: Option<JoinHandle<()>>,
    vsync_count: Arc<(Mutex<usize>, Condvar)>,
    last_vsync: usize,
}

impl VSync {
    pub fn new() -> Self {
        let interval = get_vsync_interval();
        let should_exit = Arc::new(AtomicBool::new(false));
        let should_exit2 = should_exit.clone();
        let vsync_count = Arc::new((Mutex::new(0), Condvar::new()));
        let vsync_count2 = vsync_count.clone();

        let vsync_thread = Some(spawn(move || {
            let dxgi_factory = create_dxgifactory();
            let primary_output = get_primary_output(&dxgi_factory);
            let (lock, cvar) = &*vsync_count2;
            let output = primary_output.unwrap();
            while should_exit2.load(Ordering::SeqCst) == false {
                unsafe {
                    tracy_zone!("VSyncThread");
                    //(*output).WaitForVBlank();
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
            interval,
            //last_vsync: Instant::now(),
            last_refresh: 0,
            should_exit,
            vsync_thread,
            vsync_count,
            last_vsync: 0,
        }
    }

    pub fn wait_for_vsync(&mut self) {
        let (lock, cvar) = &*self.vsync_count;
        // As long as the value inside the `Mutex<bool>` is `true`, we wait.
        let count = cvar
            .wait_while(lock.lock().unwrap(), |count| *count < self.last_vsync + 2)
            .unwrap();
        self.last_vsync = *count;
        /*
        let mut timing_info = get_composition_timing_info().unwrap();

        let elapsed = self.last_vsync.elapsed();
        let elapsed_before = elapsed.as_micros();
        let mut max: i32 = 3;
        while (timing_info.cRefresh as usize) <= (self.last_refresh + 2)  && max > 0 {
            if let Some(output) = self.primary_output {
                unsafe {
                    tracy_zone!("wait for vblank");
                    (*output).WaitForVBlank();
                }
            }
            timing_info = get_composition_timing_info().unwrap();
            max -= 1
        }
        */
        //self.last_refresh = timing_info.cRefresh as usize;
        /*
        if true || elapsed < self.interval {
            if let Some(output) = self.primary_output {
                unsafe {
                    //DwmFlush();
                    //DwmFlush();

                    (*output).WaitForVBlank();
                    let timing_info2 = get_composition_timing_info();
                    let elapsed = self.last_vsync.elapsed().as_micros();
                    let long = if elapsed > 8500 {
                        "long"
                    } else {
                        ""
                    };
                    log::trace!("{} frame time {} {}", long, elapsed_before, elapsed);

                }
            } else {
                unsafe {
                    DwmFlush();
                }
            }
        }
        */

        //self.last_vsync = Instant::now();
    }
}

impl Drop for VSync {
    fn drop(&mut self) {
        self.should_exit.store(true, Ordering::SeqCst);
        self.vsync_thread.take().unwrap().join().unwrap();
    }
}

/*
  // Returns the timestamp for the just happened vsync
  TimeStamp GetVBlankTime() {
    TimeStamp vsync = TimeStamp::Now();
    TimeStamp now = vsync;

    DWM_TIMING_INFO vblankTime;
    // Make sure to init the cbSize, otherwise
    // GetCompositionTiming will fail
    vblankTime.cbSize = sizeof(DWM_TIMING_INFO);
    HRESULT hr = DwmGetCompositionTimingInfo(0, &vblankTime);
    if (!SUCCEEDED(hr)) {
      return vsync;
    }

    LARGE_INTEGER frequency;
    QueryPerformanceFrequency(&frequency);

    LARGE_INTEGER qpcNow;
    QueryPerformanceCounter(&qpcNow);

    const int microseconds = 1000000;
    int64_t adjust = qpcNow.QuadPart - vblankTime.qpcVBlank;
    int64_t usAdjust = (adjust * microseconds) / frequency.QuadPart;
    vsync -= TimeDuration::FromMicroseconds((double)usAdjust);

    if (IsWin10OrLater()) {
      // On Windows 10 and on, DWMGetCompositionTimingInfo, mostly
      // reports the upcoming vsync time, which is in the future.
      // It can also sometimes report a vblank time in the past.
      // Since large parts of Gecko assume TimeStamps can't be in future,
      // use the previous vsync.

      // Windows 10 and Intel HD vsync timestamps are messy and
      // all over the place once in a while. Most of the time,
      // it reports the upcoming vsync. Sometimes, that upcoming
      // vsync is in the past. Sometimes that upcoming vsync is before
      // the previously seen vsync.
      // In these error cases, normalize to Now();
      if (vsync >= now) {
        vsync = vsync - mVsyncRate;
      }
    }

    // On Windows 7 and 8, DwmFlush wakes up AFTER qpcVBlankTime
    // from DWMGetCompositionTimingInfo. We can return the adjusted vsync.
    if (vsync >= now) {
      vsync = now;
    }

    // Our vsync time is some time very far in the past, adjust to Now.
    // 4 ms is arbitrary, so feel free to pick something else if this isn't
    // working. See the comment above within IsWin10OrLater().
    if ((now - vsync).ToMilliseconds() > 4.0) {
      vsync = now;
    }

    return vsync;
  }
*/

/*
+#[cfg(target_os = "windows")]
+use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput};
+
 use crate::profiling::{
     emit_frame_mark, tracy_create_gpu_context, tracy_gpu_collect, tracy_gpu_zone, tracy_zone,
 };
@@ -83,6 +86,8 @@ pub struct GlutinWindowWrapper {
     size_at_startup: PhysicalSize<u32>,
     maximized_at_startup: bool,
     window_command_receiver: UnboundedReceiver<WindowCommand>,
+    #[cfg(target_os = "windows")]
+    dxgi_output: IDXGIOutput,
 }

 impl GlutinWindowWrapper {
@@ -205,8 +210,24 @@ impl GlutinWindowWrapper {
     pub fn draw_frame(&mut self, dt: f32) {
         tracy_zone!("draw_frame");
         self.renderer.draw_frame(self.skia_renderer.canvas(), dt);
-        self.skia_renderer.gr_context.flush_and_submit();
-        self.windowed_context.swap_buffers().unwrap();
+        {
+            tracy_gpu_zone!("skia flush");
+            self.skia_renderer.gr_context.flush_and_submit();
+        }
+        #[cfg(target_os = "windows")]
+        unsafe {
+            tracy_gpu_zone!("WaitForVBlank");
+            self.dxgi_output.WaitForVBlank();
+        }
+
+        {
+            // NOTE: a gpu zone here can force a sync with the GPU and block
+            // so use a CPU zone instead
+            tracy_zone!("swap buffers");
+            self.windowed_context.swap_buffers().unwrap();
+        }
+        tracy_gpu_collect();
+        emit_frame_mark();
     }

     pub fn animate_frame(&mut self, dt: f32, time: f64) -> bool {
@@ -487,6 +508,13 @@ pub fn create_window() {
             saved_inner_size,
             saved_grid_size: None,
             window_command_receiver,
+            #[cfg(target_os = "windows")]
+            dxgi_output: {
+                let dxgi_factory = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }.unwrap();
+                let primary_adapter = unsafe { dxgi_factory.EnumAdapters1(0) }.unwrap();
+                let primary_output = unsafe { primary_adapter.EnumOutputs(0) }.unwrap();
+                primary_output
+            },
         };

         tracy_create_gpu_context("main render context");
*/
