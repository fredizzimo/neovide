use std::{
    rc::Rc,
    cell::RefCell,
};

use crate::units::{to_skia_rect, GridPos, GridScale, GridSize, PixelRect, PixelSize};
use base64::{
    alphabet,
    engine::{
        general_purpose::{GeneralPurpose, GeneralPurposeConfig},
        DecodePaddingMode,
    },
    Engine,
};
use bytemuck::cast_ref;
use glamour::{Matrix3, Matrix4};
use itertools::Itertools;
use serde::Deserialize;
use skia_safe::{
    canvas::SrcRectConstraint, matrix::Member, BlendMode, Canvas, Data, FilterMode, Image, Matrix,
    MipmapMode, Paint, RSXform, Rect, SamplingOptions, M44,
};
use std::{collections::HashMap, ops::Range};

use super::{nvim_image as image, rendered_window::ImageFragment, LineFragment};
use crate::units::{GridRect, PixelVec};

/// Don't add padding when encoding, and allow input with or without padding when decoding.
pub const NO_PAD_INDIFFERENT: GeneralPurposeConfig = GeneralPurposeConfig::new()
    .with_encode_padding(false)
    .with_decode_padding_mode(DecodePaddingMode::Indifferent);

/// A [`GeneralPurpose`] engine using the [`alphabet::STANDARD`] base64 alphabet and
/// [`NO_PAD_INDIFFERENT`] config.
pub const STANDARD_NO_PAD_INDIFFERENT: GeneralPurpose =
    GeneralPurpose::new(&alphabet::STANDARD, NO_PAD_INDIFFERENT);

struct DisplayedImage {
    width: u32,
    height: u32,
}

// struct VisibleImageFragment {
//     xform: Vec<RSXform>,
//     tex: Vec<Rect>,
//     inv_matrix: Matrix3<f32>,
//     skia_matrix: M44,
//     image_scale: GridScale,
// }

struct LoadedImage {
    skia_image: Image,
    xform: RefCell<Vec<RSXform>>,
    tex: RefCell<Vec<Rect>>,
}

struct VisibleImage {
    loaded_image: Rc<LoadedImage>, 
    placement: image::ImgShow,
}

pub struct ImageRenderer {
    loaded_images: HashMap<u32, Rc<LoadedImage>>,
    visible_images: HashMap<u32, VisibleImage>,
    in_progress_image: Option<image::UploadImage>,
    //displayed_images: HashMap<(u32, u32), DisplayedImage>,
}

// #[derive(Clone)]
// pub struct ImageFragment {
//     pub dst_col: u32,
//     pub src_row: u32,
//     pub src_range: Range<u32>,
//     pub image_id: u32,
//     pub placement_id: u32,
// }

pub struct FragmentRenderer<'a> {
    visible_images: Vec<Rc<LoadedImage>>,
    renderer: &'a ImageRenderer,
}

