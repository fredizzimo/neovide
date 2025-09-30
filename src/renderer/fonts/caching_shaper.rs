use std::{borrow::{Borrow, Cow}, iter::Iterator, num::NonZeroUsize, sync::Arc};

use itertools::{EitherOrBoth, Itertools};
use log::{debug, error, info, trace};
use lru::LruCache;
use skia_safe::{graphics::set_font_cache_limit, TextBlob, TextBlobBuilder, Vector};
use swash::{
    shape::ShapeContext,
    text::{
        cluster::{CharCluster, Parser, Status, Token},
        Script,
    },
    Metrics,
};

use crate::{
    error_msg,
    profiling::tracy_zone,
    renderer::fonts::{font_loader::*, font_options::*},
    units::PixelSize,
};

#[derive(new, Clone, Hash, PartialEq, Eq, Debug, Default)]
struct ShapeKey {
    pub text: String,
    pub cells: Vec<u8>,
    pub style: CoarseStyle,
}

struct ShapeKeyIterator<'a> {
    shape_key: &'a ShapeKey,
    current_cell: std::slice::Iter<'a, u8>,
    current_text_offset: usize,
    current_cell_nr: usize,
}

impl<'a> Iterator for ShapeKeyIterator<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(cell) = self.current_cell.next() {
            let cell_nr = self.current_cell_nr;
            // TODO: Deal with double width
            self.current_cell_nr += 1;
            let text_offset = self.current_text_offset; 
            self.current_text_offset += (*cell as usize) & 127;
            Some((cell_nr, &self.shape_key.text[text_offset..self.current_text_offset]))
        } else {
            None
        }
    }
}

const FONT_CACHE_SIZE: usize = 8 * 1024 * 1024;

pub struct CachingShaper {
    // The actual implementation is split into two, to satifsy the rust borrow checker
    shaper: Shaper,
    blob_cache: LruCache<ShapeKey, Vec<TextBlob>>,
}

pub struct Shaper {
    options: FontOptions,
    font_loader: FontLoader,
    shape_context: ShapeContext,
    scale_factor: f32,
    linespace: f32,
    font_info: Option<(Metrics, f32)>,
}

impl CachingShaper {
    pub fn new(scale_factor: f32) -> Self {
        Self {
            shaper: Shaper::new(scale_factor),
            blob_cache: LruCache::new(NonZeroUsize::new(10000).unwrap()),
        }
    }

    pub fn current_size(&self) -> f32 {
        self.shaper.current_size()
    }

    pub fn update_scale_factor(&mut self, scale_factor: f32) {
        self.shaper.update_scale_factor(scale_factor);
        self.blob_cache.clear();
    }

    pub fn update_font(&mut self, guifont_setting: &str) {
        debug!("Updating font: {guifont_setting}");

        let options = match FontOptions::parse(guifont_setting) {
            Ok(opt) => opt,
            Err(msg) => {
                error_msg!("Failed to parse guifont: {}", msg);
                return;
            }
        };

        self.update_font_options(options);
    }

    pub fn font_names(&self) -> Vec<String> {
        self.shaper.font_loader.font_names()
    }

    pub fn update_font_options(&mut self, options: FontOptions) {
        if self.shaper.update_font_options(options) {
            self.blob_cache.clear();
        }
    }

    pub fn update_linespace(&mut self, linespace: f32) {
        self.shaper.update_linespace(linespace);
    }

    pub fn font_base_dimensions(&mut self) -> PixelSize<f32> {
        self.shaper.font_base_dimensions()
    }

    pub fn underline_offset(&mut self) -> f32 {
        self.shaper.underline_offset()
    }

    pub fn baseline_offset(&mut self) -> f32 {
        self.shaper.baseline_offset()
    }

    pub fn stroke_size(&mut self) -> f32 {
        self.shaper.stroke_size()
    }

    pub fn cleanup_font_cache(&self) {
        self.shaper.cleanup_font_cache();
    }

