use std::sync::Arc;

use log::trace;

use palette::{named, Hsv, IntoColor, Srgba, WithAlpha};
use vide::{
    parley::{style::StyleProperty, Layout},
    Quad,
};

use crate::{
    editor::{Colors, Style},
    profiling::tracy_zone,
    renderer::{fonts::CachingShaper, RendererSettings},
    settings::*,
    units::{GridPos, GridScale, GridSize, PixelPos, PixelRect, PixelVec},
};

use super::fonts::font_options::FontOptions;

pub struct GridRenderer {
    pub shaper: CachingShaper,
    pub default_style: Arc<Style>,
    pub em_size: f32,
    pub grid_scale: GridScale,
    pub is_ready: bool,
}

/// Struct with named fields to be returned from draw_background
// This should probably be used
#[allow(unused)]
pub struct BackgroundInfo {
    pub custom_color: bool,
    pub transparent: bool,
}

#[derive(Clone)]
pub struct ForegroundLineFragment {
    pub layout: Layout<Srgba>,
    pub position: PixelPos<f32>,
}

impl GridRenderer {
    pub fn new(scale_factor: f64) -> Self {
        let mut shaper = CachingShaper::new(scale_factor as f32);
        let default_style = Arc::new(Style::new(Colors::new(
            Some(named::WHITE.with_alpha(255).into()),
            Some(named::BLACK.with_alpha(255).into()),
            Some(named::GREY.with_alpha(255).into()),
        )));
        let em_size = shaper.current_size();
        let font_dimensions = shaper.font_base_dimensions();

        GridRenderer {
            shaper,
            default_style,
            em_size,
            grid_scale: GridScale::new(font_dimensions),
            is_ready: false,
        }
    }

    pub fn font_names(&mut self) -> Vec<String> {
        self.shaper.font_names()
    }

    pub fn handle_scale_factor_update(&mut self, scale_factor: f64) {
        self.shaper.update_scale_factor(scale_factor as f32);
        self.update_font_dimensions();
    }

    pub fn update_font(&mut self, guifont_setting: &str) {
        self.shaper.update_font(guifont_setting);
        self.update_font_dimensions();
    }

    pub fn update_font_options(&mut self, options: FontOptions) {
        self.shaper.update_font_options(options);
        self.update_font_dimensions();
    }

    pub fn update_linespace(&mut self, linespace_setting: f32) {
        self.shaper.update_linespace(linespace_setting);
        self.update_font_dimensions();
    }

    fn update_font_dimensions(&mut self) {
        self.em_size = self.shaper.current_size();
        self.grid_scale = GridScale::new(self.shaper.font_base_dimensions());
        self.is_ready = true;
        trace!("Updated font dimensions: {:?}", self.grid_scale);
    }

    fn compute_text_region(&self, grid_position: GridPos<i32>, cell_width: i32) -> PixelRect<f32> {
        let pos = grid_position * self.grid_scale;
        let size = GridSize::new(cell_width, 1) * self.grid_scale;
        PixelRect::from_origin_and_size(pos, size)
    }

    pub fn get_default_background(&self) -> Srgba {
        self.default_style.colors.background.unwrap()
    }

    /// Draws a single background cell with the same style
    pub fn draw_background(
        &self,
        grid_position: GridPos<i32>,
        cell_width: i32,
        style: &Option<Arc<Style>>,
        quads: &mut Vec<Quad>,
    ) -> BackgroundInfo {
        tracy_zone!("draw_background");
        let debug = SETTINGS.get::<RendererSettings>().debug_renderer;
        if style.is_none() && !debug {
            return BackgroundInfo {
                custom_color: false,
                transparent: false,
            };
        }

        let region = self.compute_text_region(grid_position, cell_width);
        let style = style.as_ref().unwrap_or(&self.default_style);

        let color: Srgba = if debug {
            Hsv::new(rand::random::<f32>() * 360.0, 0.3, 0.3).into_color()
        } else {
            style.background(&self.default_style.colors)
        }
        .with_alpha(if style.blend > 0 {
            (100 - style.blend) as f32 / 100.0
        } else {
            1.0
        });

        let custom_color = color != self.default_style.colors.background.unwrap();
        if custom_color {
            let quad = Quad::new(*region.min.as_untyped(), *region.size().as_untyped(), color);
            quads.push(quad);
        }

        BackgroundInfo {
            custom_color,
            transparent: style.blend > 0,
        }
    }