#[derive(Clone, Debug, PartialEq, Default, Deserialize)]
// Units are pixels
pub struct Crop {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl From<&Crop> for PixelRect<f32> {
    fn from(val: &Crop) -> Self {
        PixelRect::from_origin_and_size(
            (val.x as f32, val.y as f32).into(),
            (val.width as f32, val.height as f32).into(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Default, Deserialize)]
// Units are cells
pub struct Pos {
    row: i32,
    col: i32,
}

impl From<&Pos> for GridPos<i32> {
    fn from(val: &Pos) -> Self {
        GridPos::new(val.col, val.row)
    }
}

#[derive(Clone, Debug, PartialEq, Default, Deserialize)]
// Units are cells
pub struct Size {
    width: i32,
    height: i32,
}

impl From<&Size> for GridSize<i32> {
    fn from(val: &Size) -> Self {
        GridSize::new(val.width, val.height)
    }
}

#[derive(Clone, Debug, PartialEq, Default, Deserialize)]
pub struct ImageRenderOpts {
    pub crop: Option<Crop>,
    pub pos: Option<Pos>,
    pub size: Option<Size>,
}

impl ImageRenderer {
    pub fn new() -> Self {
        Self {
            loaded_images: HashMap::new(),
            visible_images: HashMap::new(),
            in_progress_image: None,
        }
    }

    pub fn add_image(&mut self, opts: image::ImgAdd) {
        let image_data = {
            Data::new_copy(&opts.data)
        };

        // Assume png for now
        let skia_image = Image::from_encoded(image_data).unwrap();
        self.loaded_images.insert(opts.id, Rc::new(LoadedImage {
            skia_image,
            xform: RefCell::new(Vec::new()),
            tex: RefCell::new(Vec::new()),
        }));
        //self.displayed_images.insert((opts.id, 1), DisplayedImage { width: opts.width, height: opts.height });
    }

    pub fn show_image(&mut self, placement: image::ImgShow) {
        if let Some(loaded_image) = self.loaded_images.get(&placement.img_id) {
            self.visible_images.insert(placement.id, VisibleImage {
                loaded_image: Rc::clone(loaded_image),
                placement,
            });
        }
            // match opts.opts.relative {
            //     None => self
            //         .visible_images
            //         .push(((opts.image_id, opts.placement_id), opts.opts)),
            //     Some(image::Relative::Placement) => {
            //         self.displayed_images
            //             .insert((opts.image_id, opts.placement_id), opts.opts);
            //     }
            //     _ => {}
            // }
    }

    pub fn hide_images(&mut self, images: Vec<u32>) {
        // self.visible_images
        //     .retain(|((_, placement_id), _)| !images.iter().contains(placement_id));
    }

    pub fn draw_frame(&self, canvas: &Canvas, grid_scale: GridScale) {
        // for ((id, _), opts) in &self.visible_images {
        //     if let Some(image) = self.loaded_images.get(id) {
        //         // The position is 1-indexed
        //         let pos = ((GridPos::new(opts.col.unwrap_or(1), opts.row.unwrap_or(1))
        //             - GridPos::new(1, 1))
        //             * grid_scale)
        //             .into();
        //
        //         let image_dimensions = image.dimensions();
        //         let image_dimensions = PixelSize::new(
        //             image_dimensions.width as f32,
        //             image_dimensions.height as f32,
        //         );
        //         let image_aspect = image_dimensions.width / image_dimensions.height;
        //
        //         let size =
        //             GridSize::new(opts.width.unwrap_or(0), opts.height.unwrap_or(0)) * grid_scale;
        //         let size = match (size.width, size.height) {
        //             (0.0, 0.0) => PixelSize::default(),
        //             (x, 0.0) => PixelSize::new(x, x / image_aspect),
        //             (0.0, y) => PixelSize::new(y * image_aspect, y),
        //             (x, y) => {
        //                 let grid_aspect = x / y;
        //                 if image_aspect >= grid_aspect {
        //                     PixelSize::new(x, x / image_aspect)
        //                 } else {
        //                     PixelSize::new(y * image_aspect, y)
        //                 }
        //             }
        //         };
        //         let dst = PixelRect::from_origin_and_size(pos, size);
        //         let crop = None;
        //         let src = crop.as_ref().map(|crop| (crop, SrcRectConstraint::Strict));
        //         let paint = Paint::default();
        //         canvas.draw_image_rect(image, src, to_skia_rect(&dst), &paint);
        //     }
        // }
    }

    pub fn begin_draw_image_fragments(&self) -> FragmentRenderer {
        FragmentRenderer::new(self)
    }
}

// TODO: move directly into the image renderer
impl<'a> FragmentRenderer<'a> {
    // TODO: Image renderer should be mutable to indicate that we temporarily modify it
    pub fn new(renderer: &'a ImageRenderer) -> Self {
        Self {
            visible_images: Vec::new(),
            renderer,
        }
    }

    pub fn draw(&mut self, fragments: &Vec<LineFragment>, matrix: &Matrix, scale: &GridScale) {
        for fragment in fragments.iter().filter(|fragment| fragment.image_fragment.is_some()) {
            let image_fragment = fragment.image_fragment.as_ref().unwrap();
            let visible_image = self.renderer.visible_images.get(&image_fragment.id);
            if visible_image.is_none() {
                continue;
            }
            let visible_image = visible_image.unwrap();
            let image = &visible_image.loaded_image;
            let skia_image = &image.skia_image;
            // TODO these can be part of the placement, and re-calculated when the scale changes
            let columns = visible_image.placement.width;
            let rows = visible_image.placement.height;
            let x_scale = (columns as f32 * scale.width()) / skia_image.width() as f32;
            let y_scale = (rows as f32 * scale.height()) / skia_image.height() as f32;
            let matrix = Matrix3::from_scale((x_scale, y_scale).into());
            let inv_matrix = matrix.inverse();
            let skia_matrix = Matrix4::<f32>::from_mat3(matrix);
            let skia_matrix = M44::col_major(cast_ref(skia_matrix.as_ref()));
            let image_scale = GridScale::new(PixelSize::new(
                skia_image.width() as f32 / columns as f32,
                skia_image.height() as f32 / rows as f32,
            ));

            //         VisibleImage {
            //             image,
            //             xform: Vec::new(),
            //             tex: Vec::new(),
            //             skia_matrix,
            //             inv_matrix,
            //             image_scale,
            //         }
            //     });

            // let dest_pos = GridPos::new(fragment.window_left, 0) * *scale
            //     + PixelVec::new(matrix[Member::TransX], matrix[Member::TransY]);
            // let dest_pos = inv_matrix.transform_point2(dest_pos.to_untyped());
            let mut xform = image.xform.borrow_mut();
            if xform.is_empty() {
                self.visible_images.push(Rc::clone(&image));
            }
            xform.push(RSXform::new(1.0, 0.0, (0.0, 0.0)));

            let cell = image_fragment.index; 
            let column = cell % visible_image.placement.width;
            let row = cell / visible_image.placement.width;

            let src_min = GridPos::new(column, row);
            let src_max = GridPos::new(column + fragment.width as u32, row + 1);
            let src_rect = GridRect::new(src_min, src_max) * image_scale;
            let mut tex = image.tex.borrow_mut();
            tex.push(to_skia_rect(&src_rect));
        }
    }

    pub fn flush(self, canvas: &Canvas) {
        for image in &self.visible_images {
            let paint = Paint::default();
            // Kitty uses Linear filtering, so use that here as well
            // It does not look very good when upscaling some images like logos though
            let sampling_options = SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear);
            canvas.save();
            //canvas.set_matrix(&image.skia_matrix);
            let mut xform = image.xform.borrow_mut();
            let mut tex = image.tex.borrow_mut();
            canvas.draw_atlas(
                &image.skia_image,
                &xform,
                &tex,
                None,
                BlendMode::Src,
                sampling_options,
                None,
                &paint,
            );
            xform.clear();
            tex.clear();
            canvas.restore();
        }
    }
}

