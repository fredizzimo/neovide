use std::collections::BTreeMap;
use std::f32::consts::PI;
use std::sync::LazyLock;

use super::settings::{BoxDrawingMode, BoxDrawingSettings, ThicknessMultipliers};
use glamour::{Box2, Size2, Vector2};
use num::{Integer, ToPrimitive};
use skia_safe::{
    paint::Cap, BlendMode, Canvas, ClipOp, Color, Paint, PaintStyle, Path, PathEffect,
    PathFillType, Rect, Size,
};

use crate::renderer::fonts::font_options::points_to_pixels;
use crate::units::{to_skia_rect, PixelRect, PixelSize, PixelVec};
use crate::units::{Pixel, PixelPos};

trait LineAlignment {
    fn align_mid_line(self, stroke_width: f32) -> Self;
    fn align_outside(self) -> Self;
}

impl LineAlignment for f32 {
    fn align_mid_line(self, stroke_width: f32) -> Self {
        let rounded_stroke = stroke_width.round();
        let rounded_pos = self.round();
        if rounded_stroke.to_i64().unwrap().is_odd() {
            rounded_pos + 0.5
        } else {
            rounded_pos
        }
    }

    fn align_outside(self) -> Self {
        self.round()
    }
}

pub struct Context<'a> {
    canvas: &'a Canvas,
    settings: &'a BoxDrawingSettings,
    bounding_box: PixelRect<f32>,
    color_fg: Color,
}

impl<'a> Context<'a> {
    pub fn new(
        canvas: &'a Canvas,
        settings: &'a BoxDrawingSettings,
        bounding_box: PixelRect<f32>,
        color_fg: Color,
    ) -> Self {
        Context {
            canvas,
            settings,
            bounding_box,
            color_fg,
        }
    }

    fn get_stroke_width_pixels(&self, t: Thickness) -> f32 {
        let base_stroke_size =
            self.bounding_box.size().width * self.settings.stroke_width_ratio.unwrap_or(0.15);
        points_to_pixels(t.scale_factor(self.settings.thickness_multipliers) * base_stroke_size)
            .round()
            .max(1.0)
    }

    fn fg_paint(&self) -> Paint {
        let mut fg = Paint::default();
        fg.set_style(PaintStyle::Fill);
        fg.set_color(self.color_fg);
        fg.set_blend_mode(BlendMode::Src);
        fg.set_anti_alias(false);
        fg
    }

    fn draw_fg_line1(&self, o: Orientation, which_half: HalfSelector) {
        self.draw_line(
            o,
            which_half,
            LineSelector::Middle,
            self.get_stroke_width_pixels(Thickness::Level1),
            self.color_fg,
            None,
        );
    }

    fn draw_fg_line3(&self, o: Orientation, which_half: HalfSelector) {
        self.draw_line(
            o,
            which_half,
            LineSelector::Middle,
            self.get_stroke_width_pixels(Thickness::Level3),
            self.color_fg,
            None,
        );
    }

    fn get_dash_effect(&self, o: Orientation, num_gaps: u8) -> PathEffect {
        let Size2 {
            width: cell_width,
            height: cell_height,
        } = self.bounding_box.size();
        let total = f32::round(match o {
            Orientation::Horizontal => cell_width,
            Orientation::Vertical => cell_height,
        }) as i32;

        let gap_sz = 2;
        let all_gaps_use = (num_gaps as i32) * gap_sz;
        let num_dashes = num_gaps as i32 + 1;
        let dash_sz = (total - all_gaps_use) / num_dashes;
        PathEffect::dash(&[dash_sz as f32, gap_sz as f32], 0.)
            .expect("new path effect ptr to be not null")
    }

    fn draw_arrow(&self, side: Side) {
        let mut path = Path::default();
        let min = self.bounding_box.min.round();
        let max = self.bounding_box.max.round();
        let mid = self.bounding_box.center().round();
        path.set_fill_type(PathFillType::Winding);
        match side {
            Side::Left => {
                path.move_to((max.x, min.y));
                path.line_to((min.x, mid.y));
                path.line_to((max.x, max.y));
            }
            Side::Right => {
                path.move_to((min.x, min.y));
                path.line_to((max.x, mid.y));
                path.line_to((min.x, max.y));
            }
        }
        path.close();
        let mut fg = self.fg_paint();
        fg.set_style(PaintStyle::Fill);
        fg.set_anti_alias(true);
        self.canvas.draw_path(&path, &fg);
    }

    fn draw_quarter_triangle(&self, corner: Corner, height: Height) {
        let mut path = Path::default();
        let min = self.bounding_box.min.round();
        let max = self.bounding_box.max.round();
        let mid = self.bounding_box.center().round();
        path.set_fill_type(PathFillType::Winding);
        match corner {
            Corner::TopLeft => {
                path.move_to((min.x, min.y));
                path.line_to((max.x, min.y));
                path.line_to((
                    min.x,
                    match height {
                        Height::Tall => max.y,
                        Height::Short => mid.y,
                    },
                ));
            }
            Corner::TopRight => {
                path.move_to((max.x, min.y));
                path.line_to((
                    max.x,
                    match height {
                        Height::Tall => max.y,
                        Height::Short => mid.y,
                    },
                ));
                path.line_to((min.x, min.y));
            }
            Corner::BottomRight => {
                path.move_to((max.x, max.y));
                path.line_to((min.x, max.y));
                path.line_to((
                    max.x,
                    match height {
                        Height::Tall => min.y,
                        Height::Short => mid.y,
                    },
                ));
            }
            Corner::BottomLeft => {
                path.move_to((min.x, max.y));
                path.line_to((max.x, max.y));
                path.line_to((
                    min.x,
                    match height {
                        Height::Tall => min.y,
                        Height::Short => mid.y,
                    },
                ));
            }
        }
        path.close();
        let mut fg = self.fg_paint();
        fg.set_style(PaintStyle::Fill);
        fg.set_anti_alias(true);
        self.canvas.draw_path(&path, &fg);
    }

    fn draw_half_cross_line(&self, start_corner: Corner) {
        let mut path = Path::default();
        let min = self.bounding_box.min.round();
        let max = self.bounding_box.max.round();
        let mid = self.bounding_box.center().round();
        match start_corner {
            Corner::TopLeft => {
                path.move_to((min.x, min.y));
                path.line_to((max.x, mid.y));
            }
            Corner::TopRight => {
                path.move_to((max.x, min.y));
                path.line_to((min.x, mid.y));
            }
            Corner::BottomRight => {
                path.move_to((max.x, max.y));
                path.line_to((min.x, mid.y));
            }
            Corner::BottomLeft => {
                path.move_to((min.x, max.y));
                path.line_to((max.x, mid.y));
            }
        }
        let mut fg = self.fg_paint();
        fg.set_style(PaintStyle::Stroke);
        fg.set_stroke_width(self.get_stroke_width_pixels(Thickness::Level2));
        fg.set_anti_alias(true);
        self.canvas.draw_path(&path, &fg);
    }

