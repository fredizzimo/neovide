mod keyboard_manager;
mod mouse_manager;
mod renderer;
mod settings;
mod update_loop;
mod window_wrapper;

#[cfg(target_os = "macos")]
mod draw_background;

#[cfg(target_os = "linux")]
use std::env;

use winit::{
    dpi::PhysicalSize,
    event_loop::{EventLoop, EventLoopBuilder},
    window::{Icon, WindowBuilder},
};

#[cfg(target_os = "macos")]
use winit::platform::macos::WindowBuilderExtMacOS;

#[cfg(target_os = "macos")]
use draw_background::draw_background;

#[cfg(target_os = "linux")]
use winit::platform::wayland::WindowBuilderExtWayland;
#[cfg(target_os = "linux")]
use winit::platform::x11::WindowBuilderExtX11;

#[cfg(target_os = "windows")]
use std::{
    sync::mpsc::{channel, RecvTimeoutError, TryRecvError},
    thread,
};
#[cfg(target_os = "windows")]
use winit::{
    event::{Event, WindowEvent},
    event_loop::ControlFlow,
};

use image::{load_from_memory, GenericImageView, Pixel};
use keyboard_manager::KeyboardManager;
use mouse_manager::MouseManager;
use renderer::SkiaRenderer;
use update_loop::UpdateLoop;
use window_wrapper::WinitWindowWrapper;

use crate::{
    cmd_line::CmdLineSettings,
    frame::Frame,
    renderer::{build_window, GlWindow},
    running_tracker::*,
    settings::{load_last_window_settings, save_window_size, PersistentWindowSettings, SETTINGS},
};
pub use settings::{KeyboardSettings, WindowSettings};

static ICON: &[u8] = include_bytes!("../../assets/neovide.ico");

#[derive(Clone, Debug)]
pub enum WindowCommand {
    TitleChanged(String),
    SetMouseEnabled(bool),
    ListAvailableFonts,
    Columns(u64),
    Lines(u64),
    UIEnter,
    UIReady,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum UserEvent {
    ScaleFactorChanged(f64),
}

pub fn create_event_loop() -> EventLoop<UserEvent> {
    EventLoopBuilder::<UserEvent>::with_user_event().build()
}

pub fn create_window(event_loop: &EventLoop<UserEvent>) -> GlWindow {
    let icon = {
        let icon = load_from_memory(ICON).expect("Failed to parse icon data");
        let (width, height) = icon.dimensions();
        let mut rgba = Vec::with_capacity((width * height) as usize * 4);
        for (_, _, pixel) in icon.pixels() {
            rgba.extend_from_slice(&pixel.to_rgba().0);
        }
        Icon::from_rgba(rgba, width, height).expect("Failed to create icon object")
    };

    let cmd_line_settings = SETTINGS.get::<CmdLineSettings>();

    let window_settings = load_last_window_settings();

    let previous_position = match window_settings {
        Ok(PersistentWindowSettings::Windowed { position, .. }) => Some(position),
        _ => None,
    };

    log::trace!("Settings geometry {:?}", cmd_line_settings.geometry,);
    log::trace!("Settings size {:?}", cmd_line_settings.size);

    let default_size = PhysicalSize {
        width: 500,
        height: 500,
    };
    let inner_size = if let Some(size) = cmd_line_settings.size {
        size.into()
    } else if cmd_line_settings.geometry.is_some() {
        default_size
    } else {
        match window_settings {
            Ok(PersistentWindowSettings::Windowed {
                pixel_size: Some(size),
                ..
            }) => size,
            _ => default_size,
        }
    };

    let winit_window_builder = WindowBuilder::new()
        .with_title("Neovide")
        .with_window_icon(Some(icon))
        .with_inner_size(inner_size)
        .with_maximized(false)
        .with_transparent(true)
        .with_visible(false);

    let frame_decoration = cmd_line_settings.frame;

    // There is only two options for windows & linux, no need to match more options.
    #[cfg(not(target_os = "macos"))]
    let mut winit_window_builder =
        winit_window_builder.with_decorations(frame_decoration == Frame::Full);

    #[cfg(target_os = "macos")]
    let mut winit_window_builder = match frame_decoration {
        Frame::Full => winit_window_builder,
        Frame::None => winit_window_builder.with_decorations(false),
        Frame::Buttonless => winit_window_builder
            .with_transparent(true)
            .with_title_hidden(true)
            .with_titlebar_buttons_hidden(true)
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true),
        Frame::Transparent => winit_window_builder
            .with_title_hidden(true)
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true),
    };

    if let Some(previous_position) = previous_position {
        winit_window_builder = winit_window_builder.with_position(previous_position);
    }

    #[cfg(target_os = "linux")]
    let winit_window_builder = {
        if env::var("WAYLAND_DISPLAY").is_ok() {
            let app_id = &cmd_line_settings.wayland_app_id;
            WindowBuilderExtWayland::with_name(winit_window_builder, "neovide", app_id.clone())
        } else {
            let class = &cmd_line_settings.x11_wm_class;
            let instance = &cmd_line_settings.x11_wm_class_instance;
            WindowBuilderExtX11::with_name(winit_window_builder, class, instance)
        }
    };

    #[cfg(target_os = "macos")]
    let winit_window_builder = winit_window_builder.with_accepts_first_mouse(false);

    let gl_window = build_window(winit_window_builder, event_loop);
    let window = &gl_window.window;

    // Check that window is visible in some monitor, and reposition it if not.
    window.current_monitor().and_then(|current_monitor| {
        let monitor_position = current_monitor.position();
        let monitor_size = current_monitor.size();
        let monitor_width = monitor_size.width as i32;
        let monitor_height = monitor_size.height as i32;

        let window_position = previous_position.or_else(|| window.outer_position().ok())?;

        let window_size = window.outer_size();
        let window_width = window_size.width as i32;
        let window_height = window_size.height as i32;

        if window_position.x + window_width < monitor_position.x
            || window_position.y + window_height < monitor_position.y
            || window_position.x > monitor_position.x + monitor_width
            || window_position.y > monitor_position.y + monitor_height
        {
            window.set_outer_position(monitor_position);
        }

        Some(())
    });

    gl_window
}

