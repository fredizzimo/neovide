use std::sync::Arc;

use skia_safe::{
    canvas::SaveLayerRec, gpu::SurfaceOrigin, image_filters::blur, BlendMode, Budgeted, Canvas,
    Color, ImageInfo, Matrix, Paint, Picture, PictureRecorder, Point, Rect, Surface, SurfaceProps,
    SurfacePropsFlags,
};

use crate::{
    dimensions::Dimensions,
    editor::Style,
    profiling::tracy_zone,
    redraw_scheduler::REDRAW_SCHEDULER,
    renderer::{animation_utils::*, GridRenderer, RendererSettings},
};
use winit::dpi::PhysicalSize;

use super::opengl::clamp_render_buffer_size;

#[derive(Clone, Debug)]
pub struct LineFragment {
    pub text: String,
    pub window_left: u64,
    pub width: u64,
    pub style: Option<Arc<Style>>,
}

#[derive(Clone, Debug)]
pub enum WindowDrawCommand {
    Position {
        grid_position: (f64, f64),
        grid_size: (u64, u64),
        floating_order: Option<u64>,
    },
    DrawLine {
        row: usize,
        line_fragments: Vec<LineFragment>,
    },
    Scroll {
        top: u64,
        bottom: u64,
        left: u64,
        right: u64,
        rows: i64,
        cols: i64,
    },
    Clear,
    Show,
    Hide,
    Close,
    Viewport {
        scroll_delta: f64,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct WindowPadding {
    pub top: u32,
    pub left: u32,
    pub right: u32,
    pub bottom: u32,
}

fn build_window_surface(parent_canvas: &mut Canvas, pixel_size: PhysicalSize<u32>) -> Surface {
    let pixel_size = clamp_render_buffer_size(pixel_size);
    let mut context = parent_canvas.recording_context().unwrap();
    let budgeted = Budgeted::Yes;
    let parent_image_info = parent_canvas.image_info();
    let image_info = ImageInfo::new(
        (pixel_size.width as i32, pixel_size.height as i32),
        parent_image_info.color_type(),
        parent_image_info.alpha_type(),
        parent_image_info.color_space(),
    );
    let surface_origin = SurfaceOrigin::TopLeft;
    // Subpixel layout (should be configurable/obtained from fontconfig).
    let props = SurfaceProps::new(SurfacePropsFlags::default(), skia_safe::PixelGeometry::RGBH);
    Surface::new_render_target(
        &mut context,
        budgeted,
        &image_info,
        None,
        surface_origin,
        Some(&props),
        None,
    )
    .expect("Could not create surface")
}

fn build_window_surface_with_grid_size(
    parent_canvas: &mut Canvas,
    grid_renderer: &GridRenderer,
    grid_size: Dimensions,
) -> Surface {
    let mut surface = build_window_surface(
        parent_canvas,
        (grid_size * grid_renderer.font_dimensions).into(),
    );

    let canvas = surface.canvas();
    canvas.clear(grid_renderer.get_default_background());
    surface
}

pub struct LocatedSurface {
    surface: Surface,
    pub vertical_position: f32,
}

impl LocatedSurface {
    fn new(
        parent_canvas: &mut Canvas,
        grid_renderer: &GridRenderer,
        grid_size: Dimensions,
        vertical_position: f32,
    ) -> LocatedSurface {
        let surface = build_window_surface_with_grid_size(parent_canvas, grid_renderer, grid_size);

        LocatedSurface {
            surface,
            vertical_position,
        }
    }
}

pub struct RenderedWindow {
    pub current_surface: LocatedSurface,

    pub id: u64,
    pub hidden: bool,
    pub floating_order: Option<u64>,

    pub grid_size: Dimensions,

    lines: Vec<Option<Picture>>,
    pub top_index: isize,

    grid_start_position: Point,
    pub grid_current_position: Point,
    grid_destination: Point,
    position_t: f32,

    start_scroll: f32,
    pub current_scroll: f32,
    scroll_t: f32,

    pub padding: WindowPadding,
}

#[derive(Clone, Debug)]
pub struct WindowDrawDetails {
    pub id: u64,
    pub region: Rect,
    pub floating_order: Option<u64>,
}

impl RenderedWindow {
    pub fn new(
        parent_canvas: &mut Canvas,
        grid_renderer: &GridRenderer,
        id: u64,
        grid_position: Point,
        grid_size: Dimensions,
        padding: WindowPadding,
    ) -> RenderedWindow {
        let current_surface = LocatedSurface::new(parent_canvas, grid_renderer, grid_size, 0.);

        RenderedWindow {
            current_surface,
            id,
            hidden: false,
            floating_order: None,

            grid_size,

            lines: vec![None; (grid_size.height * 2) as usize],
            top_index: 0,

            grid_start_position: grid_position,
            grid_current_position: grid_position,
            grid_destination: grid_position,
            position_t: 2.0, // 2.0 is out of the 0.0 to 1.0 range and stops animation.

            start_scroll: 0.0,
            current_scroll: 0.0,
            scroll_t: 2.0, // 2.0 is out of the 0.0 to 1.0 range and stops animation.
            padding,
        }
    }

    pub fn pixel_region(&self, font_dimensions: Dimensions) -> Rect {
        let current_pixel_position = Point::new(
            self.grid_current_position.x * font_dimensions.width as f32,
            self.grid_current_position.y * font_dimensions.height as f32,
        );

        let image_size: (i32, i32) = (self.grid_size * font_dimensions).into();

        Rect::from_point_and_size(current_pixel_position, image_size)
    }

    pub fn update(&mut self, settings: &RendererSettings, dt: f32) -> bool {
        let mut animating = false;

        {
            if 1.0 - self.position_t < std::f32::EPSILON {
                // We are at destination, move t out of 0-1 range to stop the animation.
                self.position_t = 2.0;
            } else {
                animating = true;
                self.position_t =
                    (self.position_t + dt / settings.position_animation_length).min(1.0);
            }

            self.grid_current_position = ease_point(
                ease_out_expo,
                self.grid_start_position,
                self.grid_destination,
                self.position_t,
            );
        }

        {
            if 1.0 - self.scroll_t < std::f32::EPSILON {
                // We are at destination, move t out of 0-1 range to stop the animation.
                self.scroll_t = 2.0;
            } else {
                animating = true;
                self.scroll_t = (self.scroll_t + dt / settings.scroll_animation_length).min(1.0);
            }

            self.current_scroll = ease(ease_out_expo, self.start_scroll, 0.0, self.scroll_t);
        }

        animating
    }

    fn draw_surface(&mut self, font_dimensions: Dimensions) {
        let canvas = self.current_surface.surface.canvas();
        let mut matrix = Matrix::new_identity();

        let scroll_offset_lines = self.current_scroll.floor();
        let scroll_offset = scroll_offset_lines - self.current_scroll;

        for i in 0..self.grid_size.height + 1 {
            matrix.set_translate((
                0.0,
                (scroll_offset + i as f32) * font_dimensions.height as f32,
            ));
            let line_index = (self.top_index + scroll_offset_lines as isize + i as isize)
                .rem_euclid(self.lines.len() as isize) as usize;
            if let Some(picture) = &self.lines[line_index] {
                canvas.draw_picture(picture, Some(&matrix), None);
            }
        }
    }

    pub fn draw(
        &mut self,
        root_canvas: &mut Canvas,
        settings: &RendererSettings,
        default_background: Color,
        font_dimensions: Dimensions,
        dt: f32,
    ) -> WindowDrawDetails {
        if self.update(settings, dt) {
            REDRAW_SCHEDULER.queue_next_frame();
        }

        self.draw_surface(font_dimensions);

        let pixel_region = self.pixel_region(font_dimensions);

        root_canvas.save();
        root_canvas.clip_rect(pixel_region, None, Some(false));

        if self.floating_order.is_none() {
            root_canvas.clear(default_background);
        }

        if self.floating_order.is_some() && settings.floating_blur {
            if let Some(blur) = blur(
                (
                    settings.floating_blur_amount_x,
                    settings.floating_blur_amount_y,
                ),
                None,
                None,
                None,
            ) {
                let save_layer_rec = SaveLayerRec::default()
                    .backdrop(&blur)
                    .bounds(&pixel_region);

                root_canvas.save_layer(&save_layer_rec);
            }
        }

        let mut paint = Paint::default();
        // We want each surface to overwrite the one underneath and will use layers to ensure
        // only lower priority surfaces will get clobbered and not the underlying windows.
        paint.set_blend_mode(BlendMode::Src);
        paint.set_anti_alias(false);

        // Save layer so that setting the blend mode doesn't effect the blur.
        root_canvas.save_layer(&SaveLayerRec::default());
        let mut a = 255;
        if self.floating_order.is_some() {
            a = (settings.floating_opacity.min(1.0).max(0.0) * 255.0) as u8;
        }

        paint.set_color(default_background.with_a(a));
        root_canvas.draw_rect(pixel_region, &paint);

        paint.set_color(Color::from_argb(255, 255, 255, 255));

        // Draw current surface.
        let snapshot = self.current_surface.surface.image_snapshot();
        root_canvas.draw_image_rect(snapshot, None, pixel_region, &paint);

        root_canvas.restore();

        if self.floating_order.is_some() {
            root_canvas.restore();
        }

        root_canvas.restore();

        WindowDrawDetails {
            id: self.id,
            region: pixel_region,
            floating_order: self.floating_order,
        }
    }

    fn reset_scroll(&mut self) {
        self.start_scroll = 0.0;
        self.scroll_t = 2.0;
    }

    pub fn handle_window_draw_command(
        &mut self,
        grid_renderer: &mut GridRenderer,
        draw_command: WindowDrawCommand,
    ) {
        match draw_command {
            WindowDrawCommand::Position {
                grid_position: (grid_left, grid_top),
                grid_size,
                floating_order,
            } => {
                tracy_zone!("position_cmd", 0);
                let Dimensions {
                    width: font_width,
                    height: font_height,
                } = grid_renderer.font_dimensions;

                let top_offset = self.padding.top as f32 / font_height as f32;
                let left_offset = self.padding.left as f32 / font_width as f32;

                let grid_left = grid_left.max(0.0);
                let grid_top = grid_top.max(0.0);
                let new_destination: Point =
                    (grid_left as f32 + left_offset, grid_top as f32 + top_offset).into();
                let new_grid_size: Dimensions = grid_size.into();

                if self.grid_destination != new_destination {
                    if self.grid_start_position.x.abs() > f32::EPSILON
                        || self.grid_start_position.y.abs() > f32::EPSILON
                    {
                        self.position_t = 0.0; // Reset animation as we have a new destination.
                        self.grid_start_position = self.grid_current_position;
                    } else {
                        // We don't want to animate since the window is animating out of the start location,
                        // so we set t to 2.0 to stop animations.
                        self.position_t = 2.0;
                        self.grid_start_position = new_destination;
                    }
                    self.grid_destination = new_destination;
                }

                if self.grid_size != new_grid_size {
                    self.current_surface.surface = build_window_surface_with_grid_size(
                        self.current_surface.surface.canvas(),
                        grid_renderer,
                        new_grid_size,
                    );
                    self.grid_size = new_grid_size;
                }

                // This could perhaps be optimized, setting the position does not necessarily need
                // to resize and reset everything. See editor::window::Window::position for the
                // corresponding code on the logic side.
                self.lines = vec![None; (new_grid_size.height * 2) as usize];
                self.top_index = 0;

                self.floating_order = floating_order;

                if self.hidden {
                    self.hidden = false;
                    self.position_t = 2.0; // We don't want to animate since the window is becoming visible,
                                           // so we set t to 2.0 to stop animations.
                    self.grid_start_position = new_destination;
                    self.grid_destination = new_destination;
                }
                self.reset_scroll();
            }
            WindowDrawCommand::DrawLine {
                row,
                line_fragments,
            } => {
                tracy_zone!("draw_line_cmd", 0);
                let font_dimensions = grid_renderer.font_dimensions;
                let mut recorder = PictureRecorder::new();

                let grid_rect = Rect::from_wh(
                    (self.grid_size.width * font_dimensions.width) as f32,
                    font_dimensions.height as f32,
                );
                let canvas = recorder.begin_recording(grid_rect, None);

                for line_fragment in line_fragments.iter() {
                    let LineFragment {
                        window_left,
                        width,
                        style,
                        ..
                    } = line_fragment;
                    let grid_position = (*window_left, 0);
                    grid_renderer.draw_background(
                        canvas,
                        grid_position,
                        *width,
                        style,
                        self.floating_order.is_some(),
                    );
                }

                for line_fragment in line_fragments.into_iter() {
                    let LineFragment {
                        text,
                        window_left,
                        width,
                        style,
                    } = line_fragment;
                    let grid_position = (window_left, 0);
                    grid_renderer.draw_foreground(canvas, text, grid_position, width, &style);
                }

                let line_index =
                    (self.top_index + row as isize).rem_euclid(self.lines.len() as isize) as usize;
                self.lines[line_index] = recorder.finish_recording_as_picture(None);
            }
            WindowDrawCommand::Scroll {
                top,
                bottom,
                left,
                right,
                rows,
                cols,
            } => {
                tracy_zone!("scroll_cmd", 0);
                if top == 0
                    && bottom == self.grid_size.height
                    && left == 0
                    && right == self.grid_size.width
                    && cols == 0
                {
                    let mut scroll_offset = self.current_scroll;
                    self.top_index += rows as isize;
                    let minmax = self.lines.len() - self.grid_size.height as usize;
                    if rows.unsigned_abs() as usize > minmax {
                        // The scroll offset has to be reset when scrolling too far
                        scroll_offset = 0.0;
                    } else {
                        scroll_offset -= rows as f32;
                        // And even when scrolling in steps, we can't let it drift too far, since the
                        // buffer size is limited
                        scroll_offset = scroll_offset.clamp(-(minmax as f32), minmax as f32);
                    }
                    self.start_scroll = scroll_offset;
                    self.scroll_t = 0.0;
                }
            }
            WindowDrawCommand::Clear => {
                tracy_zone!("clear_cmd", 0);
                self.top_index = 0;
                self.reset_scroll();
                self.current_surface.surface = build_window_surface_with_grid_size(
                    self.current_surface.surface.canvas(),
                    grid_renderer,
                    self.grid_size,
                );
            }
            WindowDrawCommand::Show => {
                tracy_zone!("show_cmd", 0);
                if self.hidden {
                    self.hidden = false;
                    self.position_t = 2.0; // We don't want to animate since the window is becoming visible,
                                           // so we set t to 2.0 to stop animations.
                    self.grid_start_position = self.grid_destination;
                    self.reset_scroll();
                }
            }
            WindowDrawCommand::Hide => {
                tracy_zone!("hide_cmd", 0);
                self.hidden = true;
            }
            WindowDrawCommand::Viewport { .. } => {}
            _ => {}
        };
    }
}