    fn draw_d(&self, side: Side, fill: PaintStyle, close_path: bool) {
        let mut path = Path::default();
        let bounds = self.bounding_box;
        let stroke_width = self.get_stroke_width_pixels(Thickness::Level2);
        let mut radius = (bounds.size().width).min(bounds.size().height / 2.0);
        // Leave a small gap between the circles, and also allow them to move a bit to the side
        // depending on the pixel alignment of the cell.
        radius -= 1.0;
        if fill == PaintStyle::Stroke {
            radius -= stroke_width / 2.0;
        }
        let diameter = PixelSize::new(radius * 2.0, radius * 2.0);

        match side {
            Side::Left => {
                let origin = PixelPos::new(
                    bounds.max.x.align_outside() - radius,
                    bounds.center().y - radius,
                );
                let rect = to_skia_rect(&PixelRect::from_origin_and_size(origin, diameter));
                let start_angle = 90.0;
                let sweep_angle = 180.0;
                path.arc_to(rect, start_angle, sweep_angle, true);
            }
            Side::Right => {
                let origin = PixelPos::new(
                    bounds.min.x.align_outside() - radius,
                    bounds.center().y - radius,
                );
                let rect = to_skia_rect(&PixelRect::from_origin_and_size(origin, diameter));
                let start_angle = 270.0;
                let sweep_angle = 180.0;
                path.arc_to(rect, start_angle, sweep_angle, true);
            }
        }
        if close_path {
            path.close();
        }
        let mut fg = self.fg_paint();
        fg.set_stroke_width(stroke_width);
        fg.set_style(fill);
        fg.set_anti_alias(true);
        self.canvas.draw_path(&path, &fg);
    }

    fn draw_cross_line(&self, side: Side) {
        let stroke_width = self.get_stroke_width_pixels(Thickness::Level2);
        let min = self.bounding_box.min;
        let max = self.bounding_box.max;
        // The bounding box needs to be extended slightly to the sides, so that thick lines and
        // anti-aliasing can be drawn outside of it. stroke_width is a bit too much, but we don't
        // know how much the anti-aliasing uses.
        let mut extended_bounding_box = self.bounding_box;
        extended_bounding_box.min.x -= stroke_width;
        extended_bounding_box.max.x += stroke_width;
        // This is stupid, but skia does not allow overriding a clip rect so assume that the only
        // saved state is the previous clip rect Don't restore the state afterwards, it will be
        // done outside of this.
        self.canvas.restore();
        self.canvas.save();
        self.canvas
            .clip_rect(to_skia_rect(&extended_bounding_box), None, Some(false));
        let mut fg = self.fg_paint();
        fg.set_stroke_width(stroke_width);
        fg.set_style(PaintStyle::Stroke);
        fg.set_anti_alias(true);
        fg.set_stroke_cap(Cap::Square);
        match side {
            Side::Left => {
                self.canvas.draw_line((min.x, min.y), (max.x, max.y), &fg);
            }
            Side::Right => {
                self.canvas.draw_line((max.x, min.y), (min.x, max.y), &fg);
            }
        }
    }

    fn draw_progress(&self, section: Section, fill: PaintStyle) {
        let bounds = to_skia_rect(&self.bounding_box);
        let t: f32 = self.get_stroke_width_pixels(Thickness::Level1);
        let clip_rect = match section {
            Section::Left => bounds.with_inset((0., t)).with_offset((t, 0.)),
            Section::Middle => bounds.with_inset((0., t)),
            Section::Right => bounds.with_inset((0., t)).with_offset((-t, 0.)),
        };
        let mut fg = self.fg_paint();
        fg.set_style(PaintStyle::Fill);
        self.canvas.save();
        {
            self.canvas
                .clip_rect(clip_rect, ClipOp::Difference, Some(false));
            self.canvas.draw_rect(bounds, &fg);
        }
        self.canvas.restore();
        if fill == PaintStyle::Fill {
            let gap_factor: f32 = 3.0;
            let gap = gap_factor * t;
            let inner_rect = clip_rect.with_inset((0., gap)).with_offset(match section {
                Section::Left => (gap, 0.),
                Section::Middle => (0., 0.),
                Section::Right => (-gap, 0.),
            });
            self.canvas.draw_rect(inner_rect, &fg);
        }
    }

    fn draw_double_line(&self, o: Orientation, which_half: HalfSelector) {
        let stroke_width = self.get_stroke_width_pixels(Thickness::Level1);
        self.draw_line(
            o,
            which_half,
            LineSelector::Left,
            stroke_width,
            self.color_fg,
            None,
        );
        self.draw_line(
            o,
            which_half,
            LineSelector::Right,
            stroke_width,
            self.color_fg,
            None,
        );
    }

    // (min.x, min.y)                      (max.x, min.y)
    //      o------------------------------o
    //      |                              |
    //      |                              |
    //      |              o               |
    //      |         (mid.x, mid.y)       |
    //      |                              |
    //      o------------------------------o
    // (min.x, max.y)                      (max.x, max.y)
    fn draw_line(
        &self,
        o: Orientation,
        which_half: HalfSelector,
        which_line: LineSelector,
        stroke_width: f32,
        color: Color,
        effect: impl Into<Option<PathEffect>>,
    ) {
        let min = self.bounding_box.min;
        let max = self.bounding_box.max;
        let mid = self.bounding_box.center();
        let offset = match which_line {
            LineSelector::Left => -stroke_width,
            LineSelector::Right => stroke_width,
            LineSelector::Middle => 0.0,
        };
        let double_offset = stroke_width / 2.0;
        let (p1, p2) = match (o, which_half) {
            (Orientation::Horizontal, HalfSelector::First) => {
                ((min.x, mid.y + offset), (mid.x, mid.y + offset))
            }
            (Orientation::Horizontal, HalfSelector::FirstDouble) => (
                (min.x, mid.y + offset),
                (mid.x - double_offset, mid.y + offset),
            ),
            (Orientation::Horizontal, HalfSelector::Last) => {
                ((mid.x, mid.y + offset), (max.x, mid.y + offset))
            }
            (Orientation::Horizontal, HalfSelector::LastDouble) => (
                (mid.x + double_offset, mid.y + offset),
                (max.x, mid.y + offset),
            ),
            (Orientation::Horizontal, HalfSelector::Both) => {
                ((min.x, mid.y + offset), (max.x, mid.y + offset))
            }
            (Orientation::Vertical, HalfSelector::First) => {
                ((mid.x + offset, min.y), (mid.x + offset, mid.y))
            }
            (Orientation::Vertical, HalfSelector::FirstDouble) => (
                (mid.x + offset, min.y),
                (mid.x + offset, mid.y - double_offset),
            ),
            (Orientation::Vertical, HalfSelector::Last) => {
                ((mid.x + offset, mid.y), (mid.x + offset, max.y))
            }
            (Orientation::Vertical, HalfSelector::LastDouble) => (
                (mid.x + offset, mid.y + double_offset),
                (mid.x + offset, max.y),
            ),
            (Orientation::Vertical, HalfSelector::Both) => {
                ((mid.x + offset, min.y), (mid.x + offset, max.y))
            }
        };
        let mut paint = self.fg_paint();
        paint.set_style(PaintStyle::Stroke);
        paint.set_stroke_width(stroke_width);
        paint.set_color(color);
        if let Some(effect) = effect.into() {
            paint.set_path_effect(effect);
            let mut path = Path::default();
            path.move_to(p1);
            path.line_to(p2);
            self.canvas.draw_path(&path, &paint);
        } else {
            self.canvas.draw_line(p1, p2, &paint);
        }
    }