    /// Draws some foreground text.
    /// Returns true if any text was actually drawn.
    pub fn draw_foreground(
        &mut self,
        text: &str,
        grid_position: GridPos<i32>,
        _cell_width: i32,
        style: &Option<Arc<Style>>,
        fragments: &mut Vec<ForegroundLineFragment>,
    ) -> bool {
        tracy_zone!("draw_foreground");
        let pos = grid_position * self.grid_scale;
        // let size = GridSize::new(cell_width, 0) * self.grid_scale;
        //let width = size.width;

        let style = style.as_ref().unwrap_or(&self.default_style);
        let mut drawn = false;

        // We don't want to clip text in the x position, only the y so we add a buffer of 1
        // character on either side of the region so that we clip vertically but not horizontally.
        // let clip_position = (grid_position.x.saturating_sub(1), grid_position.y).into();
        //let region = self.compute_text_region(clip_position, cell_width + 2);

        // TODO: Draw underline
        if let Some(_underline_style) = style.underline {
            /*
            let stroke_size = self.shaper.stroke_size();
            let underline_position = self.shaper.underline_position();
            let p1 = pos + PixelVec::new(0.0, underline_position);
            let p2 = pos + PixelVec::new(width, underline_position);

            self.draw_underline(canvas, style, underline_style, stroke_size, p1, p2);
            */
            drawn = true;
        }

        let color: Srgba = if SETTINGS.get::<RendererSettings>().debug_renderer {
            Hsv::new(rand::random::<f32>() * 360.0, 1.0, 1.0).into_color()
        } else {
            style.foreground(&self.default_style.colors)
        };

        // There's a lot of overhead for empty blobs in Skia, for some reason they never hit the
        // cache, so trim all the spaces
        let trimmed = text.trim_start();
        let leading_space_bytes = text.len() - trimmed.len();
        let leading_spaces = text[..leading_space_bytes].chars().count();
        let trimmed = trimmed.trim_end();
        let adjustment = PixelVec::new(
            leading_spaces as f32 * self.grid_scale.width(),
            self.shaper.baseline_offset(),
        );

        if !trimmed.is_empty() {
            tracy_zone!("draw_text_blob");
            let position = pos + adjustment;
            let layout = self.shaper.layout_with(trimmed, &style.into(), |builder| {
                builder.push_default(&StyleProperty::Brush(color));
            });
            fragments.push(ForegroundLineFragment { layout, position });
            drawn = true;
        }

        if style.strikethrough {
            /*
            let line_position = region.center().y;
            paint.set_color(style.special(&self.default_style.colors).to_color());
            canvas.draw_line(
                (pos.x, line_position),
                (pos.x + width, line_position),
                &paint,
            );
            */
            drawn = true;
        }

        drawn
    }

    /*
    fn draw_underline(
        &self,
        style: &Arc<Style>,
        underline_style: UnderlineStyle,
        stroke_size: f32,
        p1: PixelPos<f32>,
        p2: PixelPos<f32>,
    ) {
        tracy_zone!("draw_underline");
        canvas.save();

        let mut underline_paint = Paint::default();
        underline_paint.set_anti_alias(false);
        underline_paint.set_blend_mode(BlendMode::SrcOver);
        let underline_stroke_scale = SETTINGS.get::<RendererSettings>().underline_stroke_scale;
        // clamp to 1 and round to avoid aliasing issues
        let stroke_width = (stroke_size * underline_stroke_scale).max(1.).round();

        // offset y by width / 2 to align the *top* of the underline with p1 and p2
        // also round to avoid aliasing issues
        let p1 = (p1.x.round(), (p1.y + stroke_width / 2.).round());
        let p2 = (p2.x.round(), (p2.y + stroke_width / 2.).round());

        underline_paint
            .set_color(style.special(&self.default_style.colors).to_color())
            .set_stroke_width(stroke_width);

        match underline_style {
            UnderlineStyle::Underline => {
                underline_paint.set_path_effect(None);
                canvas.draw_line(p1, p2, &underline_paint);
            }
            UnderlineStyle::UnderDouble => {
                underline_paint.set_path_effect(None);
                canvas.draw_line(p1, p2, &underline_paint);
                let p1 = (p1.0, p1.1 + 2. * stroke_width);
                let p2 = (p2.0, p2.1 + 2. * stroke_width);
                canvas.draw_line(p1, p2, &underline_paint);
            }
            UnderlineStyle::UnderCurl => {
                let p1 = (p1.0, p1.1 + stroke_width);
                let p2 = (p2.0, p2.1 + stroke_width);
                underline_paint
                    .set_path_effect(None)
                    .set_anti_alias(true)
                    .set_style(skia_safe::paint::Style::Stroke);
                let mut path = Path::default();
                path.move_to(p1);
                let mut sin = -2. * stroke_width;
                let dx = self.grid_scale.width() / 2.;
                let count = ((p2.0 - p1.0) / dx).round();
                let dy = (p2.1 - p1.1) / count;
                for _ in 0..(count as i32) {
                    sin *= -1.;
                    path.r_quad_to((dx / 2., sin), (dx, dy));
                }
                canvas.draw_path(&path, &underline_paint);
            }
            UnderlineStyle::UnderDash => {
                underline_paint.set_path_effect(dash_path_effect::new(
                    &[6.0 * stroke_width, 2.0 * stroke_width],
                    0.0,
                ));
                canvas.draw_line(p1, p2, &underline_paint);
            }
            UnderlineStyle::UnderDot => {
                underline_paint.set_path_effect(dash_path_effect::new(
                    &[1.0 * stroke_width, 1.0 * stroke_width],
                    0.0,
                ));
                canvas.draw_line(p1, p2, &underline_paint);
            }
        }

        canvas.restore();
    }
    */
}
