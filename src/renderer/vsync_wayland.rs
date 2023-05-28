use std::sync::mpsc::{
    channel,
    Sender,
    Receiver
};
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{spawn, JoinHandle},
};
use std::time::Duration;

use super::{WindowedContext, VSync};
use winit::platform::wayland::WindowExtWayland;


use wayland_client::{
    Dispatch,
    Connection,
    Proxy,
    EventQueue,
    QueueHandle,
    protocol::wl_surface::WlSurface,
    protocol::wl_callback::WlCallback,
    backend::ObjectId,
};
use wayland_sys::client::{
    wl_proxy,
    wl_display,
};

use wayland_backend::sys::client::Backend;


struct VSyncDispatcher {
    vsync_sender: Sender<()>,
    vsync_signaled: Arc<(Mutex<bool>, Condvar)>,
}


pub struct VSyncWayland {
    wl_surface: WlSurface,
    event_queue_handle: QueueHandle<VSyncDispatcher>,
    vsync_receiver: Receiver<()>,
    should_exit: Arc<AtomicBool>,
    vsync_thread: Option<JoinHandle<()>>,

    vsync_signaled: Arc<(Mutex<bool>, Condvar)>,
}

impl VSyncWayland {
    pub fn new(vsync_enabled: bool, context: &WindowedContext) -> Self {
        let window = context.window();

        let surface = window
            .wayland_surface()
            .expect("Failed to get the wayland surface of the window")
            as *mut wl_proxy;

        let interface = WlSurface::interface();

        let id = unsafe {
            ObjectId::from_ptr(&interface, surface)
        }.expect("Failed to get wayland surface id");

        let display = window.wayland_display()
            .expect("Failed to get the wayland display of the window")
            as *mut wl_display;

        let backend = unsafe {
            Backend::from_foreign_display(display)
        };

        let conn = Connection::from_backend(backend);

        let mut event_queue = conn.new_event_queue::<VSyncDispatcher>();
        
        let wl_surface = <WlSurface as Proxy>::from_id(&conn, id).expect("Failed to create wl_surface proxy");

        let (vsync_sender, vsync_receiver) = channel();
        let vsync_signaled = Arc::new((Mutex::new(false), Condvar::new()));

        let mut dispatcher = VSyncDispatcher {
            vsync_sender,
            vsync_signaled: vsync_signaled.clone(),
        };
        let event_queue_handle = event_queue.handle();

        let should_exit = Arc::new(AtomicBool::new(false));
        let should_exit2 = should_exit.clone();
        let vsync_thread = Some(spawn(move || {
            while !should_exit2.load(Ordering::SeqCst) {
                event_queue.blocking_dispatch(&mut dispatcher);
            }
        }));



        Self {
            wl_surface,
            event_queue_handle,
            vsync_receiver,
            should_exit,
            vsync_thread,
            vsync_signaled,
        }
    }

    pub fn wait_for_vsync(&mut self) {
        let duration = Duration::from_millis(100);
        let (lock, cvar) = &*self.vsync_signaled;
        {
            *lock.lock().unwrap() = false;
        }
        let _callback = self.wl_surface.frame(&self.event_queue_handle, ());

        let _ = cvar
            .wait_timeout_while(lock.lock().unwrap(), duration, |signaled| {
                !*signaled
            })
            .unwrap();


        {
            *lock.lock().unwrap() = false;
        }
        let _callback = self.wl_surface.frame(&self.event_queue_handle, ());

        let _ = cvar
            .wait_timeout_while(lock.lock().unwrap(), duration, |signaled| {
                !*signaled
            })
            .unwrap();
    }

    pub fn set_refresh_rate(&mut self, desired_rate: u64) {}

    pub fn notify_frame_duration(&mut self, context: &WindowedContext, duration: f64) {}
}

impl Dispatch<WlCallback, ()> for VSyncDispatcher {
    fn event(
        state: &mut Self,
        _proxy: &WlCallback,
        _event: <WlCallback as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {

        let (lock, cvar) = &*state.vsync_signaled;
        let mut signaled = lock.lock().unwrap();
        *signaled = true;
        cvar.notify_one();
        state.vsync_sender.send(()).unwrap();
    }
}

impl Drop for VSyncWayland {
    fn drop(&mut self) {
        self.should_exit.store(true, Ordering::SeqCst);
        self.vsync_thread.take().unwrap().join().unwrap();
    }
}