    fn draw_eighth(&self, o: Orientation, which: impl std::ops::RangeBounds<u8>) {
        let min = self.bounding_box.min;
        let Size2 { width, height } = self.bounding_box.size();
        let (start, num_steps) = {
            let start_idx = match which.start_bound() {
                std::ops::Bound::Included(&s) => s,
                std::ops::Bound::Excluded(&s) => s.saturating_add(1).min(7),
                std::ops::Bound::Unbounded => 0,
            };
            let end_idx = match which.end_bound() {
                std::ops::Bound::Included(&s) => s.saturating_add(1).min(8),
                std::ops::Bound::Excluded(&s) => s,
                std::ops::Bound::Unbounded => 8,
            };

            (start_idx as f32, end_idx.saturating_sub(start_idx) as f32)
        };
        let rect = match o {
            Orientation::Horizontal => {
                let step = height / 8.0;
                let y1 = min.y + start * step;
                Rect::from_point_and_size((min.x, y1), Size::new(width, num_steps * step))
            }
            Orientation::Vertical => {
                let step = width / 8.0;
                let x1 = min.x + start * step;
                Rect::from_point_and_size((x1, min.y), Size::new(num_steps * step, height))
            }
        };
        let mut paint = self.fg_paint();
        paint.set_style(PaintStyle::Fill);
        self.canvas.draw_rect(rect, &paint);
    }

    // Test 1:
    // ░
    // ░░░░░░░░░░
    // ░░░░░░░░░░
    // ░░░░░░░░░░
    // Test 2:
    // ▒▒▒▒▒▒▒▒▒▒
    // ▒▒▒▒▒▒▒▒▒▒
    // Test 3:
    // ▓▓▓▓▓▓▓▓▓▓
    // ▓▓▓▓▓▓▓▓▓▓
    // Test 4:
    // 🮌
    // 🮌
    // Test 5:
    // 🮍
    // 🮍
    // Test 6:
    // 🮎🮎🮎🮎🮎🮎🮎🮎🮎🮎
    // Test 7:
    // 🮏🮏🮏🮏🮏🮏🮏🮏🮏🮏
    // Test 8:
    // 🮐🮐🮐🮐🮐🮐🮐🮐🮐🮐
    // 🮐🮐🮐🮐🮐🮐🮐🮐🮐🮐
    // Test 9:
    // 🮑🮑🮑🮑🮑🮑🮑🮑🮑🮑
    // 🮑🮑🮑🮑🮑🮑🮑🮑🮑🮑
    // Test 10:
    // 🮒🮒🮒🮒🮒🮒🮒🮒🮒🮒
    // 🮒🮒🮒🮒🮒🮒🮒🮒🮒🮒
    // Test 11:
    // 🮓🮓🮓🮓🮓🮓🮓🮓🮓🮓
    // 🮓🮓🮓🮓🮓🮓🮓🮓🮓🮓
    // Test 12:
    // 🮔🮔🮔🮔🮔🮔🮔🮔🮔🮔
    // 🮔🮔🮔🮔🮔🮔🮔🮔🮔🮔
    fn draw_shade(
        &self,
        o: Orientation,
        which_half: HalfSelector,
        shade: Shade,
        mirror: MirrorMode,
        color_mode: ColorMode,
    ) {
        self.canvas.save();
        self.canvas.clip_rect(
            {
                let mut rect = to_skia_rect(&self.bounding_box);
                match which_half {
                    HalfSelector::First => match o {
                        Orientation::Horizontal => rect.right = rect.center_x(),
                        Orientation::Vertical => rect.bottom = rect.center_y(),
                    },
                    HalfSelector::Last => match o {
                        Orientation::Horizontal => rect.left = rect.center_x(),
                        Orientation::Vertical => rect.top = rect.center_y(),
                    },
                    _ => {}
                }
                rect
            },
            ClipOp::Intersect,
            Some(false),
        );

        const NUM_STRIPES: i32 = 4;
        let tile_sz = self.bounding_box.size();
        let stripe_gap = tile_sz.height / NUM_STRIPES as f32;
        let mut rotation_degrees = f32::atan(stripe_gap / tile_sz.width) * 180.0 / PI;
        let stripe_height = match shade {
            Shade::Light => 1.0,
            Shade::Medium => 2.0,
            Shade::Dark => 3.0,
        };
        let mut fg = self.fg_paint();
        fg.set_style(PaintStyle::Fill);
        fg.set_anti_alias(true);
        match color_mode {
            ColorMode::Normal => (),
            ColorMode::Inverted => {
                // TODO: fix this
                // self.canvas.draw_paint(&fg);
                // fg.set_color(self.color_bg);
            }
        }

        {
            let stripe_sz = (3.0 * tile_sz.width.max(tile_sz.height), stripe_height);
            match mirror {
                MirrorMode::Normal => (),
                MirrorMode::Mirror => {
                    rotation_degrees = 180.0 - rotation_degrees;
                    self.canvas.translate((tile_sz.width, 0.0));
                }
            };
            let top_left = self.bounding_box.min;
            for i in -1..NUM_STRIPES + 1 {
                let (dx, dy) = (0., i as f32 * stripe_gap);
                let stripe_top_left = top_left.translate(Vector2::new(dx, dy));
                self.canvas.save();
                self.canvas
                    .rotate(rotation_degrees, Some(stripe_top_left.to_tuple().into()));
                self.canvas.draw_rect(
                    Rect::from_point_and_size(stripe_top_left.to_tuple(), stripe_sz),
                    &fg,
                );
                self.canvas.restore();
            }
        }
        self.canvas.restore();
    }

    fn triangle_path(&self, corner: Corner) -> Path {
        let mut path = Path::default();
        let bb = to_skia_rect(&self.bounding_box);
        let top_left = (bb.left, bb.top);
        let top_right = (bb.right, bb.top);
        let bottom_left = (bb.left, bb.bottom);
        let bottom_right = (bb.right, bb.bottom);
        match corner {
            Corner::TopLeft => {
                path.move_to(top_left);
                path.line_to(top_right);
                path.line_to(bottom_left);
            }
            Corner::TopRight => {
                path.move_to(top_right);
                path.line_to(top_left);
                path.line_to(bottom_right);
            }
            Corner::BottomRight => {
                path.move_to(bottom_right);
                path.line_to(top_right);
                path.line_to(bottom_left);
            }
            Corner::BottomLeft => {
                path.move_to(bottom_left);
                path.line_to(top_left);
                path.line_to(bottom_right);
            }
        }
        path.close();
        path
    }