    pub fn shape_cached<'a, I, F>(&mut self, cells: I, style: CoarseStyle, on_shaped: F)
    where
        I: Iterator<Item = &'a str> + Clone,
        F: Fn(Vector, &Vec<TextBlob>),
    {
        tracy_zone!("shape_cached");
        // let mut chunk_storage = Vec::new();
        let mut cached_key = ShapeKey::default();
        let font_width = self.shaper.font_base_dimensions().width;
        let mut pixel_offset = Vector::default();

        // let with_index = cells.clone().enumerate();
        // // Get the cell width and remove all empty cells from the shaper view
        // let with_widths = with_index
        //     .zip_longest(cells.skip(1))
        //     .filter_map(|e| match e {
        //         EitherOrBoth::Both((_, ""), _) => None,
        //         EitherOrBoth::Both((i, str), "") => Some((i, str, 2)),
        //         EitherOrBoth::Both((i, str), _) => Some((i, str, 1)),
        //         EitherOrBoth::Left((i, str)) => Some((i, str, 1)),
        //         EitherOrBoth::Right(_) => None,
        //     });
        let whitespace_chunked = cells.enumerate()
            .map(|c| (c.1.chars().next().unwrap().is_whitespace(), c))
            .chunk_by(|c| c.0);
        for (_, word) in whitespace_chunked
            .into_iter()
            .filter(|(is_whitespace, _)| !is_whitespace)
        {
            //chunk_storage.extend(chunk.map(|(_, c)| c));
            cached_key.text.clear();
            cached_key.cells.clear();
            let mut word_offset = 0;
            let mut first = true;
            for (index, (cell_offset, cell)) in word {
                if first {
                    word_offset = cell_offset;
                    first = false;
                }
                if cell.is_empty() {
                    // TODO: Deal with words starting with a double width char
                    if let Some(last) = cached_key.cells.last_mut() {
                        *last |= 128;
                    }
                } else {
                    cached_key.text.push_str(cell);
                    // TODO: deal with overflow
                    cached_key.cells.push(cell.len() as u8);
                }
            }


            // cached_key
            //     .text
            //     .extend(chunk_storage.iter().map(|(_, str, _)| *str));
            cached_key.style = style;
            pixel_offset.x = word_offset as f32 * font_width;
            on_shaped(
                pixel_offset,
                self.blob_cache.get_or_insert_ref(&cached_key, || {
                    trace!("Shaping text: {:?}", cached_key.text);

                    let iter = ShapeKeyIterator {
                        shape_key: &cached_key,
                        current_cell: cached_key.cells.iter(),
                        current_text_offset: 0,
                        current_cell_nr: 0,
                    };
                    self.shaper.shape(iter, style)
                }),
            );
            //
            // chunk_storage.clear();
        }
    }
}

impl Shaper {
    fn new(scale_factor: f32) -> Self {
        let options = FontOptions::default();
        let font_size = options.size * scale_factor;
        let mut shaper = Self {
            options,
            font_loader: FontLoader::new(font_size),
            shape_context: ShapeContext::new(),
            scale_factor,
            linespace: 0.0,
            font_info: None,
        };
        shaper.reset_font_loader();
        shaper
    }

    fn current_font_pair(&mut self) -> Arc<FontPair> {
        self.font_loader
            .get_or_load(&FontKey {
                font_desc: self.options.primary_font(),
                hinting: self.options.hinting.clone(),
                edging: self.options.edging.clone(),
            })
            .unwrap_or_else(|| {
                self.font_loader
                    .get_or_load(&FontKey::default())
                    .expect("Could not load default font")
            })
    }

    fn current_size(&self) -> f32 {
        let min_font_size = 1.0;
        (self.options.size * self.scale_factor).max(min_font_size)
    }

    fn update_scale_factor(&mut self, scale_factor: f32) {
        debug!("scale_factor changed: {scale_factor:.2}");
        self.scale_factor = scale_factor;
        self.reset_font_loader();
    }

    fn update_font_options(&mut self, options: FontOptions) -> bool {
        debug!("Updating font options: {options:?}");

        let keys = options
            .possible_fonts()
            .iter()
            .map(|desc| FontKey {
                font_desc: Some(desc.clone()),
                hinting: options.hinting.clone(),
                edging: options.edging.clone(),
            })
            .unique()
            .collect::<Vec<_>>();

        let failed_fonts = keys
            .iter()
            .filter(|key| self.font_loader.get_or_load(key).is_none())
            .collect_vec();

        if !failed_fonts.is_empty() {
            error_msg!(
                "Font can't be updated to: {:#?}\n\
                Following fonts couldn't be loaded: {}",
                options,
                failed_fonts.iter().join(",\n"),
            );
        }

        if failed_fonts.len() != keys.len() {
            debug!("Font updated to: {options:?}");
            self.options = options;
            true
        } else {
            false
        }
    }

    fn update_linespace(&mut self, linespace: f32) {
        debug!("Updating linespace: {linespace}");

        let font_height = self.font_base_dimensions().height;
        let impossible_linespace = font_height + linespace <= 0.0;

        if !impossible_linespace {
            debug!("Linespace updated to: {linespace}");
            self.linespace = linespace;
            self.reset_font_loader();
        } else {
            let reason = if impossible_linespace {
                "Linespace too negative, would make font invisible"
            } else {
                "Font not found"
            };
            error!("Linespace can't be updated to {linespace} due to: {reason}");
        }
    }

