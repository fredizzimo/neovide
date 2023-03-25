mod keyboard_manager;
mod mouse_manager;
mod renderer;
mod settings;

#[cfg(target_os = "macos")]
mod draw_background;

use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use log::trace;
use simple_moving_average::{NoSumSMA, SMA};
use tokio::sync::mpsc::UnboundedReceiver;
use winit::{
    dpi::PhysicalSize,
    event::{self, Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{self, Fullscreen, Icon, Window},
};

#[cfg(target_os = "macos")]
use winit::platform::macos::WindowBuilderExtMacOS;

#[cfg(target_os = "macos")]
use draw_background::draw_background;

#[cfg(target_os = "linux")]
use winit::platform::unix::WindowBuilderExtUnix;

use crate::profiling::{
    emit_frame_mark, tracy_create_gpu_context, tracy_gpu_collect, tracy_gpu_zone, tracy_zone,
};

use image::{load_from_memory, GenericImageView, Pixel};
use keyboard_manager::KeyboardManager;
use mouse_manager::MouseManager;
use renderer::WGpuRenderer;

use crate::{
    bridge::{ParallelCommand, UiCommand},
    cmd_line::CmdLineSettings,
    dimensions::Dimensions,
    editor::EditorCommand,
    event_aggregator::EVENT_AGGREGATOR,
    frame::Frame,
    renderer::Renderer,
    renderer::WindowPadding,
    //renderer::{build_context, WindowedContext},
    running_tracker::*,
    settings::{
        load_last_window_settings, save_window_geometry, PersistentWindowSettings, SETTINGS,
    },
};
pub use settings::{KeyboardSettings, WindowSettings};

static ICON: &[u8] = include_bytes!("../../assets/neovide.ico");

const MIN_WINDOW_WIDTH: u64 = 20;
const MIN_WINDOW_HEIGHT: u64 = 6;

#[derive(Clone, Debug)]
pub enum WindowCommand {
    TitleChanged(String),
    SetMouseEnabled(bool),
    ListAvailableFonts,
}

#[derive(Clone, Debug)]
pub enum UserEvent {
    ScaleFactorChanged(f64),
}

pub struct WinitWindowWrapper {
    //windowed_context: WindowedContext,
    wgpu_renderer: WGpuRenderer,
    window: Window,
    renderer: Renderer,
    keyboard_manager: KeyboardManager,
    mouse_manager: MouseManager,
    title: String,
    fullscreen: bool,
    font_changed_last_frame: bool,
    saved_inner_size: PhysicalSize<u32>,
    saved_grid_size: Option<Dimensions>,
    size_at_startup: PhysicalSize<u32>,
    maximized_at_startup: bool,
    window_command_receiver: UnboundedReceiver<WindowCommand>,
}

impl WinitWindowWrapper {
    pub fn toggle_fullscreen(&mut self) {
        let window = &self.window;
        if self.fullscreen {
            window.set_fullscreen(None);
        } else {
            let handle = window.current_monitor();
            window.set_fullscreen(Some(Fullscreen::Borderless(handle)));
        }

        self.fullscreen = !self.fullscreen;
    }

    pub fn synchronize_settings(&mut self) {
        let fullscreen = { SETTINGS.get::<WindowSettings>().fullscreen };

        if self.fullscreen != fullscreen {
            self.toggle_fullscreen();
        }
    }

    #[allow(clippy::needless_collect)]
    pub fn handle_window_commands(&mut self) {
        tracy_zone!("handle_window_commands", 0);
        while let Ok(window_command) = self.window_command_receiver.try_recv() {
            match window_command {
                WindowCommand::TitleChanged(new_title) => self.handle_title_changed(new_title),
                WindowCommand::SetMouseEnabled(mouse_enabled) => {
                    self.mouse_manager.enabled = mouse_enabled
                }
                WindowCommand::ListAvailableFonts => self.send_font_names(),
            }
        }
    }

    pub fn handle_title_changed(&mut self, new_title: String) {
        self.title = new_title;
        self.window.set_title(&self.title);
    }

    pub fn send_font_names(&self) {
        let font_names = self.renderer.font_names();
        EVENT_AGGREGATOR.send(UiCommand::Parallel(ParallelCommand::DisplayAvailableFonts(
            font_names,
        )));
    }

    pub fn handle_quit(&mut self) {
        if SETTINGS.get::<CmdLineSettings>().remote_tcp.is_none() {
            EVENT_AGGREGATOR.send(UiCommand::Parallel(ParallelCommand::Quit));
        } else {
            RUNNING_TRACKER.quit("window closed");
        }
    }

    pub fn handle_focus_lost(&mut self) {
        EVENT_AGGREGATOR.send(UiCommand::Parallel(ParallelCommand::FocusLost));
    }

    pub fn handle_focus_gained(&mut self) {
        EVENT_AGGREGATOR.send(UiCommand::Parallel(ParallelCommand::FocusGained));
    }

    pub fn handle_event(&mut self, event: Event<UserEvent>) -> bool {
        tracy_zone!("handle_event", 0);
        let mut should_render = false;
        self.keyboard_manager.handle_event(&event);
        self.mouse_manager.handle_event(
            &event,
            &self.keyboard_manager,
            &self.renderer,
            &self.window,
        );
        self.renderer.handle_event(&event);
        match event {
            Event::LoopDestroyed => {
                self.handle_quit();
            }
            Event::Resumed => {
                EVENT_AGGREGATOR.send(EditorCommand::RedrawScreen);
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                self.handle_quit();
            }
            Event::UserEvent(UserEvent::ScaleFactorChanged(scale_factor)) => {
                self.handle_scale_factor_update(scale_factor);
            }
            Event::WindowEvent {
                event: WindowEvent::DroppedFile(path),
                ..
            } => {
                let file_path = path.into_os_string().into_string().unwrap();
                EVENT_AGGREGATOR.send(UiCommand::Parallel(ParallelCommand::FileDrop(file_path)));
            }
            Event::WindowEvent {
                event: WindowEvent::Focused(focus),
                ..
            } => {
                if focus {
                    self.handle_focus_gained();
                    should_render = true;
                } else {
                    self.handle_focus_lost();
                }
            }
            Event::RedrawRequested(..) | Event::WindowEvent { .. } => {
                should_render = true;
            }
            _ => {}
        }
        should_render
    }

    pub fn draw_frame(&mut self, dt: f32) {
        tracy_zone!("draw_frame");
        /*
        {
            tracy_gpu_zone!("draw window surfaces");
            self.renderer.draw_window_surfaces();
            self.skia_renderer.gr_context.flush_and_submit();
        }

        {
            tracy_gpu_zone!("skia clear");
            let default_background = self.renderer.grid_renderer.get_default_background();
            self.skia_renderer.canvas().clear(default_background);
            self.skia_renderer.gr_context.flush_and_submit();
        }

        self.renderer.draw_frame(self.skia_renderer.canvas(), dt);
        {
            tracy_gpu_zone!("skia flush");
            self.skia_renderer.gr_context.flush_and_submit();
        }
        {
            tracy_gpu_zone!("swap buffers");
            self.windowed_context.swap_buffers().unwrap();
        }
        emit_frame_mark();
        tracy_gpu_collect();
        */
    }

    pub fn animate_frame(&mut self, dt: f32) -> bool {
        tracy_zone!("animate_frame", 0);
        self.renderer.animate_frame(dt)
    }

    pub fn prepare_frame(&mut self) -> bool {
        tracy_zone!("prepare_frame", 0);
        let mut should_render = false;

        let window = &self.window;
        let new_size = window.inner_size();

        let window_settings = SETTINGS.get::<WindowSettings>();
        let window_padding = WindowPadding {
            top: window_settings.padding_top,
            left: window_settings.padding_left,
            right: window_settings.padding_right,
            bottom: window_settings.padding_bottom,
        };

        let padding_changed = window_padding != self.renderer.window_padding;
        if padding_changed {
            self.renderer.window_padding = window_padding;
        }

        if self.saved_inner_size != new_size || self.font_changed_last_frame || padding_changed {
            self.font_changed_last_frame = false;
            self.saved_inner_size = new_size;

            self.handle_new_grid_size(new_size);
            self.wgpu_renderer.resize(&self.window);
            should_render = true;
        }

        self.font_changed_last_frame = self.renderer.handle_draw_commands();

        // Wait until fonts are loaded, so we can set proper window size.
        if !self.renderer.grid_renderer.is_ready {
            return false;
        }

        let settings = SETTINGS.get::<CmdLineSettings>();
        // Resize at startup happens when window is maximized or when using tiling WM
        // which already resized window.
        let resized_at_startup = self.maximized_at_startup || self.has_been_resized();

        /*
        log::trace!(
            "Settings geometry {:?}",
            PhysicalSize::from(settings.geometry)
        );
        log::trace!("Inner size: {:?}", new_size);
        */

        if self.saved_grid_size.is_none() && !resized_at_startup {
            let window = &self.window;
            window.set_inner_size(
                self.renderer
                    .grid_renderer
                    .convert_grid_to_physical(settings.geometry),
            );
            self.saved_grid_size = Some(settings.geometry);
            // Font change at startup is ignored, so grid size (and startup screen) could be preserved.
            // But only when not resized yet. With maximized or resized window we should redraw grid.
            self.font_changed_last_frame = false;
        }
        should_render
    }

    fn handle_new_grid_size(&mut self, new_size: PhysicalSize<u32>) {
        let window_padding = self.renderer.window_padding;
        let window_padding_width = window_padding.left + window_padding.right;
        let window_padding_height = window_padding.top + window_padding.bottom;

        let content_size = PhysicalSize {
            width: new_size.width - window_padding_width,
            height: new_size.height - window_padding_height,
        };

        let grid_size = self
            .renderer
            .grid_renderer
            .convert_physical_to_grid(content_size);

        // Have a minimum size
        if grid_size.width < MIN_WINDOW_WIDTH || grid_size.height < MIN_WINDOW_HEIGHT {
            return;
        }

        if self.saved_grid_size == Some(grid_size) {
            trace!("Grid matched saved size, skip update.");
            return;
        }
        self.saved_grid_size = Some(grid_size);
        EVENT_AGGREGATOR.send(UiCommand::Parallel(ParallelCommand::Resize {
            width: grid_size.width,
            height: grid_size.height,
        }));
    }

    fn handle_scale_factor_update(&mut self, scale_factor: f64) {
        self.renderer.handle_os_scale_factor_change(scale_factor);
        EVENT_AGGREGATOR.send(EditorCommand::RedrawScreen);
    }

    fn has_been_resized(&self) -> bool {
        self.window.inner_size() != self.size_at_startup
    }
}

pub fn create_window() {
    let icon = {
        let icon = load_from_memory(ICON).expect("Failed to parse icon data");
        let (width, height) = icon.dimensions();
        let mut rgba = Vec::with_capacity((width * height) as usize * 4);
        for (_, _, pixel) in icon.pixels() {
            rgba.extend_from_slice(&pixel.to_rgba().0);
        }
        Icon::from_rgba(rgba, width, height).expect("Failed to create icon object")
    };

    let event_loop = EventLoop::<UserEvent>::with_user_event();

    let cmd_line_settings = SETTINGS.get::<CmdLineSettings>();

    let mut maximized = cmd_line_settings.maximized;
    let mut previous_position = None;
    if let Ok(last_window_settings) = load_last_window_settings() {
        match last_window_settings {
            PersistentWindowSettings::Maximized => {
                maximized = true;
            }
            PersistentWindowSettings::Windowed { position, .. } => {
                previous_position = Some(position);
            }
        }
    }

    let winit_window_builder = window::WindowBuilder::new()
        .with_title("Neovide")
        .with_window_icon(Some(icon))
        .with_maximized(maximized)
        .with_transparent(true);

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
        if !maximized {
            winit_window_builder = winit_window_builder.with_position(previous_position);
        }
    }

    #[cfg(target_os = "linux")]
    let winit_window_builder = winit_window_builder
        .with_app_id(cmd_line_settings.wayland_app_id.clone())
        .with_class(
            cmd_line_settings.x11_wm_class_instance.clone(),
            cmd_line_settings.x11_wm_class.clone(),
        );

    #[cfg(target_os = "macos")]
    let winit_window_builder = winit_window_builder.with_accepts_first_mouse(false);

    let window = winit_window_builder.build(&event_loop).unwrap();

    enum FocusedState {
        Focused,
        UnfocusedNotDrawn,
        Unfocused,
    }

    let (txtemp, rx) = mpsc::channel::<Event<UserEvent>>();
    let mut tx = Some(txtemp);
    let mut render_thread_handle = Some(thread::spawn(move || {
        let initial_size = window.inner_size();

        // Check that window is visible in some monitor, and reposition it if not.
        let did_reposition = window
            .current_monitor()
            .and_then(|current_monitor| {
                let monitor_position = current_monitor.position();
                let monitor_size = current_monitor.size();
                let monitor_width = monitor_size.width as i32;
                let monitor_height = monitor_size.height as i32;

                let window_position = window.outer_position().ok()?;
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
            })
            .is_some();

        log::trace!("repositioned window: {}", did_reposition);

        let scale_factor = window.scale_factor();
        let renderer = Renderer::new(scale_factor);
        let saved_inner_size = window.inner_size();

        let window_command_receiver = EVENT_AGGREGATOR.register_event::<WindowCommand>();

        log::info!(
            "window created (scale_factor: {:.4}, font_dimensions: {:?})",
            scale_factor,
            renderer.grid_renderer.font_dimensions,
        );

        let wgpu_renderer = WGpuRenderer::new(&window);

        let mut window_wrapper = WinitWindowWrapper {
            wgpu_renderer,
            window,
            renderer,
            keyboard_manager: KeyboardManager::new(),
            mouse_manager: MouseManager::new(),
            title: String::from("Neovide"),
            fullscreen: false,
            font_changed_last_frame: false,
            size_at_startup: initial_size,
            maximized_at_startup: maximized,
            saved_inner_size,
            saved_grid_size: None,
            window_command_receiver,
        };

        tracy_create_gpu_context("main render context");

        let max_animation_dt = 1.0 / 120.0;
        let mut focused = FocusedState::Focused;
        let mut prev_frame_start = Instant::now();
        let mut frame_dt_avg = NoSumSMA::<f64, f64, 10>::new();

        #[allow(unused_assignments)]
        loop {
            tracy_zone!("render loop", 0);
            let e = rx.try_recv();
            let mut should_render = false;

            match e {
                // Window focus changed
                Ok(Event::WindowEvent {
                    event: WindowEvent::Focused(focused_event),
                    ..
                }) => {
                    focused = if focused_event {
                        FocusedState::Focused
                    } else {
                        FocusedState::UnfocusedNotDrawn
                    };
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    break;
                }
                _ => {}
            }
            if let Ok(e) = e {
                should_render |= window_wrapper.handle_event(e);
                window_wrapper.handle_window_commands();
                window_wrapper.synchronize_settings();
            } else {
                let mut dt = frame_dt_avg.get_average();
                should_render |= window_wrapper.prepare_frame();
                while dt > 0.0 {
                    let step = dt.min(max_animation_dt);

                    window_wrapper.animate_frame(step as f32);
                    dt -= step;
                }
                // Always render for now
                #[allow(clippy::overly_complex_bool_expr)]
                if should_render || true {
                    window_wrapper
                        .draw_frame(frame_dt_avg.get_most_recent_sample().unwrap_or(0.0) as f32);
                    frame_dt_avg.add_sample(prev_frame_start.elapsed().as_secs_f64());
                    prev_frame_start = Instant::now();
                }

                if let FocusedState::UnfocusedNotDrawn = focused {
                    focused = FocusedState::Unfocused;
                }
                #[cfg(target_os = "macos")]
                draw_background(&window_wrapper.window);
            }
        }
        let window = window_wrapper.window;
        save_window_geometry(
            window.is_maximized(),
            window_wrapper.saved_grid_size,
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
                Event::UserEvent(UserEvent::ScaleFactorChanged(scale_factor))
            }
            _ => {
                // With the current Winit version, all events, except ScaleFactorChanged are static
                e.to_static().expect("Unexpected event received")
            }
        };
        if let Some(tx) = &tx {
            tx.send(e).unwrap();
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