    fn draw_rounded_corner(&self, corner: Corner) {
        let stroke_width = self.get_stroke_width_pixels(Thickness::Level1);
        let mut path = Path::new();
        let (mut x1, mut y1, mut x2, mut y2) = match corner {
            Corner::TopLeft => (
                self.bounding_box.max.x,
                self.bounding_box.center().y,
                self.bounding_box.center().x,
                self.bounding_box.max.y,
            ),
            Corner::TopRight => (
                self.bounding_box.min.x,
                self.bounding_box.center().y,
                self.bounding_box.center().x,
                self.bounding_box.max.y,
            ),
            Corner::BottomLeft => (
                self.bounding_box.max.x,
                self.bounding_box.center().y,
                self.bounding_box.center().x,
                self.bounding_box.min.y,
            ),
            Corner::BottomRight => (
                self.bounding_box.min.x,
                self.bounding_box.center().y,
                self.bounding_box.center().x,
                self.bounding_box.min.y,
            ),
        };
        x1 = x1.align_outside();
        y1 = y1.align_mid_line(stroke_width);
        x2 = x2.align_mid_line(stroke_width);
        y2 = y2.align_outside();
        let radius = (x1 - x2).abs();
        path.move_to((x1, y1));
        path.arc_to_tangent((x2, y1), (x2, y2), radius);
        path.line_to((x2, y2));
        let mut fg = self.fg_paint();
        fg.set_anti_alias(true);
        fg.set_style(PaintStyle::Stroke);
        fg.set_stroke_width(stroke_width);
        self.canvas.draw_path(&path, &fg);
    }

    fn draw_t_joint(
        &self,
        north: impl Into<Option<Thickness>>,
        east: impl Into<Option<Thickness>>,
        south: impl Into<Option<Thickness>>,
        west: impl Into<Option<Thickness>>,
    ) {
        let fg = self.color_fg;
        for (t, o, h) in [
            (north.into(), Orientation::Vertical, HalfSelector::First),
            (east.into(), Orientation::Horizontal, HalfSelector::Last),
            (south.into(), Orientation::Vertical, HalfSelector::Last),
            (west.into(), Orientation::Horizontal, HalfSelector::First),
        ] {
            if let Some(t) = t {
                self.draw_line(
                    o,
                    h,
                    LineSelector::Middle,
                    self.get_stroke_width_pixels(t),
                    fg,
                    None,
                );
            }
        }
    }

    fn draw_corner(&self, corner: Corner, horiz_t: Thickness, vert_t: Thickness) {
        let horiz_t = self.get_stroke_width_pixels(horiz_t);
        let vert_t = self.get_stroke_width_pixels(vert_t);
        let color = self.color_fg;
        let min = self.bounding_box.min;
        let max = self.bounding_box.max;
        let mid = self.bounding_box.center();
        let mut fg = self.fg_paint();
        fg.set_style(PaintStyle::Stroke);
        fg.set_color(color);

        let aligned_mid = match corner {
            Corner::TopLeft | Corner::TopRight => {
                mid.translate(Vector2::from((0.0, horiz_t * -0.5)))
            }
            Corner::BottomLeft | Corner::BottomRight => {
                mid.translate(Vector2::from((0.0, horiz_t * 0.5)))
            }
        };
        match corner {
            Corner::TopLeft => {
                fg.set_stroke_width(horiz_t);
                self.canvas.draw_line(mid.to_tuple(), (max.x, mid.y), &fg);
                fg.set_stroke_width(vert_t);
                self.canvas
                    .draw_line(aligned_mid.to_tuple(), (mid.x, max.y), &fg);
            }
            Corner::TopRight => {
                fg.set_stroke_width(horiz_t);
                self.canvas.draw_line((min.x, mid.y), mid.to_tuple(), &fg);
                fg.set_stroke_width(vert_t);
                self.canvas
                    .draw_line(aligned_mid.to_tuple(), (mid.x, max.y), &fg);
            }
            Corner::BottomRight => {
                fg.set_stroke_width(horiz_t);
                self.canvas.draw_line((min.x, mid.y), mid.to_tuple(), &fg);
                fg.set_stroke_width(vert_t);
                self.canvas
                    .draw_line((mid.x, min.y), aligned_mid.to_tuple(), &fg);
            }
            Corner::BottomLeft => {
                fg.set_stroke_width(horiz_t);
                self.canvas.draw_line(mid.to_tuple(), (max.x, mid.y), &fg);
                fg.set_stroke_width(vert_t);
                self.canvas
                    .draw_line((mid.x, min.y), aligned_mid.to_tuple(), &fg);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Height {
    Tall,
    Short,
}

#[derive(Clone, Copy)]
enum Corner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy)]
enum Section {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy)]
enum Thickness {
    Level1,
    Level2,
    Level3,
}

impl Thickness {
    fn scale_factor(self, mult: Option<ThicknessMultipliers>) -> f32 {
        let ThicknessMultipliers(mult) = mult.unwrap_or_default();
        match self {
            Thickness::Level1 => mult[0],
            Thickness::Level2 => mult[1],
            Thickness::Level3 => mult[2],
        }
    }
}

#[derive(Clone, Copy)]
enum Orientation {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy)]
enum HalfSelector {
    First,
    FirstDouble,
    Last,
    LastDouble,
    Both,
}

#[derive(Clone, Copy)]
enum LineSelector {
    Middle,
    Left,
    Right,
}

#[derive(Clone, Copy)]
enum Shade {
    Light,
    Medium,
    Dark,
}

#[derive(Clone, Copy)]
enum MirrorMode {
    Normal,
    Mirror,
}

#[derive(Clone, Copy)]
enum ColorMode {
    Normal,
    Inverted,
}

type BoxDrawFn = Box<dyn Fn(&Context) + Send + Sync>;