// Use a render thread on Windows to work around performance issues with Winit
// see: https://github.com/rust-windowing/winit/issues/2782
#[cfg(target_os = "windows")]
pub fn main_loop(window: GlWindow, event_loop: EventLoop<UserEvent>) {
    let cmd_line_settings = SETTINGS.get::<CmdLineSettings>();
    let (txtemp, rx) = channel::<Event<UserEvent>>();
    let mut tx = Some(txtemp);
    let mut render_thread_handle = Some(thread::spawn(move || {
        let mut window_wrapper = WinitWindowWrapper::new(window);
        let mut update_loop = UpdateLoop::new(
            cmd_line_settings.vsync,
            cmd_line_settings.idle,
            &window_wrapper.windowed_context,
        );
        #[allow(unused_assignments)]
        loop {
            let (wait_duration, _) = update_loop.get_event_wait_time();
            let event = if wait_duration.as_nanos() == 0 {
                rx.try_recv()
                    .map_err(|e| matches!(e, TryRecvError::Disconnected))
            } else {
                rx.recv_timeout(wait_duration)
                    .map_err(|e| matches!(e, RecvTimeoutError::Disconnected))
            };

            if update_loop.step(&mut window_wrapper, event).is_err() {
                break;
            }
        }
        let window = window_wrapper.windowed_context.window();
        save_window_size(
            window.is_maximized(),
            window.inner_size(),
            window.outer_position().ok(),
        );
        std::process::exit(RUNNING_TRACKER.exit_code());
    }));

    event_loop.run(move |e, _window_target, control_flow| {
        let e = match e {
            Event::WindowEvent {
                event: WindowEvent::ScaleFactorChanged { scale_factor, .. },
                ..
            } => {
                // It's really unfortunate that we have to do this, but
                // https://github.com/rust-windowing/winit/issues/1387
                Some(Event::UserEvent(UserEvent::ScaleFactorChanged(
                    scale_factor,
                )))
            }
            Event::MainEventsCleared => None,
            _ => {
                // With the current Winit version, all events, except ScaleFactorChanged are static
                Some(e.to_static().expect("Unexpected event received"))
            }
        };
        if let Some(e) = e {
            let _ = tx.as_ref().unwrap().send(e);
        }

        if !RUNNING_TRACKER.is_running() {
            let tx = tx.take().unwrap();
            drop(tx);
            let handle = render_thread_handle.take().unwrap();
            handle.join().unwrap();
        }
        // We need to wake up regularly to check the running tracker, so that we can exit
        *control_flow = ControlFlow::WaitUntil(
            std::time::Instant::now() + std::time::Duration::from_millis(100),
        );
    });
}

#[cfg(not(target_os = "windows"))]
pub fn main_loop(window: GlWindow, event_loop: EventLoop<UserEvent>) {
    let cmd_line_settings = SETTINGS.get::<CmdLineSettings>();
    let mut window_wrapper = WinitWindowWrapper::new(window);

    let mut update_loop = UpdateLoop::new(
        cmd_line_settings.vsync,
        cmd_line_settings.idle,
        &window_wrapper.windowed_context,
    );

    event_loop.run(move |e, _window_target, control_flow| {
        *control_flow = update_loop.step(&mut window_wrapper, Ok(e)).unwrap();

        if !RUNNING_TRACKER.is_running() {
            let window = window_wrapper.windowed_context.window();
            save_window_size(
                window.is_maximized(),
                window.inner_size(),
                window.outer_position().ok(),
            );

            std::process::exit(RUNNING_TRACKER.exit_code());
        }
    });
}