    fn reset_font_loader(&mut self) {
        tracy_zone!("reset_font_loader");
        self.font_info = None;
        let font_size = self.current_size();

        self.font_loader = FontLoader::new(font_size);
        let (_, font_width) = self.info();
        info!("Reset Font Loader: font_size: {font_size:.2}px, font_width: {font_width:.2}px");
    }

    fn info(&mut self) -> (Metrics, f32) {
        if let Some(info) = self.font_info {
            return info;
        }

        let font_pair = self.current_font_pair();
        let size = self.current_size();
        let mut shaper = self
            .shape_context
            .builder(font_pair.swash_font.as_ref())
            .size(size)
            .build();
        shaper.add_str("M");
        let metrics = shaper.metrics();
        let mut advance = metrics.average_width;
        shaper.shape_with(|cluster| {
            advance = cluster
                .glyphs
                .first()
                .map_or(metrics.average_width, |g| g.advance);
        });
        self.font_info = Some((metrics, advance));
        (metrics, advance)
    }

    fn metrics(&mut self) -> Metrics {
        tracy_zone!("font_metrics");
        self.info().0
    }

    fn font_base_dimensions(&mut self) -> PixelSize<f32> {
        let (metrics, glyph_advance) = self.info();

        let bare_font_height = metrics.ascent + metrics.descent + metrics.leading;
        // assuming that linespace is checked on receive for validity
        let font_height = (bare_font_height + self.linespace).ceil();
        let font_width = glyph_advance + self.options.width;

        (font_width, font_height).into()
    }

    fn underline_offset(&mut self) -> f32 {
        let metrics = self.metrics();
        if metrics.underline_offset != 0. {
            metrics.underline_offset
        } else {
            // If a font does not have an underline_offset, use the stroke_size as offset
            // A negative offset places the underline below the baseline
            -metrics.stroke_size
        }
    }

    fn stroke_size(&mut self) -> f32 {
        self.metrics().stroke_size
    }

    fn baseline_offset(&mut self) -> f32 {
        let metrics = self.metrics();
        // NOTE: leading is also called linegap and should be equally distributed on the top and
        // bottom, so it's centered like our linespace settings. That's how it works on the web,
        // but some desktop applications only use the top according to:
        // https://googlefonts.github.io/gf-guide/metrics.html#8-linegap-values-must-be-0
        metrics.ascent + (metrics.leading + self.linespace) / 2.0
    }