static BOX_CHARS: LazyLock<BTreeMap<char, BoxDrawFn>> = LazyLock::new(|| {
    use Orientation::*;
    let mut m: BTreeMap<char, BoxDrawFn> = BTreeMap::new();

    macro_rules! box_char {
        ($($chars:literal),* -> $func:expr) => {
            for ch in &[$($chars),*] {
                m.insert(*ch, Box::new($func));
            }
        };
    }

    box_char!['─' -> |ctx: &Context| {
        ctx.draw_fg_line1(Horizontal, HalfSelector::Both);
    }];
    box_char!['━' -> |ctx: &Context| {
        ctx.draw_fg_line3(Horizontal, HalfSelector::Both);
    }];
    box_char!['│' -> |ctx: &Context| {
        ctx.draw_fg_line1(Vertical, HalfSelector::Both);
    }];
    box_char!['┃' -> |ctx: &Context| {
        ctx.draw_fg_line3(Vertical, HalfSelector::Both);
    }];
    box_char!['╌' -> |ctx: &Context| {
        ctx.draw_line(
            Horizontal,
            HalfSelector::Both,
            LineSelector::Middle,
            ctx.get_stroke_width_pixels(Thickness::Level1),
            ctx.color_fg,
            ctx.get_dash_effect(Horizontal, 1),
        );
    }];
    box_char!['╍' -> |ctx: &Context| {
        ctx.draw_line(
            Horizontal,
            HalfSelector::Both,
            LineSelector::Middle,
            ctx.get_stroke_width_pixels(Thickness::Level3),
            ctx.color_fg,
            ctx.get_dash_effect(Horizontal, 1),
        );
    }];
    box_char!['┅' -> |ctx: &Context| {
        ctx.draw_line(
            Horizontal,
            HalfSelector::Both,
            LineSelector::Middle,
            ctx.get_stroke_width_pixels(Thickness::Level3),
            ctx.color_fg,
            ctx.get_dash_effect(Horizontal, 2),
        );
    }];
    box_char!['┈' -> |ctx: &Context| {
        ctx.draw_line(
            Horizontal,
            HalfSelector::Both,
            LineSelector::Middle,
            ctx.get_stroke_width_pixels(Thickness::Level1),
            ctx.color_fg,
            ctx.get_dash_effect(Horizontal, 3),
        );
    }];
    box_char!['┉' -> |ctx: &Context| {
        ctx.draw_line(
            Horizontal,
            HalfSelector::Both,
            LineSelector::Middle,
            ctx.get_stroke_width_pixels(Thickness::Level3),
            ctx.color_fg,
            ctx.get_dash_effect(Horizontal, 3),
        );
    }];

    box_char!['╎' -> |ctx: &Context| {
        ctx.draw_line(
            Vertical,
            HalfSelector::Both,
            LineSelector::Middle,
            ctx.get_stroke_width_pixels(Thickness::Level1),
            ctx.color_fg,
            ctx.get_dash_effect(Vertical, 1),
        );
    }];
    box_char!['╏' -> |ctx: &Context| {
        ctx.draw_line(
            Vertical,
            HalfSelector::Both,
            LineSelector::Middle,
            ctx.get_stroke_width_pixels(Thickness::Level3),
            ctx.color_fg,
            ctx.get_dash_effect(Vertical, 1),
        );
    }];
    box_char!['┆' -> |ctx: &Context| {
        ctx.draw_line(
            Vertical,
            HalfSelector::Both,
            LineSelector::Middle,
            ctx.get_stroke_width_pixels(Thickness::Level1),
            ctx.color_fg,
            ctx.get_dash_effect(Vertical, 2),
        );
    }];
    box_char!['┇' -> |ctx: &Context| {
        ctx.draw_line(
            Vertical,
            HalfSelector::Both,
            LineSelector::Middle,
            ctx.get_stroke_width_pixels(Thickness::Level3),
            ctx.color_fg,
            ctx.get_dash_effect(Vertical, 2),
        );
    }];
    box_char!['┊' -> |ctx: &Context| {
        ctx.draw_line(
            Vertical,
            HalfSelector::Both,
            LineSelector::Middle,
            ctx.get_stroke_width_pixels(Thickness::Level1),
            ctx.color_fg,
            ctx.get_dash_effect(Vertical, 3),
        );
    }];
    box_char!['┋' -> |ctx: &Context| {
        ctx.draw_line(
            Vertical,
            HalfSelector::Both,
            LineSelector::Middle,
            ctx.get_stroke_width_pixels(Thickness::Level3),
            ctx.color_fg,
            ctx.get_dash_effect(Vertical, 3),
        );
    }];

    // Half lines
    box_char!['╴' -> |ctx: &Context| {
        ctx.draw_fg_line1(Horizontal, HalfSelector::First);
    }];
    box_char!['╵' -> |ctx: &Context| {
        ctx.draw_fg_line1(Vertical, HalfSelector::First);
    }];
    box_char!['╶' -> |ctx: &Context| {
        ctx.draw_fg_line1(Horizontal, HalfSelector::Last);
    }];
    box_char!['╷' -> |ctx: &Context| {
        ctx.draw_fg_line1(Vertical, HalfSelector::Last);
    }];
    box_char!['╸' -> |ctx: &Context| {
        ctx.draw_fg_line3(Horizontal, HalfSelector::First);
    }];
    box_char!['╹' -> |ctx: &Context| {
        ctx.draw_fg_line3(Vertical, HalfSelector::First);
    }];
    box_char!['╺' -> |ctx: &Context| {
        ctx.draw_fg_line3(Horizontal, HalfSelector::Last);
    }];
    box_char!['╻' -> |ctx: &Context| {
        ctx.draw_fg_line3(Vertical, HalfSelector::Last);
    }];
    box_char!['╼' -> |ctx: &Context| {
        ctx.draw_fg_line1(Horizontal, HalfSelector::First);
        ctx.draw_fg_line3(Horizontal, HalfSelector::Last);
    }];
    box_char!['╽' -> |ctx: &Context| {
        ctx.draw_fg_line1(Vertical, HalfSelector::First);
        ctx.draw_fg_line3(Vertical, HalfSelector::Last);
    }];
    box_char!['╾' -> |ctx: &Context| {
        ctx.draw_fg_line3(Horizontal, HalfSelector::First);
        ctx.draw_fg_line1(Horizontal, HalfSelector::Last);
    }];
    box_char!['╿' -> |ctx: &Context| {
        ctx.draw_fg_line3(Vertical, HalfSelector::First);
        ctx.draw_fg_line1(Vertical, HalfSelector::Last);
    }];

    box_char!['' -> |ctx: &Context| {
        ctx.draw_arrow(Side::Right);
    }];
    box_char!['' -> |ctx: &Context| {
        ctx.draw_quarter_triangle(Corner::TopRight, Height::Short);
        ctx.draw_quarter_triangle(Corner::BottomRight, Height::Short);
    }];
    box_char!['' -> |ctx: &Context| {
        ctx.draw_arrow(Side::Left);
    }];
    box_char!['' -> |ctx: &Context| {
        ctx.draw_quarter_triangle(Corner::TopLeft, Height::Short);
        ctx.draw_quarter_triangle(Corner::BottomLeft, Height::Short);
    }];
    box_char!['' -> |ctx: &Context| {
        ctx.draw_half_cross_line(Corner::TopLeft);
        ctx.draw_half_cross_line(Corner::BottomLeft);
    }];
    box_char!['' -> |ctx: &Context| {
        ctx.draw_half_cross_line(Corner::TopRight);
        ctx.draw_half_cross_line(Corner::BottomRight);
    }];
    box_char!['', '◗' -> |ctx: &Context| {
        ctx.draw_d(Side::Right, PaintStyle::Fill, true);
    }];
    box_char!['', '◖' -> |ctx: &Context| {
        ctx.draw_d(Side::Left, PaintStyle::Fill, true);
    }];
    box_char!['' -> |ctx: &Context| {
        ctx.draw_d(Side::Right, PaintStyle::Stroke, false);
    }];
    box_char!['' -> |ctx: &Context| {
        ctx.draw_d(Side::Left, PaintStyle::Stroke, false);
    }];

    box_char!['', '', '╲' -> |ctx: &Context| {
        ctx.draw_cross_line(Side::Left);
    }];
    box_char!['', '', '╱' -> |ctx: &Context| {
        ctx.draw_cross_line(Side::Right);
    }];
    box_char!['╳' -> |ctx: &Context| {
        ctx.draw_cross_line(Side::Left);
        ctx.draw_cross_line(Side::Right);
    }];

    box_char!['', '◣' -> |ctx: &Context| {
        ctx.draw_quarter_triangle(Corner::BottomLeft, Height::Tall);
    }];
    box_char!['', '◢' -> |ctx: &Context| {
        ctx.draw_quarter_triangle(Corner::BottomRight, Height::Tall);
    }];
    box_char!['', '◤' -> |ctx: &Context| {
        ctx.draw_quarter_triangle(Corner::TopLeft, Height::Tall);
    }];
    box_char!['', '◥' -> |ctx: &Context| {
        ctx.draw_quarter_triangle(Corner::TopRight, Height::Tall);
    }];

    // 
    box_char!['' -> |ctx: &Context| {
        ctx.draw_progress(Section::Left, PaintStyle::Stroke);
    }];
    box_char!['' -> |ctx: &Context| {
        ctx.draw_progress(Section::Middle, PaintStyle::Stroke);
    }];
    box_char!['' -> |ctx: &Context| {
        ctx.draw_progress(Section::Right, PaintStyle::Stroke);
    }];
    // 
    box_char!['' -> |ctx: &Context| {
        ctx.draw_progress(Section::Left, PaintStyle::Fill);
    }];
    box_char!['' -> |ctx: &Context| {
        ctx.draw_progress(Section::Middle, PaintStyle::Fill);
    }];
    box_char!['' -> |ctx: &Context| {
        ctx.draw_progress(Section::Right, PaintStyle::Fill);
    }];

    // double lines
    box_char!('═' -> |ctx: &Context|{
        ctx.draw_double_line(Horizontal, HalfSelector::Both);
    });
    box_char!('║' -> |ctx: &Context|{
        ctx.draw_double_line(Vertical, HalfSelector::Both);
    });
    box_char!['╞' -> |ctx: &Context| {
        ctx.draw_fg_line1(Vertical, HalfSelector::Both);
        ctx.draw_double_line(Horizontal, HalfSelector::Last);
    }];
    box_char!['╡' -> |ctx: &Context| {
        ctx.draw_fg_line1(Vertical, HalfSelector::Both);
        ctx.draw_double_line(Horizontal, HalfSelector::First);
    }];
    box_char!['╥' -> |ctx: &Context| {
        ctx.draw_fg_line1(Horizontal, HalfSelector::Both);
        ctx.draw_double_line(Vertical, HalfSelector::Last);
    }];
    box_char!['╨' -> |ctx: &Context| {
        ctx.draw_fg_line1(Horizontal, HalfSelector::Both);
        ctx.draw_double_line(Vertical, HalfSelector::First);
    }];
    box_char!['╪' -> |ctx: &Context| {
        ctx.draw_fg_line1(Vertical, HalfSelector::FirstDouble);
        ctx.draw_fg_line1(Vertical, HalfSelector::LastDouble);
        ctx.draw_double_line(Horizontal, HalfSelector::Both);
    }];
    box_char!['╫' -> |ctx: &Context| {
        ctx.draw_fg_line1(Horizontal, HalfSelector::FirstDouble);
        ctx.draw_fg_line1(Horizontal, HalfSelector::LastDouble);
        ctx.draw_double_line(Vertical, HalfSelector::Both);
    }];
    box_char!['╬' -> |ctx: &Context| {
        ctx.draw_double_line(Vertical, HalfSelector::FirstDouble);
        ctx.draw_double_line(Vertical, HalfSelector::LastDouble);
        ctx.draw_double_line(Horizontal, HalfSelector::FirstDouble);
        ctx.draw_double_line(Horizontal, HalfSelector::LastDouble);
    }];
    box_char!['╠' -> |ctx: &Context| {
        let stroke_width = ctx.get_stroke_width_pixels(Thickness::Level1);
        let o = Orientation::Vertical;
        ctx.draw_line(
            o,
            HalfSelector::Both,
            LineSelector::Left,
            stroke_width,
            ctx.color_fg,
            None,
        );
        ctx.draw_line(
            o,
            HalfSelector::FirstDouble,
            LineSelector::Right,
            stroke_width,
            ctx.color_fg,
            None,
        );
        ctx.draw_line(
            o,
            HalfSelector::LastDouble,
            LineSelector::Right,
            stroke_width,
            ctx.color_fg,
            None,
        );
        ctx.draw_double_line(Horizontal, HalfSelector::LastDouble);
    }];
    box_char!['╣' -> |ctx: &Context| {
        let stroke_width = ctx.get_stroke_width_pixels(Thickness::Level1);
        let o = Orientation::Vertical;
        ctx.draw_line(
            o,
            HalfSelector::Both,
            LineSelector::Right,
            stroke_width,
            ctx.color_fg,
            None,
        );
        ctx.draw_line(
            o,
            HalfSelector::FirstDouble,
            LineSelector::Left,
            stroke_width,
            ctx.color_fg,
            None,
        );
        ctx.draw_line(
            o,
            HalfSelector::LastDouble,
            LineSelector::Left,
            stroke_width,
            ctx.color_fg,
            None,
        );
        ctx.draw_double_line(Horizontal, HalfSelector::FirstDouble);
    }];
    box_char!['╦' -> |ctx: &Context| {
        let stroke_width = ctx.get_stroke_width_pixels(Thickness::Level1);
        let o = Orientation::Horizontal;
        ctx.draw_line(
            o,
            HalfSelector::Both,
            LineSelector::Left,
            stroke_width,
            ctx.color_fg,
            None,
        );
        ctx.draw_line(
            o,
            HalfSelector::FirstDouble,
            LineSelector::Right,
            stroke_width,
            ctx.color_fg,
            None,
        );
        ctx.draw_line(
            o,
            HalfSelector::LastDouble,
            LineSelector::Right,
            stroke_width,
            ctx.color_fg,
            None,
        );
        ctx.draw_double_line(Vertical, HalfSelector::LastDouble);
    }];
    box_char!['╩' -> |ctx: &Context| {
        let stroke_width = ctx.get_stroke_width_pixels(Thickness::Level1);
        let o = Orientation::Horizontal;
        ctx.draw_line(
            o,
            HalfSelector::Both,
            LineSelector::Right,
            stroke_width,
            ctx.color_fg,
            None,
        );
        ctx.draw_line(
            o,
            HalfSelector::FirstDouble,
            LineSelector::Left,
            stroke_width,
            ctx.color_fg,
            None,
        );
        ctx.draw_line(
            o,
            HalfSelector::LastDouble,
            LineSelector::Left,
            stroke_width,
            ctx.color_fg,
            None,
        );
        ctx.draw_double_line(Vertical, HalfSelector::FirstDouble);
    }];

    // eighth blocks
    box_char!['▀' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 0..=3);
    }];
    box_char!['▁' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 7..=7);
    }];
    box_char!['▂' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 6..=7);
    }];
    box_char!['▃' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 5..=7);
    }];
    box_char!['▄' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 4..=7);
    }];
    box_char!['▅' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 3..=7);
    }];
    box_char!['▆' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 2..=7);
    }];
    box_char!['▇' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 1..=7);
    }];
    box_char!['█' -> |ctx: &Context| {
        let mut paint = ctx.fg_paint();
        paint.set_style(PaintStyle::Fill);
        ctx.canvas.draw_paint(&paint);
    }];
    box_char!['▉' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 0..=6);
    }];
    box_char!['▊' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 0..=5);
    }];
    box_char!['▋' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 0..=4);
    }];
    box_char!['▌' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 0..=3);
    }];
    box_char!['▍' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 0..=2);
    }];
    box_char!['▎' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 0..=1);
    }];
    box_char!['▏' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 0..=0);
    }];
    box_char!['▐' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 4..=7);
    }];
    box_char!['▔' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 0..=0);
    }];
    box_char!['▕' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 7..=7);
    }];
    box_char!['🭼' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 0..=0);
        ctx.draw_eighth(Horizontal, 7..=7);
    }];
    box_char!['🭼' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 0..=0);
        ctx.draw_eighth(Horizontal, 0..=0);
    }];
    box_char!['🭾' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 7..=7);
        ctx.draw_eighth(Horizontal, 0..=0);
    }];
    box_char!['🭿' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 7..=7);
        ctx.draw_eighth(Horizontal, 7..=7);
    }];
    box_char!['🮀' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 0..=0);
        ctx.draw_eighth(Horizontal, 7..=7);
    }];
    box_char!['🮁' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 0..=0);
        ctx.draw_eighth(Horizontal, 2..=2);
        ctx.draw_eighth(Horizontal, 4..=4);
        ctx.draw_eighth(Horizontal, 7..=7);
    }];
    box_char!['🮂' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 0..=1);
    }];
    box_char!['🮃' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 0..=2);
    }];
    box_char!['🮄' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 0..=4);
    }];
    box_char!['🮅' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 0..=5);
    }];
    box_char!['🮆' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 0..=6);
    }];
    box_char!['🮇' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 6..=7);
    }];
    box_char!['🮈' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 5..=7);
    }];
    box_char!['🮉' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 3..=7);
    }];
    box_char!['🮊' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 2..=7);
    }];
    box_char!['🮋' -> |ctx: &Context| {
        ctx.draw_eighth(Vertical, 1..=7);
    }];
    box_char!['🮂' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 0..=1);
    }];
    box_char!['🮃' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 0..=2);
    }];
    box_char!['🮄' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 0..=4);
    }];
    box_char!['🮅' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 0..=5);
    }];
    box_char!['🮆' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 0..=6);
    }];
    box_char!['🮇' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 6..=7);
    }];
    box_char!['🮈' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 5..=7);
    }];
    box_char!['🮉' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 3..=7);
    }];
    box_char!['🮊' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 2..=7);
    }];
    box_char!['🮋' -> |ctx: &Context| {
        ctx.draw_eighth(Horizontal, 1..=7);
    }];

    // Shade
    box_char!['░' -> |ctx: &Context| {
        ctx.draw_shade(Orientation::Horizontal, HalfSelector::Both, Shade::Light, MirrorMode::Normal, ColorMode::Normal);
    }];
    box_char!['▒' -> |ctx: &Context| {
        ctx.draw_shade(Orientation::Horizontal, HalfSelector::Both, Shade::Medium, MirrorMode::Normal, ColorMode::Normal);
    }];
    box_char!['▓' -> |ctx: &Context| {
        ctx.draw_shade(Orientation::Horizontal, HalfSelector::Both, Shade::Dark, MirrorMode::Normal, ColorMode::Normal);
    }];
    box_char!['🮌' -> |ctx: &Context| {
        ctx.draw_shade(Orientation::Horizontal, HalfSelector::First, Shade::Medium, MirrorMode::Normal, ColorMode::Normal);
    }];
    box_char!['🮍' -> |ctx: &Context| {
        ctx.draw_shade(Orientation::Horizontal, HalfSelector::Last, Shade::Medium, MirrorMode::Normal, ColorMode::Normal);
    }];
    box_char!['🮎' -> |ctx: &Context| {
        ctx.draw_shade(Orientation::Vertical, HalfSelector::First, Shade::Medium, MirrorMode::Normal, ColorMode::Normal);
    }];
    box_char!['🮏' -> |ctx: &Context| {
        ctx.draw_shade(Orientation::Vertical, HalfSelector::Last, Shade::Medium, MirrorMode::Normal, ColorMode::Normal);
    }];
    box_char!['🮐' -> |ctx: &Context| {
        ctx.draw_shade(Orientation::Horizontal, HalfSelector::Both, Shade::Medium, MirrorMode::Normal, ColorMode::Inverted);
    }];
    box_char!['🮑' -> |ctx: &Context| {
        ctx.draw_shade(Orientation::Vertical, HalfSelector::Last, Shade::Medium, MirrorMode::Normal, ColorMode::Inverted);
    }];
    box_char!['🮒' -> |ctx: &Context| {
        ctx.draw_shade(Orientation::Vertical, HalfSelector::First, Shade::Medium, MirrorMode::Normal, ColorMode::Inverted);
    }];
    box_char!['🮓' -> |ctx: &Context| {
        ctx.draw_shade(Orientation::Horizontal, HalfSelector::Last, Shade::Medium, MirrorMode::Normal, ColorMode::Inverted);
    }];
    box_char!['🮔' -> |ctx: &Context| {
        ctx.draw_shade(Orientation::Horizontal, HalfSelector::First, Shade::Medium, MirrorMode::Normal, ColorMode::Inverted);
    }];
    box_char!['🮜' -> |ctx: &Context| {
        ctx.canvas.clip_path(&ctx.triangle_path(Corner::TopLeft), None, Some(false));
        ctx.draw_shade(Orientation::Horizontal, HalfSelector::Both, Shade::Medium,  MirrorMode::Normal, ColorMode::Normal);
    }];
    box_char!['🮝' -> |ctx: &Context| {
        ctx.canvas.clip_path(&ctx.triangle_path(Corner::TopRight), None, Some(false));
        ctx.draw_shade(Orientation::Horizontal, HalfSelector::Both, Shade::Medium,  MirrorMode::Normal, ColorMode::Normal);
    }];
    box_char!['🮞' -> |ctx: &Context| {
        ctx.canvas.clip_path(&ctx.triangle_path(Corner::BottomRight), None, Some(false));
        ctx.draw_shade(Orientation::Horizontal, HalfSelector::Both, Shade::Medium,  MirrorMode::Normal, ColorMode::Normal);
    }];
    // 🮜🮝
    // 🮞🮟
    // 🮝🮜
    // 🮟🮞
    box_char!['🮟' -> |ctx: &Context| {
        ctx.canvas.clip_path(&ctx.triangle_path(Corner::BottomLeft), None, Some(false));
        ctx.draw_shade(Orientation::Horizontal, HalfSelector::Both, Shade::Medium,  MirrorMode::Normal, ColorMode::Normal);
    }];
    // 🮙🮙🮙🮙🮙🮙🮙🮙🮙🮙
    // 🮙🮙🮙🮙🮙🮙🮙🮙🮙🮙
    box_char!['🮙' -> |ctx: &Context| {
        ctx.draw_shade(Orientation::Horizontal, HalfSelector::Both, Shade::Light,  MirrorMode::Normal, ColorMode::Normal);
    }];
    // 🮘🮘🮘🮘🮘🮘🮘🮘🮘🮘
    // 🮘🮘🮘🮘🮘🮘🮘🮘🮘🮘
    box_char!['🮘' -> |ctx: &Context| {
       ctx.draw_shade(Orientation::Horizontal, HalfSelector::Both, Shade::Light,  MirrorMode::Mirror, ColorMode::Normal);
    }];

    // ╭╮╰╯
    // ╭──────────╮
    // │          │
    // ╰──────────╯
    box_char!['╭' -> |ctx: &Context| {
        ctx.draw_rounded_corner(Corner::TopLeft);
    }];
    box_char!['╮' -> |ctx: &Context| {
        ctx.draw_rounded_corner(Corner::TopRight);
    }];
    box_char!['╰' -> |ctx: &Context| {
        ctx.draw_rounded_corner(Corner::BottomLeft);
    }];
    box_char!['╯' -> |ctx: &Context| {
        ctx.draw_rounded_corner(Corner::BottomRight);
    }];

    // T joints
    {
        use Thickness::{Level1 as t1, Level3 as t3};
        macro_rules! t_joint {
            ($($ch:literal -> $north:ident, $east:ident, $south:ident, $west:ident)+) => {
                $(m.insert(
                    $ch,
                    Box::new(move |ctx: &Context| {
                        ctx.draw_t_joint($north, $east, $south, $west);
                    }),
                ));+
            };
        }

        t_joint![
        // ┬ ┭ ┮ ┯ ┰ ┱ ┲ ┳
        '┬' -> None, t1, t1, t1
        '┭' -> None, t1, t1, t3
        '┮' -> None, t1, t3, t1
        '┯' -> None, t1, t3, t3
        '┰' -> None, t3, t1, t1
        '┱' -> None, t3, t1, t3
        '┲' -> None, t3, t3, t1
        '┳' -> None, t3, t3, t3

        // ┤ ┥ ┦ ┧ ┨ ┩ ┪ ┫
        '┤' -> t1, None, t1, t1
        '┥' -> t1, None, t1, t3
        '┦' -> t3, None, t1, t1
        '┧' -> t1, None, t3, t1
        '┨' -> t3, None, t3, t1
        '┩' -> t3, None, t1, t3
        '┪' -> t1, None, t3, t3
        '┫' -> t3, None, t3, t3

        // ┴ ┵ ┶ ┷ ┸ ┹ ┺ ┻
        '┴' -> t1, t1, None, t1
        '┵' -> t1, t1, None, t3
        '┶' -> t3, t1, None, t1
        '┷' -> t3, t1, None, t3
        '┸' -> t1, t3, None, t1
        '┹' -> t1, t3, None, t3
        '┺' -> t3, t3, None, t1
        '┻' -> t3, t3, None, t3

        // ├ ┝ ┞ ┟ ┠ ┡ ┢ ┣
        '├' -> t1, t1, t1, None
        '┝' -> t1, t3, t1, None
        '┞' -> t3, t1, t1, None
        '┟' -> t1, t1, t3, None
        '┠' -> t3, t1, t3, None
        '┡' -> t3, t3, t1, None
        '┢' -> t1, t3, t3, None
        '┣' -> t3, t3, t3, None
        ];
    }

    // Corners
    // ┌ ┍ ┎ ┏
    // ┐ ┑ ┒ ┓
    // └ ┕ ┖ ┗
    // ┘ ┙ ┚ ┛
    //
    // Test 1:
    // ┌─┐
    // │ │
    // └─┘
    //
    // Test 2:
    // ┍━┑
    // │ │
    // ┕━┙
    //
    // Test 3:
    // ┎─┒
    // ┃ ┃
    // ┖─┚
    //
    // Test 4:
    // ┏━┓
    // ┃ ┃
    // ┗━┛
    {
        use Corner::*;
        use Thickness::{Level1 as t1, Level3 as t3};
        macro_rules! corner {
            ($($ch:literal -> $corner:ident, $horiz:ident, $vert:ident)+) => {
                $(m.insert(
                    $ch,
                    Box::new(move |ctx: &Context| {
                        ctx.draw_corner($corner, $horiz, $vert);
                    }),
                ));+
            };
        }
        corner![
            '┌' -> TopLeft, t1, t1
            '┍' -> TopLeft, t3, t1
            '┎' -> TopLeft, t1, t3
            '┏' -> TopLeft, t3, t3

            '┐' -> TopRight, t1, t1
            '┑' -> TopRight, t3, t1
            '┒' -> TopRight, t1, t3
            '┓' -> TopRight, t3, t3

            '└' -> BottomLeft, t1, t1
            '┕' -> BottomLeft, t3, t1
            '┖' -> BottomLeft, t1, t3
            '┗' -> BottomLeft, t3, t3

            '┘' -> BottomRight, t1, t1
            '┙' -> BottomRight, t3, t1
            '┚' -> BottomRight, t1, t3
            '┛' -> BottomRight, t3, t3
        ];
    }

    m
});

