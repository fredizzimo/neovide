use std::{cell::RefCell, rc::Rc, sync::Arc};

use skia_safe::{
    canvas::SaveLayerRec,
    image_filters::blur,
    utils::shadow_utils::{draw_shadow, ShadowFlags},
    BlendMode, Canvas, ClipOp, Color, Matrix, Paint, Path, Picture, PictureRecorder, Point3, Rect,
};

use crate::{
    cmd_line::CmdLineSettings,
    editor::{AnchorInfo, Style, WindowType},
    profiling::{tracy_plot, tracy_zone},
    renderer::{animation_utils::*, GridRenderer, RendererSettings},
    settings::SETTINGS,
    units::{to_skia_rect, GridPos, GridRect, GridScale, GridSize, PixelRect},
    utils::RingBuffer,
};

#[derive(Clone, Debug, PartialEq)]
pub struct LineFragment {
    pub text: String,
    pub window_left: u64,
    pub width: u64,
    pub style: Option<Arc<Style>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewportMargins {
    pub top: u64,
    pub bottom: u64,
    pub inferred: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WindowDrawCommand {
    Position {
        grid_position: (f64, f64),
        grid_size: (u64, u64),
        anchor_info: Option<AnchorInfo>,
        window_type: WindowType,
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
    ViewportMargins {
        top: u64,
        bottom: u64,
        left: u64,
        right: u64,
    },
}

#[derive(Clone)]
struct Line {
    line_fragments: Vec<LineFragment>,
    background_picture: Option<Picture>,
    foreground_picture: Option<Picture>,
    has_transparency: bool,
    is_inferred_border: bool,
    is_valid: bool,
}

pub struct RenderedWindow {
    pub id: u64,
    pub hidden: bool,
    pub anchor_info: Option<AnchorInfo>,
    window_type: WindowType,

    pub grid_size: GridSize<u32>,

    scrollback_lines: RingBuffer<Option<Rc<RefCell<Line>>>>,
    actual_lines: RingBuffer<Option<Rc<RefCell<Line>>>>,
    scroll_delta: isize,
    pub viewport_margins: ViewportMargins,

    grid_start_position: GridPos<f32>,
    pub grid_current_position: GridPos<f32>,
    grid_destination: GridPos<f32>,
    position_t: f32,

    pub scroll_animation: CriticallyDampedSpringAnimation,

    has_transparency: bool,
}

#[derive(Clone, Debug)]
pub struct WindowDrawDetails {
    pub id: u64,
    pub region: PixelRect<f32>,
}

impl WindowDrawDetails {
    pub fn event_grid_id(&self) -> u64 {
        if SETTINGS.get::<CmdLineSettings>().no_multi_grid {
            0
        } else {
            self.id
        }
    }
}

impl RenderedWindow {
    pub fn new(id: u64, grid_position: GridPos<i32>, grid_size: GridSize<u32>) -> RenderedWindow {
        RenderedWindow {
            id,
            hidden: false,
            anchor_info: None,
            window_type: WindowType::Editor,

            grid_size,

            actual_lines: RingBuffer::new(grid_size.height as usize, None),
            scrollback_lines: RingBuffer::new(2 * grid_size.height as usize, None),
            scroll_delta: 0,
            viewport_margins: ViewportMargins {
                top: 0,
                bottom: 0,
                inferred: true,
            },

            grid_start_position: grid_position.cast(),
            grid_current_position: grid_position.cast(),
            grid_destination: grid_position.cast(),
            position_t: 2.0, // 2.0 is out of the 0.0 to 1.0 range and stops animation.

            scroll_animation: CriticallyDampedSpringAnimation::new(),

            has_transparency: false,
        }
    }

    pub fn pixel_region(&self, grid_scale: GridScale) -> PixelRect<f32> {
        GridRect::<f32>::from_origin_and_size(self.grid_current_position, self.grid_size.cast())
            * grid_scale
    }

    fn get_target_position(&self, grid_rect: &GridRect<f32>) -> GridPos<f32> {
        let destination = self.grid_destination + grid_rect.min.to_vector();

        if self.anchor_info.is_none() {
            return destination;
        }

        let mut grid_size: GridSize<f32> = self.grid_size.cast();

        if matches!(self.window_type, WindowType::Message { .. }) {
            // The message grid size is always the full window size, so use the relative position to
            // calculate the actual grid size
            grid_size.height -= self.grid_destination.y;
        }
        // If a floating window is partially outside the grid, then move it in from the right, but
        // ensure that the left edge is always visible.
        let x = destination
            .x
            .min(grid_rect.max.x - grid_size.width)
            .max(grid_rect.min.x);

        // For messages the last line is most important, (it shows press enter), so let the position go negative
        // Otherwise ensure that the window start row is within the screen
        let mut y = destination.y.min(grid_rect.max.y - grid_size.height);
        if !matches!(self.window_type, WindowType::Message { .. }) {
            y = y.max(grid_rect.min.y)
        }
        GridPos::<f32>::new(x, y)
    }

    /// Returns `true` if the window has been animated in this step.
    pub fn animate(
        &mut self,
        settings: &RendererSettings,
        grid_rect: &GridRect<f32>,
        dt: f32,
    ) -> bool {
        let mut animating = false;

        if self.position_t > 1.0 - f32::EPSILON {
            // We are at destination, move t out of 0-1 range to stop the animation.
            self.position_t = 2.0;
        } else {
            animating = true;
            self.position_t = (self.position_t + dt / settings.position_animation_length).min(1.0);
        }

        let prev_position = self.grid_current_position;
        self.grid_current_position = ease_point(
            ease_out_expo,
            self.grid_start_position.cast_unit(),
            self.get_target_position(grid_rect).cast_unit(),
            self.position_t,
        )
        .cast_unit();
        animating |= self.grid_current_position != prev_position;

        let scrolling = self
            .scroll_animation
            .update(dt, settings.scroll_animation_length);

        animating |= scrolling;

        if scrolling {
            tracy_plot!("Scroll position {}", self.scroll_animation.position.into());
        }

        animating
    }

    pub fn draw_surface(
        &mut self,
        canvas: &Canvas,
        pixel_region: &Rect,
        grid_scale: GridScale,
        default_background: Color,
    ) {
        let scroll_offset_lines = self.scroll_animation.position.floor();
        let scroll_offset = scroll_offset_lines - self.scroll_animation.position;
        let scroll_offset_lines = scroll_offset_lines as isize;
        let scroll_offset_pixels = (scroll_offset * grid_scale.0.height).round() as isize;
        let line_height = grid_scale.0.height;
        let mut has_transparency = false;

        let lines: Vec<(Matrix, &Rc<RefCell<Line>>)> = if !self.scrollback_lines.is_empty() {
            (0..self.grid_size.height as isize + 1)
                .filter_map(|i| {
                    self.scrollback_lines[scroll_offset_lines + i]
                        .as_ref()
                        .map(|line| (i, line))
                })
                .map(|(i, line)| {
                    let mut matrix = Matrix::new_identity();
                    matrix.set_translate((
                        pixel_region.left(),
                        pixel_region.top()
                            + (scroll_offset_pixels
                                + ((i + self.viewport_margins.top as isize)
                                    * grid_scale.0.height as isize))
                                as f32,
                    ));
                    (matrix, line)
                })
                .collect()
        } else {
            Vec::new()
        };

        let top_border_indices = 0..self.viewport_margins.top as isize;
        let actual_line_count = self.actual_lines.len() as isize;
        let bottom_border_indices =
            actual_line_count - self.viewport_margins.bottom as isize..actual_line_count;
        let margins_inferred = self.viewport_margins.inferred;

        let border_lines: Vec<_> = top_border_indices
            .chain(bottom_border_indices)
            .filter_map(|i| {
                self.actual_lines[i].as_ref().and_then(|line| {
                    if !margins_inferred || line.borrow().is_inferred_border {
                        Some((i, line))
                    } else {
                        None
                    }
                })
            })
            .map(|(i, line)| {
                let mut matrix = Matrix::new_identity();
                matrix.set_translate((
                    pixel_region.left(),
                    pixel_region.top() + (i * grid_scale.0.height as isize) as f32,
                ));
                (matrix, line)
            })
            .collect();

        let inner_region = Rect::from_xywh(
            pixel_region.x(),
            pixel_region.y() + self.viewport_margins.top as f32 * line_height,
            pixel_region.width(),
            pixel_region.height()
                - (self.viewport_margins.top + self.viewport_margins.bottom) as f32 * line_height,
        );

        let mut background_paint = Paint::default();
        background_paint.set_blend_mode(BlendMode::Src);
        background_paint.set_alpha(default_background.a());

        let save_layer_rec = SaveLayerRec::default()
            .bounds(pixel_region)
            .paint(&background_paint);
        canvas.save_layer(&save_layer_rec);
        canvas.clear(default_background.with_a(255));
        for (matrix, line) in &border_lines {
            let line = line.borrow();
            if let Some(background_picture) = &line.background_picture {
                has_transparency |= line.has_transparency;
                canvas.draw_picture(background_picture, Some(matrix), None);
            }
        }
        canvas.save();
        canvas.clip_rect(inner_region, None, false);
        for (matrix, line) in &lines {
            let line = line.borrow();
            if let Some(background_picture) = &line.background_picture {
                has_transparency |= line.has_transparency;
                canvas.draw_picture(background_picture, Some(matrix), None);
            }
        }
        canvas.restore();
        canvas.restore();

        for (matrix, line) in &border_lines {
            let line = line.borrow();
            if let Some(foreground_picture) = &line.foreground_picture {
                canvas.draw_picture(foreground_picture, Some(matrix), None);
            }
        }
        canvas.save();
        canvas.clip_rect(inner_region, None, false);
        for (matrix, line) in &lines {
            let line = line.borrow();
            if let Some(foreground_picture) = &line.foreground_picture {
                canvas.draw_picture(foreground_picture, Some(matrix), None);
            }
        }
        canvas.restore();
        self.has_transparency = has_transparency;
    }

    fn has_transparency(&self) -> bool {
        let scroll_offset_lines = self.scroll_animation.position.floor() as isize;
        if self.scrollback_lines.is_empty() {
            return false;
        }
        self.scrollback_lines
            .iter_range(
                scroll_offset_lines..scroll_offset_lines + self.grid_size.height as isize + 1,
            )
            .flatten()
            .any(|line| line.borrow().has_transparency)
    }

    pub fn draw(
        &mut self,
        root_canvas: &Canvas,
        settings: &RendererSettings,
        default_background: Color,
        grid_scale: GridScale,
        previous_floating_rects: &mut Vec<PixelRect<f32>>,
    ) -> WindowDrawDetails {
        let has_transparency = default_background.a() != 255 || self.has_transparency();

        let pixel_region_box = self.pixel_region(grid_scale);
        let pixel_region = to_skia_rect(&pixel_region_box);
        let transparent_floating = self.anchor_info.is_some() && has_transparency;

        if self.anchor_info.is_some()
            && settings.floating_shadow
            && !previous_floating_rects
                .iter()
                .any(|rect| rect.contains_box(&pixel_region_box))
        {
            root_canvas.save();
            let shadow_path = Path::rect(pixel_region, None);
            // We clip using the Difference op to make sure that the shadow isn't rendered inside
            // the window itself.
            root_canvas.clip_path(&shadow_path, Some(ClipOp::Difference), None);
            // The light angle is specified in degrees from the vertical, so we first convert them
            // to radians and then use sin/cos to get the y and z components of the light
            let light_angle_radians = settings.light_angle_degrees.to_radians();
            draw_shadow(
                root_canvas,
                &shadow_path,
                // Specifies how far from the root canvas the shadow casting rect is. We just use
                // the z component here to set it a constant distance away.
                Point3::new(0., 0., settings.floating_z_height),
                // Because we use the DIRECTIONAL_LIGHT shadow flag, this specifies the angle that
                // the light is coming from.
                Point3::new(0., -light_angle_radians.sin(), light_angle_radians.cos()),
                // This is roughly equal to the apparent radius of the light .
                5.,
                Color::from_argb((0.03 * 255.) as u8, 0, 0, 0),
                Color::from_argb((0.35 * 255.) as u8, 0, 0, 0),
                // Directional Light flag is necessary to make the shadow render consistently
                // across various sizes of floating windows. It effects how the light direction is
                // processed.
                Some(ShadowFlags::DIRECTIONAL_LIGHT),
            );
            root_canvas.restore();
            previous_floating_rects.push(pixel_region_box);
        }

        root_canvas.save();
        root_canvas.clip_rect(pixel_region, None, Some(false));
        let need_blur = transparent_floating && settings.floating_blur;

        if need_blur {
            if let Some(blur) = blur(
                (
                    settings.floating_blur_amount_x,
                    settings.floating_blur_amount_y,
                ),
                None,
                None,
                None,
            ) {
                let paint = Paint::default()
                    .set_anti_alias(false)
                    .set_blend_mode(BlendMode::Src)
                    .to_owned();
                let save_layer_rec = SaveLayerRec::default()
                    .backdrop(&blur)
                    .bounds(&pixel_region)
                    .paint(&paint);
                root_canvas.save_layer(&save_layer_rec);
                root_canvas.restore();
            }
        }

        let paint = Paint::default()
            .set_anti_alias(false)
            .set_color(Color::from_argb(255, 255, 255, default_background.a()))
            .set_blend_mode(if self.anchor_info.is_some() {
                BlendMode::SrcOver
            } else {
                BlendMode::Src
            })
            .to_owned();

        let save_layer_rec = SaveLayerRec::default().bounds(&pixel_region).paint(&paint);
        root_canvas.save_layer(&save_layer_rec);
        self.draw_surface(root_canvas, &pixel_region, grid_scale, default_background);
        root_canvas.restore();

        root_canvas.restore();

        WindowDrawDetails {
            id: self.id,
            region: pixel_region_box,
        }
    }

    pub fn handle_window_draw_command(&mut self, draw_command: WindowDrawCommand) {
        match draw_command {
            WindowDrawCommand::Position {
                grid_position,
                grid_size,
                anchor_info,
                window_type,
            } => {
                tracy_zone!("position_cmd", 0);

                let new_grid_size: GridSize<u32> =
                    GridSize::<u64>::from(grid_size).try_cast().unwrap();
                let grid_position: GridPos<f32> =
                    GridPos::<f64>::from(grid_position).try_cast().unwrap();

                if self.grid_destination != grid_position {
                    if self.grid_start_position.x.abs() > f32::EPSILON
                        || self.grid_start_position.y.abs() > f32::EPSILON
                    {
                        self.position_t = 0.0; // Reset animation as we have a new destination.
                        self.grid_start_position = self.grid_current_position;
                    } else {
                        // We don't want to animate since the window is animating out of the start location,
                        // so we set t to 2.0 to stop animations.
                        self.position_t = 2.0;
                        self.grid_start_position = grid_position;
                    }
                    self.grid_destination = grid_position;
                }

                let height = new_grid_size.height as usize;
                self.actual_lines.resize(height, None);
                self.grid_size = new_grid_size;

                self.scrollback_lines.resize(2 * height, None);
                self.scrollback_lines.clone_from_iter(&self.actual_lines);
                self.scroll_delta = 0;

                if height != self.actual_lines.len() {
                    self.scroll_animation.reset();
                }

                self.anchor_info = anchor_info;
                self.window_type = window_type;

                if self.hidden {
                    self.hidden = false;
                    self.position_t = 2.0; // We don't want to animate since the window is becoming visible,
                                           // so we set t to 2.0 to stop animations.
                    self.grid_start_position = grid_position;
                    self.grid_destination = grid_position;
                }
            }
            WindowDrawCommand::DrawLine {
                row,
                line_fragments,
            } => {
                tracy_zone!("draw_line_cmd", 0);

                let mut line = Line {
                    line_fragments,
                    background_picture: None,
                    foreground_picture: None,
                    has_transparency: false,
                    is_inferred_border: false,
                    is_valid: false,
                };

                if self.viewport_margins.inferred {
                    let check_border = |fragment: &LineFragment, check: &dyn Fn(&str) -> bool| {
                        fragment.style.as_ref().map_or(false, |style| {
                            style.infos.last().map_or(false, |info| {
                                // The specification seems to indicate that kind should be UI and
                                // then we only need to test ui_name. But at least for FloatTitle,
                                // that is not the case, the kind is set to syntax and hi_name is
                                // set.
                                check(&info.ui_name) || check(&info.hi_name)
                            })
                        })
                    };

                    let float_border =
                        |s: &str| matches!(s, "FloatBorder" | "FloatTitle" | "FloatFooter");
                    let winbar = |s: &str| matches!(s, "WinBar" | "WinBarNC");

                    // Lines with purly border highlight groups are considered borders.
                    line.is_inferred_border = line
                        .line_fragments
                        .iter()
                        .map(|fragment| check_border(fragment, &float_border))
                        .all(|v| v);

                    // And also lines with a winbar highlight anywhere
                    line.is_inferred_border |= line
                        .line_fragments
                        .iter()
                        .map(|fragment| check_border(fragment, &winbar))
                        .any(|v| v)
                }

                self.actual_lines[row] = Some(Rc::new(RefCell::new(line)));
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
                    && bottom == u64::from(self.grid_size.height)
                    && left == 0
                    && right == u64::from(self.grid_size.width)
                    && cols == 0
                {
                    self.actual_lines.rotate(rows as isize);
                }
            }
            WindowDrawCommand::Clear => {
                tracy_zone!("clear_cmd", 0);
                self.scroll_delta = 0;
                self.scrollback_lines
                    .iter_mut()
                    .for_each(|line| *line = None);
                self.scroll_animation.reset();
            }
            WindowDrawCommand::Show => {
                tracy_zone!("show_cmd", 0);
                if self.hidden {
                    self.hidden = false;
                    self.position_t = 2.0; // We don't want to animate since the window is becoming visible,
                                           // so we set t to 2.0 to stop animations.
                    self.grid_start_position = self.grid_destination;
                    self.scroll_animation.reset();
                }
            }
            WindowDrawCommand::Hide => {
                tracy_zone!("hide_cmd", 0);
                self.hidden = true;
            }
            WindowDrawCommand::Viewport { scroll_delta } => {
                log::trace!("Handling Viewport {}", self.id);
                self.scroll_delta = scroll_delta.round() as isize;
            }
            WindowDrawCommand::ViewportMargins { top, bottom, .. } => {
                self.viewport_margins = ViewportMargins {
                    top,
                    bottom,
                    inferred: false,
                }
            }
            _ => {}
        };
    }

    fn infer_viewport_margins(&mut self) {
        if self.viewport_margins.inferred {
            self.viewport_margins.top = self
                .actual_lines
                .iter()
                .take_while(|line| {
                    if let Some(line) = line {
                        line.borrow().is_inferred_border
                    } else {
                        false
                    }
                })
                .count() as u64;
            self.viewport_margins.bottom = (self.viewport_margins.top as usize
                ..self.actual_lines.len())
                .rev()
                .map(|i| self.actual_lines[i].as_ref())
                .take_while(|line| {
                    if let Some(line) = line {
                        line.borrow().is_inferred_border
                    } else {
                        false
                    }
                })
                .count() as u64;
        }
    }

    pub fn flush(&mut self, renderer_settings: &RendererSettings) {
        self.infer_viewport_margins();

        // If the borders are changed, reset the scrollback to only fit the inner view
        let inner_range = self.viewport_margins.top as isize
            ..(self.actual_lines.len() - self.viewport_margins.bottom as usize) as isize;
        let inner_size = inner_range.len();
        let inner_view = self.actual_lines.iter_range(inner_range);
        if inner_size != self.scrollback_lines.len() / 2 {
            self.scrollback_lines.resize(2 * inner_size, None);
            self.scrollback_lines.clone_from_iter(inner_view);
            self.scroll_delta = 0;
            self.scroll_animation.reset();
            return;
        }

        let scroll_delta = self.scroll_delta;
        self.scrollback_lines.rotate(scroll_delta);

        self.scrollback_lines.clone_from_iter(inner_view);

        if scroll_delta != 0 {
            let mut scroll_offset = self.scroll_animation.position;

            let max_delta = self.scrollback_lines.len() - self.grid_size.height as usize;
            log::trace!(
                "Scroll offset {scroll_offset}, delta {scroll_delta}, max_delta {max_delta}"
            );
            // Do a limited scroll with empty lines when scrolling far
            if scroll_delta.unsigned_abs() > max_delta {
                let far_lines = renderer_settings
                    .scroll_animation_far_lines
                    .min(self.actual_lines.len() as u32) as isize;

                scroll_offset = -(far_lines * scroll_delta.signum()) as f32;
                let empty_lines = if scroll_delta > 0 {
                    -far_lines..0
                } else {
                    self.actual_lines.len() as isize..self.actual_lines.len() as isize + far_lines
                };
                for i in empty_lines {
                    self.scrollback_lines[i] = None;
                }
            // And even when scrolling in steps, we can't let it drift too far, since the
            // buffer size is limited
            } else {
                scroll_offset -= scroll_delta as f32;
                scroll_offset = scroll_offset.clamp(-(max_delta as f32), max_delta as f32);
            }
            self.scroll_animation.position = scroll_offset;
            log::trace!("Current scroll {scroll_offset}");
        }
        self.scroll_delta = 0;
    }

    pub fn prepare_lines(&mut self, grid_renderer: &mut GridRenderer) {
        let scroll_offset_lines = self.scroll_animation.position.floor() as isize;
        let height = self.grid_size.height as isize;
        if height == 0 {
            return;
        }
        let grid_scale = grid_renderer.grid_scale;

        let mut prepare_line = |line: &Rc<RefCell<Line>>| {
            let mut line = line.borrow_mut();
            if line.is_valid {
                return;
            }

            let mut recorder = PictureRecorder::new();

            let line_size = GridSize::new(self.grid_size.width, 1).cast() * grid_scale;
            let grid_rect = Rect::from_wh(line_size.width, line_size.height);
            let canvas = recorder.begin_recording(grid_rect, None);

            let mut has_transparency = false;
            let mut custom_background = false;

            for line_fragment in line.line_fragments.iter() {
                let LineFragment {
                    window_left,
                    width,
                    style,
                    ..
                } = line_fragment;
                let grid_position = (i32::try_from(*window_left).unwrap(), 0).into();
                let background_info = grid_renderer.draw_background(
                    canvas,
                    grid_position,
                    i32::try_from(*width).unwrap(),
                    style,
                );
                custom_background |= background_info.custom_color;
                has_transparency |= background_info.transparent;
            }
            let background_picture =
                custom_background.then_some(recorder.finish_recording_as_picture(None).unwrap());

            let canvas = recorder.begin_recording(grid_rect, None);
            let mut foreground_drawn = false;
            for line_fragment in &line.line_fragments {
                let LineFragment {
                    text,
                    window_left,
                    width,
                    style,
                } = line_fragment;
                let grid_position = (i32::try_from(*window_left).unwrap(), 0).into();

                foreground_drawn |= grid_renderer.draw_foreground(
                    canvas,
                    text,
                    grid_position,
                    i32::try_from(*width).unwrap(),
                    style,
                );
            }
            let foreground_picture =
                foreground_drawn.then_some(recorder.finish_recording_as_picture(None).unwrap());

            line.background_picture = background_picture;
            line.foreground_picture = foreground_picture;
            line.has_transparency = has_transparency;
            line.is_valid = true;
        };

        if !self.scrollback_lines.is_empty() {
            for line in self
                .scrollback_lines
                .iter_range_mut(scroll_offset_lines..scroll_offset_lines + height + 1)
                .flatten()
            {
                prepare_line(line)
            }
        }

        for line in self
            .actual_lines
            .iter_range_mut(0..self.viewport_margins.top as isize)
            .flatten()
        {
            prepare_line(line)
        }
        let actual_line_count = self.actual_lines.len() as isize;
        for line in self
            .actual_lines
            .iter_range_mut(
                actual_line_count - self.viewport_margins.bottom as isize..actual_line_count,
            )
            .flatten()
        {
            prepare_line(line)
        }
    }
}