    fn build_clusters<'a, I>(
        &mut self,
        cells: I,
        style: CoarseStyle,
    ) -> Vec<(Vec<CharCluster>, Arc<FontPair>)> 
        where I: Iterator<Item = (usize, &'a str)>
    {
        tracy_zone!("build_clusters");
        let mut cluster = CharCluster::new();
        let mut results = Vec::new();
        // Neovim has already run it's own clustering algorithm and we need to follow the same rule.
        // So respect it by processing one cell at a time.
        // The third element is glyph width (1 or 2), but it's currently unused
        for (cell_nr, str) in cells {
            let mut parser = Parser::new(
                Script::Latin,
                str.char_indices().map(move |(offset, character)| Token {
                    ch: character,
                    offset: offset as u32,
                    len: character.len_utf8() as u8,
                    info: character.into(),
                    data: cell_nr as u32,
                }),
            );

            'cluster: while parser.next(&mut cluster) {
                // TODO: Don't redo this work for every cluster. Save it some how
                // Create font fallback list
                let mut font_fallback_keys = Vec::new();

                // Add parsed fonts from guifont or config file
                font_fallback_keys.extend(
                    self.options
                        .font_list(style)
                        .iter()
                        .map(|font_desc| FontKey {
                            font_desc: Some(font_desc.clone()),
                            hinting: self.options.hinting.clone(),
                            edging: self.options.edging.clone(),
                        })
                        .unique(),
                );

                // Add default font
                font_fallback_keys.push(FontKey {
                    font_desc: None,
                    hinting: self.options.hinting.clone(),
                    edging: self.options.edging.clone(),
                });

                // Use the cluster.map function to select a viable font from the fallback list and loaded fonts

                let mut best = None;
                // Search through the configured and default fonts for a match
                for fallback_key in font_fallback_keys.iter() {
                    if let Some(font_pair) = self.font_loader.get_or_load(fallback_key) {
                        let charmap = font_pair.swash_font.as_ref().charmap();
                        match cluster.map(|ch| charmap.map(ch)) {
                            Status::Complete => {
                                results.push((cluster.to_owned(), font_pair.clone()));
                                continue 'cluster;
                            }
                            Status::Keep => best = Some(font_pair),
                            Status::Discard => {}
                        }
                    }
                }

                // Configured font/default didn't work. Search through currently loaded ones
                for loaded_font in self.font_loader.loaded_fonts() {
                    let charmap = loaded_font.swash_font.as_ref().charmap();
                    match cluster.map(|ch| charmap.map(ch)) {
                        Status::Complete => {
                            results.push((cluster.to_owned(), loaded_font.clone()));
                            self.font_loader.refresh(loaded_font.as_ref());
                            continue 'cluster;
                        }
                        Status::Keep => best = Some(loaded_font),
                        Status::Discard => {}
                    }
                }

                if let Some(best) = best {
                    results.push((cluster.to_owned(), best.clone()));
                } else {
                    let fallback_character = cluster.chars()[0].ch;
                    if let Some(fallback_font) = self
                        .font_loader
                        .load_font_for_character(style, fallback_character)
                    {
                        results.push((cluster.to_owned(), fallback_font));
                    } else {
                        // Last Resort covers all of the unicode space so we will always have a fallback
                        results.push((
                            cluster.to_owned(),
                            self.font_loader.get_or_load_last_resort().unwrap(),
                        ));
                    }
                }
            }
        }

        // Now we have to group clusters by the font used so that the shaper can actually form
        // ligatures across clusters
        let mut grouped_results = Vec::new();
        let mut current_group = Vec::new();
        let mut current_font_option = None;
        for (cluster, font) in results {
            if let Some(current_font) = current_font_option.clone() {
                if current_font == font {
                    current_group.push(cluster);
                } else {
                    grouped_results.push((current_group, current_font));
                    current_group = vec![cluster];
                    current_font_option = Some(font);
                }
            } else {
                current_group = vec![cluster];
                current_font_option = Some(font);
            }
        }

        if !current_group.is_empty() {
            grouped_results.push((current_group, current_font_option.unwrap()));
        }

        grouped_results
    }

    fn cleanup_font_cache(&self) {
        tracy_zone!("purge_font_cache");
        set_font_cache_limit(FONT_CACHE_SIZE / 2);
        set_font_cache_limit(FONT_CACHE_SIZE);
    }

    fn shape<'a, I>(&mut self, cells: I, style: CoarseStyle) -> Vec<TextBlob> 
        where I: Iterator<Item = (usize, &'a str)>
    {
        let current_size = self.current_size();
        let glyph_width = self.font_base_dimensions().width;

        let mut resulting_blobs = Vec::new();

        for (cluster_group, font_pair) in self.build_clusters(cells, style) {
            tracy_zone!("shape cluster group");
            let features = self.get_font_features(
                font_pair
                    .as_ref()
                    .key
                    .font_desc
                    .as_ref()
                    .map(|desc| desc.family.as_str()),
            );

            let mut shaper = self
                .shape_context
                .builder(font_pair.swash_font.as_ref())
                .features(features.iter().map(|(name, value)| (name.as_ref(), *value)))
                .size(current_size)
                .build();

            let charmap = font_pair.swash_font.as_ref().charmap();
            for mut cluster in cluster_group {
                cluster.map(|ch| charmap.map(ch));
                shaper.add_cluster(&cluster);
            }

            let mut glyph_data = Vec::new();

            shaper.shape_with(|glyph_cluster| {
                //Align to the grid at the start of each cluster
                let mut x_offset = glyph_width * glyph_cluster.data as f32;

                for glyph in glyph_cluster.glyphs {
                    let position = (x_offset + glyph.x, -glyph.y);
                    glyph_data.push((glyph.id, position));
                    x_offset += glyph.advance;
                }
            });

            if glyph_data.is_empty() {
                continue;
            }

            let mut blob_builder = TextBlobBuilder::new();
            let (glyphs, positions) =
                blob_builder.alloc_run_pos(&font_pair.skia_font, glyph_data.len(), None);
            for (i, (glyph_id, glyph_position)) in glyph_data.iter().enumerate() {
                glyphs[i] = *glyph_id;
                positions[i] = (*glyph_position).into();
            }

            let blob = blob_builder.make();
            resulting_blobs.push(blob.expect("Could not create textblob"));
        }

        resulting_blobs
    }

    fn get_font_features(&self, name: Option<&str>) -> Vec<(String, u16)> {
        if let Some(name) = name {
            self.options
                .features
                .get(name)
                .map(|features| {
                    features
                        .iter()
                        .map(|feature| (feature.0.clone(), feature.1))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            vec![]
        }
    }
}
