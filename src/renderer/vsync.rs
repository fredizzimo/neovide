use super::WindowedContext;
use std::thread::sleep;
use std::time::{Duration, Instant};

enum SwapInterval {
    Interval(u32),
    Disabled,
}

const MAX_UNSYNCHRONIZED: i32 = 5;
const MAX_SWAP_INTERVAL: u32 = 10;

pub struct VSync {
    vsync_enabled: bool,

    desired_frame_duration: Option<f64>,
    current_swap_interval: SwapInterval,
    start_time: Instant,
    num_unsynchronized: i32,
}

impl VSync {
    pub fn new(vsync_enabled: bool) -> Self {
        let current_swap_interval = if vsync_enabled {
            SwapInterval::Interval(1)
        } else {
            SwapInterval::Disabled
        };
        let start_time = Instant::now();
        Self {
            vsync_enabled,
            desired_frame_duration: None,
            current_swap_interval,
            start_time,
            num_unsynchronized: 0,
        }
    }

    pub fn wait_for_vsync(&self) {
        if !matches!(self.current_swap_interval, SwapInterval::Disabled) {
            return;
        }
        let frame_duration = self.desired_frame_duration.unwrap();

        let current_time = self.start_time.elapsed().as_secs_f64();
        let current_interval = (current_time / frame_duration).floor();
        let next_time = (current_interval + 1.0) * frame_duration;
        let duration = Duration::from_secs_f64(next_time - current_time);
        sleep(duration);
    }

    pub fn set_refresh_rate(&mut self, desired_rate: u64) {
        let desired_frame_length = 1.0 / desired_rate as f64;
        self.desired_frame_duration = Some(desired_frame_length);
    }

    pub fn notify_frame_duration(&mut self, context: &WindowedContext, duration: f64) {
        if !self.vsync_enabled {
            self.current_swap_interval = SwapInterval::Disabled;
            return;
        }

        let mut current_swap_interval = match self.current_swap_interval {
            SwapInterval::Interval(interval) => interval,
            SwapInterval::Disabled => MAX_SWAP_INTERVAL,
        };
        let desired_duration = self.desired_frame_duration.unwrap();

        let estimated_refresh_interval = duration / current_swap_interval as f64;
        let estimated_swap_interval =
            (desired_duration / estimated_refresh_interval).round() as u32;
        if current_swap_interval == estimated_swap_interval {
            self.num_unsynchronized = 0;
            return;
        }

        if current_swap_interval < estimated_swap_interval {
            self.num_unsynchronized += 1;
        } else {
            self.num_unsynchronized -= 1;
        }
        if self.num_unsynchronized.abs() <= MAX_UNSYNCHRONIZED {
            return;
        }
        let num_unsynchronized = self.num_unsynchronized;
        self.num_unsynchronized = 0;

        if num_unsynchronized > MAX_UNSYNCHRONIZED {
            current_swap_interval += 1;
            if current_swap_interval > MAX_SWAP_INTERVAL {
                // The swap interval does not seem to do anything, so switch to a timer based
                // synchronization. The driver is probably overriding it. Leave the maximum swap
                // rate enabled on the context, so we can detect if the swap interval starts
                // working again.
                self.current_swap_interval = SwapInterval::Disabled;
                return;
            }
        } else if num_unsynchronized < -MAX_UNSYNCHRONIZED {
            if current_swap_interval == 1 {
                // Never attempt to go faster than the vsync when vsync is enabled
                return;
            }
            current_swap_interval -= 1;
        }
        current_swap_interval = current_swap_interval.clamp(1, MAX_SWAP_INTERVAL);
        context.set_swap_interval(current_swap_interval);
        self.current_swap_interval = SwapInterval::Interval(current_swap_interval);
    }
}
