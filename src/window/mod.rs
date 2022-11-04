mod keyboard_manager;
mod mouse_manager;
mod renderer;
mod settings;

#[cfg(target_os = "macos")]
mod draw_background;

#[cfg(target_os = "linux")]
use std::env;
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use log::trace;
use simple_moving_average::{NoSumSMA, SMA};
use tokio::sync::mpsc::UnboundedReceiver;
use winit::{
    dpi::PhysicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::{self, Fullscreen, Icon},
};

#[cfg(target_os = "macos")]
use winit::platform::macos::WindowBuilderExtMacOS;

#[cfg(target_os = "macos")]
use draw_background::draw_background;

#[cfg(target_os = "linux")]
use winit::platform::wayland::WindowBuilderExtWayland;
#[cfg(target_os = "linux")]
use winit::platform::x11::WindowBuilderExtX11;

use crate::profiling::{
    emit_frame_mark, tracy_create_gpu_context, tracy_gpu_collect, tracy_gpu_zone, tracy_zone,
};

use image::{load_from_memory, GenericImageView, Pixel};
use keyboard_manager::KeyboardManager;
use mouse_manager::MouseManager;
use renderer::SkiaRenderer;

use crate::{
    bridge::{ParallelCommand, UiCommand},
    cmd_line::CmdLineSettings,
    dimensions::Dimensions,
    editor::EditorCommand,
    event_aggregator::EVENT_AGGREGATOR,
    frame::Frame,
    renderer::Renderer,
    renderer::WindowPadding,
    renderer::{build_context, build_window, VSync, WindowedContext},
    running_tracker::*,
    settings::{
        load_last_window_settings, save_window_size, PersistentWindowSettings,
        DEFAULT_WINDOW_GEOMETRY, SETTINGS,
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
    windowed_context: WindowedContext,
    skia_renderer: SkiaRenderer,
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
        let window = self.windowed_context.window();
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
        self.windowed_context.window().set_title(&self.title);
    }

    pub fn send_font_names(&self) {
        let font_names = self.renderer.font_names();
        EVENT_AGGREGATOR.send(UiCommand::Parallel(ParallelCommand::DisplayAvailableFonts(
            font_names,
        )));
    }

    pub fn handle_quit(&mut self) {
        if SETTINGS.get::<CmdLineSettings>().server.is_none() {
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
            self.windowed_context.window(),
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
            Event::RedrawRequested(..) | Event::WindowEvent { .. } => {}
            _ => {}
        }
        should_render
    }

    pub fn draw_frame(&mut self, vsync: &mut VSync, dt: f32) {
        tracy_zone!("draw_frame");
        self.renderer.draw_frame(self.skia_renderer.canvas(), dt);
        {
            tracy_gpu_zone!("skia flush");
            self.skia_renderer.gr_context.flush_and_submit();
        }
        {
            tracy_gpu_zone!("wait for vsync");
            vsync.wait_for_vsync();
        }
        {
            tracy_gpu_zone!("swap buffers");
            self.windowed_context.swap_buffers().unwrap();
        }
        emit_frame_mark();
        tracy_gpu_collect();
    }

    pub fn animate_frame(&mut self, dt: f32) -> bool {
        tracy_zone!("animate_frame", 0);
        self.renderer.animate_frame(dt)
    }

    pub fn prepare_frame(&mut self) -> bool {
        tracy_zone!("prepare_frame", 0);
        let mut should_render = false;

        let window = self.windowed_context.window();

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

        let new_size = window.inner_size();
        if self.saved_inner_size != new_size || self.font_changed_last_frame || padding_changed {
            self.font_changed_last_frame = false;
            self.saved_inner_size = new_size;

            self.handle_new_grid_size(new_size);
            self.skia_renderer.resize(&self.windowed_context);
            should_render = true;
        }

        let handle_draw_commands_result = self.renderer.handle_draw_commands();

        self.font_changed_last_frame |= handle_draw_commands_result.0;
        should_render |= handle_draw_commands_result.1;

        // Wait until fonts are loaded, so we can set proper window size.
        if !self.renderer.grid_renderer.is_ready {
            return false;
        }

        // Resize at startup happens when window is maximized or when using tiling WM
        // which already resized window.
        let resized_at_startup = self.maximized_at_startup || self.has_been_resized();

        log::trace!("Inner size: {:?}", new_size);

        if self.saved_grid_size.is_none() && !resized_at_startup {
            self.init_window_size();
            should_render |= true;
        }
        should_render
    }

    fn init_window_size(&self) {
        let settings = SETTINGS.get::<CmdLineSettings>();
        log::trace!("Settings geometry {:?}", settings.geometry,);
        log::trace!("Settings size {:?}", settings.size);

        let window = self.windowed_context.window();
        let inner_size = if let Some(size) = settings.size {
            // --size
            size.into()
        } else if let Some(geometry) = settings.geometry {
            // --geometry
            self.renderer
                .grid_renderer
                .convert_grid_to_physical(geometry)
        } else if let Ok(PersistentWindowSettings::Windowed {
            pixel_size: Some(size),
            ..
        }) = load_last_window_settings()
        {
            // remembered size
            size
        } else {
            // default geometry
            self.renderer
                .grid_renderer
                .convert_grid_to_physical(DEFAULT_WINDOW_GEOMETRY)
        };
        window.set_inner_size(inner_size);
        // next frame will detect change in window.inner_size() and hence will
        // handle_new_grid_size automatically
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
        self.windowed_context.window().inner_size() != self.size_at_startup
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

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

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

    let (window, config) = build_window(winit_window_builder, &event_loop);

    let (txtemp, rx) = mpsc::channel::<Event<UserEvent>>();
    let mut tx = Some(txtemp);
    let mut render_thread_handle = Some(thread::spawn(move || {
        let windowed_context = build_context(window, config, &cmd_line_settings);
        let window = windowed_context.window();
        let initial_size = window.inner_size();

        // Check that window is visible in some monitor, and reposition it if not.
        let did_reposition = window
            .current_monitor()
            .and_then(|current_monitor| {
                let monitor_position = current_monitor.position();
                let monitor_size = current_monitor.size();
                let monitor_width = monitor_size.width as i32;
                let monitor_height = monitor_size.height as i32;

                let window_position = previous_position
                    .filter(|_| !maximized)
                    .or_else(|| window.outer_position().ok())?;

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

        let scale_factor = windowed_context.window().scale_factor();
        let renderer = Renderer::new(scale_factor);
        let saved_inner_size = window.inner_size();

        let skia_renderer = SkiaRenderer::new(&windowed_context);

        let window_command_receiver = EVENT_AGGREGATOR.register_event::<WindowCommand>();

        log::info!(
            "window created (scale_factor: {:.4}, font_dimensions: {:?})",
            scale_factor,
            renderer.grid_renderer.font_dimensions,
        );

        let mut window_wrapper = WinitWindowWrapper {
            windowed_context,
            skia_renderer,
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

        tracy_create_gpu_context("main_render_context");

        let max_animation_dt = 1.0 / 120.0;

        let mut previous_frame_start = Instant::now();
        let mut last_dt: f32 = 0.0;
        let mut frame_dt_avg = NoSumSMA::<f64, f64, 10>::new();
        let mut should_render = true;
        let mut num_consecutive_rendered: usize = 0;

        enum FocusedState {
            Focused,
            UnfocusedNotDrawn,
            Unfocused,
        }
        let mut focused = FocusedState::Focused;

        let mut vsync = VSync::new();

        #[allow(unused_assignments)]
        loop {
            tracy_zone!("render loop", 0);

            let refresh_rate = match focused {
                FocusedState::Focused | FocusedState::UnfocusedNotDrawn => {
                    SETTINGS.get::<WindowSettings>().refresh_rate as f32
                }
                FocusedState::Unfocused => {
                    SETTINGS.get::<WindowSettings>().refresh_rate_idle as f32
                }
            }
            .max(1.0);
            let expected_frame_duration = Duration::from_secs_f32(1.0 / refresh_rate);

            let e = if num_consecutive_rendered > 0 {
                rx.try_recv()
                    .map_err(|e| matches!(e, TryRecvError::Disconnected))
            } else {
                let deadline = previous_frame_start + expected_frame_duration;
                let duration = deadline.saturating_duration_since(Instant::now());
                rx.recv_timeout(duration)
                    .map_err(|e| matches!(e, RecvTimeoutError::Disconnected))
            };

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
                Err(true) => {
                    break;
                }
                _ => {}
            }
            if let Ok(e) = e {
                window_wrapper.handle_window_commands();
                window_wrapper.synchronize_settings();
                should_render |= window_wrapper.handle_event(e);
            } else {
                let dt = if num_consecutive_rendered > 0 && frame_dt_avg.get_num_samples() > 0 {
                    frame_dt_avg.get_average() as f32
                } else {
                    last_dt
                }
                .min(1.0);
                vsync.set_refresh_rate(SETTINGS.get::<WindowSettings>().refresh_rate);
                should_render |= window_wrapper.prepare_frame();
                let num_steps = (dt / max_animation_dt).ceil();
                let step = dt / num_steps;
                for _ in 0..num_steps as usize {
                    should_render |= window_wrapper.animate_frame(step);
                }
                if should_render || cmd_line_settings.no_idle {
                    window_wrapper.draw_frame(&mut vsync, last_dt);

                    if num_consecutive_rendered > 2 {
                        frame_dt_avg.add_sample(previous_frame_start.elapsed().as_secs_f64());
                    }
                    should_render = false;
                    num_consecutive_rendered += 1;
                } else {
                    num_consecutive_rendered = 0;
                }
                last_dt = previous_frame_start.elapsed().as_secs_f32();
                previous_frame_start = Instant::now();
                if let FocusedState::UnfocusedNotDrawn = focused {
                    focused = FocusedState::Unfocused;
                }

                #[cfg(target_os = "macos")]
                draw_background(window_wrapper.windowed_context.window());
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