pub fn is_box_char(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|ch| BOX_CHARS.contains_key(&ch))
}

pub struct Renderer {
    settings: BoxDrawingSettings,
    cell_size: Size2<Pixel<f32>>,
}

impl Renderer {
    pub fn new(cell_size: Size2<Pixel<f32>>, settings: BoxDrawingSettings) -> Self {
        Self {
            settings,
            cell_size,
        }
    }

    pub fn update_dimensions(&mut self, new_cell_size: Size2<Pixel<f32>>) {
        if self.cell_size != new_cell_size {
            self.cell_size = new_cell_size;
        }
    }

    pub fn update_settings(&mut self, settings: BoxDrawingSettings) {
        if self.settings != settings {
            self.settings = settings;
        }
    }

    pub fn draw_glyph(
        &self,
        box_char_text: &str,
        canvas: &Canvas,
        dst: PixelRect<f32>,
        color_fg: Color,
        window_pos: PixelPos<f32>,
    ) -> bool {
        match self
            .settings
            .mode
            .as_ref()
            .unwrap_or(&BoxDrawingMode::default())
        {
            BoxDrawingMode::FontGlyph => false,
            BoxDrawingMode::Native => {
                self.draw_box_glyph(box_char_text, canvas, dst, color_fg, window_pos)
            }
            BoxDrawingMode::SelectedNative => {
                let selected = self.settings.selected.as_deref().unwrap_or("");
                let is_selected = box_char_text
                    .chars()
                    .next()
                    .is_some_and(|first| selected.contains(first));
                if is_selected {
                    self.draw_box_glyph(box_char_text, canvas, dst, color_fg, window_pos)
                } else {
                    false
                }
            }
        }
    }

    fn draw_box_glyph(
        &self,
        box_char_text: &str,
        canvas: &Canvas,
        dst: PixelRect<f32>,
        color_fg: Color,
        window_pos: PixelPos<f32>,
    ) -> bool {
        let Some(ch) = box_char_text.chars().next() else {
            return false;
        };
        let Some(draw_fn) = BOX_CHARS.get(&ch) else {
            return false;
        };
        for (i, _) in box_char_text.chars().enumerate() {
            canvas.save();
            // Box chars need to be rendered with absolute x positions, so translate the x coordinates.
            // The line height is already a multiplier of pixels, so it does not need a fixup.
            let rect = Box2::from_rect(glamour::Rect::new(
                dst.min + Vector2::new(self.cell_size.width * i as f32, 0.0),
                self.cell_size,
            )) + PixelVec::new(window_pos.x, 0.0);
            canvas.clip_rect(to_skia_rect(&rect), None, Some(false));
            let ctx = Context::new(canvas, &self.settings, rect, color_fg);
            (draw_fn)(&ctx);
            canvas.restore();
        }
        true
    }
}
