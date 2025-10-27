use std::{
    iter::{zip, Iterator},
    num::NonZeroUsize,
    ops::{Bound, Range},
    rc::Rc,
};

use harfrust::{self, GlyphBuffer, GlyphInfo, UnicodeBuffer};
use icu_properties::{
    props::{Emoji, EmojiModifier, EmojiPresentation},
    CodePointSetData, CodePointSetDataBorrowed,
};
use itertools::{izip, Itertools};
use log::{debug, error, info, trace};
use lru::LruCache;
use skia_safe::{graphics::set_font_cache_limit, Point, TextBlob, TextBlobBuilder};
use skrifa::metrics::Metrics;

use crate::{
    error_msg,
    profiling::tracy_zone,
    renderer::{
        fonts::{font_loader::*, font_options::*},
        RenderedWord,
    },
    units::PixelSize,
};

#[derive(new, Clone, Hash, PartialEq, Eq, Debug)]
struct ShapeKey {
    pub text: String,
    pub style: CoarseStyle,
}

enum GlyphPosition {
    Relative(harfrust::GlyphPosition),
    Absolute(Point),
}

struct Glyph {
    glyph_id: u32,
    position: GlyphPosition,
}

struct GraphemeCluster<'a> {
    cell_nr: usize,
    text: &'a str,
    complete: bool,
    glyphs: Range<usize>,
}

struct GlyphCluster<'a> {
    glyph_infos: &'a [GlyphInfo],
    glyph_positions: &'a [harfrust::GlyphPosition],
    graphemes: (Bound<usize>, Bound<usize>),
}

struct GlyphClusterIterator<'a> {
    glyph_infos: &'a [GlyphInfo],
    glyph_positions: &'a [harfrust::GlyphPosition],
}

impl<'a> GlyphClusterIterator<'a> {
    fn new(buffer: &'a GlyphBuffer) -> Self {
        Self {
            glyph_infos: buffer.glyph_infos(),
            glyph_positions: buffer.glyph_positions(),
        }
    }
}

struct Font {
    font_pair: Rc<FontPair>,
    glyph_range: Range<usize>,
    scaled_size: f32,
}

impl<'a> Iterator for GlyphClusterIterator<'a> {
    type Item = GlyphCluster<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.glyph_infos.is_empty() {
            None
        } else {
            let mut len = 1;
            let mut iter = self.glyph_infos.windows(2);
            while let Some([l, r]) = iter.next() {
                if l.cluster >> 16 == r.cluster >> 16 {
                    len += 1
                } else {
                    break;
                }
            }
            let (glyph_infos, tail) = self.glyph_infos.split_at(len);
            self.glyph_infos = tail;
            let (glyph_positions, tail) = self.glyph_positions.split_at(len);
            self.glyph_positions = tail;

            let first_grapheme = Bound::Included((glyph_infos[0].cluster >> 16) as usize);
            let next_grapheme = if let Some(next_info) = self.glyph_infos.first() {
                Bound::Excluded((next_info.cluster >> 16) as usize)
            } else {
                Bound::Unbounded
            };
            let graphemes = (first_grapheme, next_grapheme);

            Some(GlyphCluster {
                glyph_infos,
                glyph_positions,
                graphemes,
            })
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.glyph_infos.is_empty() {
            (0, Some(0))
        } else {
            (1, Some(self.glyph_infos.len()))
        }
    }
}

const FONT_CACHE_SIZE: usize = 8 * 1024 * 1024;

struct EmojiDetector {
    emoji: CodePointSetDataBorrowed<'static>,
    emoji_presentation: CodePointSetDataBorrowed<'static>,
    emoji_modifier: CodePointSetDataBorrowed<'static>,
}

pub struct CachingShaper {
    options: FontOptions,
    font_loader: FontLoader,
    blob_cache: LruCache<ShapeKey, Option<TextBlob>>,
    scale_factor: f32,
    linespace: f32,
    font_info: Option<(Metrics, f32)>,
    emoji_detector: EmojiDetector,
}

impl CachingShaper {
    pub fn new(scale_factor: f32) -> CachingShaper {
        let options = FontOptions::default();
        let mut shaper = CachingShaper {
            font_loader: FontLoader::new(options.clone()),
            options,
            blob_cache: LruCache::new(NonZeroUsize::new(10000).unwrap()),
            scale_factor,
            linespace: 0.0,
            font_info: None,
            emoji_detector: EmojiDetector {
                emoji: CodePointSetData::new::<Emoji>(),
                emoji_presentation: CodePointSetData::new::<EmojiPresentation>(),
                emoji_modifier: CodePointSetData::new::<EmojiModifier>(),
            },
        };
        shaper.reset_font_loader();
        shaper
    }

    fn current_font_pair(&mut self) -> Rc<FontPair> {
        self.options
            .primary_font()
            .and_then(|font| {
                self.font_loader.get_or_load(&FontKey {
                    font_desc: font,
                    hinting: self.options.hinting.clone(),
                    edging: self.options.edging.clone(),
                })
            })
            // NOTE: Update font options already tries to load the primary font, and won't pass it here if it fails.
            // So the only way this can happen, is if the default system monospace font can't be loaded
            .expect("Could not load the system monospace font")
    }

    pub fn current_size(&self) -> f32 {
        let min_font_size = 1.0;
        (self.options.size * self.scale_factor).max(min_font_size)
    }

    pub fn update_scale_factor(&mut self, scale_factor: f32) {
        debug!("scale_factor changed: {scale_factor:.2}");
        self.scale_factor = scale_factor;
        self.reset_font_loader();
    }

    pub fn update_font(&mut self, guifont_setting: &str) {
        if guifont_setting.is_empty() {
            return;
        }
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

    pub fn update_font_options(&mut self, options: FontOptions) {
        debug!("Updating font options: {options:?}");

        if let Some(font) = options.primary_font() {
            let key = FontKey {
                font_desc: font,
                hinting: options.hinting.clone(),
                edging: options.edging.clone(),
            };
            if self.font_loader.get_or_load(&key).is_none() {
                error_msg!(
                    "Failed to load primary font ({}).\n The font settings have not been updated.",
                    key.font_desc.family
                );
                return;
            }
        } else {
            error_msg!("No primary font specified. The font settings have not been updated.");
            return;
        }

        let keys = options
            .possible_fonts()
            .iter()
            .map(|desc| FontKey {
                font_desc: desc.clone(),
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
            self.reset_font_loader();
        }
    }

    pub fn update_linespace(&mut self, linespace: f32) {
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

        self.font_loader = FontLoader::new(self.options.clone());
        let (_, font_width) = self.info();
        info!("Reset Font Loader: font_size: {font_size:.2}px, font_width: {font_width:.2}px");

        self.blob_cache.clear();
    }

    pub fn font_names(&self) -> Vec<String> {
        self.font_loader.font_names()
    }

    fn info(&mut self) -> (Metrics, f32) {
        if let Some(info) = self.font_info {
            return info;
        }

        let size = self.current_size();
        let pair = &self.current_font_pair();
        let font_info = pair.font_info.scale(size);
        let advance = font_info.advance;
        self.font_info = Some((font_info.metrics, advance));
        self.font_info.unwrap()
    }

    fn metrics(&mut self) -> Metrics {
        tracy_zone!("font_metrics");
        self.info().0
    }

    pub fn font_base_dimensions(&mut self) -> PixelSize<f32> {
        let (metrics, glyph_advance) = self.info();

        let bare_font_height = metrics.ascent - metrics.descent + metrics.leading;
        // assuming that linespace is checked on receive for validity
        let font_height = (bare_font_height + self.linespace).ceil();
        let font_width = glyph_advance + self.options.width;

        (font_width, font_height).into()
    }

    pub fn underline_offset(&mut self) -> f32 {
        let metrics = self.metrics();
        if self.options.underline_offset.is_some() {
            -self.options.underline_offset.unwrap()
        } else if let Some(underline) = metrics.underline {
            underline.offset
        } else {
            1.0
        }
    }

    pub fn stroke_size(&mut self) -> f32 {
        if let Some(underline) = self.metrics().underline {
            underline.thickness
        } else {
            1.0
        }
    }

    pub fn baseline_offset(&mut self) -> f32 {
        let metrics = self.metrics();
        // NOTE: leading is also called linegap and should be equally distributed on the top and
        // bottom, so it's centered like our linespace settings. That's how it works on the web,
        // but some desktop applications only use the top according to:
        // https://googlefonts.github.io/gf-guide/metrics.html#8-linegap-values-must-be-0
        metrics.ascent + (metrics.leading + self.linespace) / 2.0
    }

    pub fn cleanup_font_cache(&self) {
        tracy_zone!("purge_font_cache");
        set_font_cache_limit(FONT_CACHE_SIZE / 2);
        set_font_cache_limit(FONT_CACHE_SIZE);
    }

    pub fn shape(&mut self, word: RenderedWord<'_>, style: CoarseStyle) -> Option<TextBlob> {
        // Add parsed fonts from guifont or config file

        let glyph_width = self.font_base_dimensions().width;
        let mut font_shaper = FontShaper::new();

        let font_list = self.options.font_list(&style);
        let font_fallback_keys = font_list.map(|font_desc| FontKey {
            font_desc: font_desc.clone(),
            hinting: self.options.hinting.clone(),
            edging: self.options.edging.clone(),
        });

        let mut clusters = word
            .clusters()
            .map(|(cell_nr, text)| GraphemeCluster {
                cell_nr,
                text,
                complete: false,
                glyphs: 0..0,
            })
            .collect_vec();
        let mut glyphs = Vec::new();
        let mut buffer = UnicodeBuffer::new();
        let font_loader = &mut self.font_loader;
        let emoji_detector = &self.emoji_detector;

        // The color emoji has the highest priority
        for chunk in clusters.chunk_by_mut(|a, b| {
            emoji_detector.is_color_emoji(a.text.chars())
                != emoji_detector.is_color_emoji(b.text.chars())
        }) {
            if emoji_detector.is_color_emoji(chunk[0].text.chars()) {
                let mut chars = chunk[0].text.chars();
                let first_char = chars.next().unwrap_or_default();
                if let Some(emoji_font) = font_loader.get_or_load_emoji(first_char) {
                    buffer = font_shaper.shape(&emoji_font, chunk, &mut glyphs, buffer);
                }
            }
        }

        for key in font_fallback_keys {
            if let Some(font_pair) = self.font_loader.get_or_load(&key) {
                buffer = font_shaper.shape(&font_pair, &mut clusters, &mut glyphs, buffer);
            }
        }

        let mut remaining_clusters = &mut clusters[..];
        while let Some((index, first_invalid)) =
            remaining_clusters.iter().find_position(|c| !c.complete)
        {
            let fallback_character = first_invalid.text.chars().next().unwrap_or('a');
            remaining_clusters = &mut remaining_clusters[index..];
            if let Some(fallback_font) =
                self.font_loader
                    .load_font_for_character(style, fallback_character, &[])
            {
                buffer = font_shaper.shape(&fallback_font, remaining_clusters, &mut glyphs, buffer);
            }

            // We need to progress at least one cluster forward for each system fallback to avoid an invalid loop
            // Any failures will be shaped using the last resort font
            remaining_clusters = &mut remaining_clusters[1..];
        }

        if let Some(last_resort) = self.font_loader.get_or_load_last_resort() {
            let _ = font_shaper.shape(&last_resort, &mut clusters, &mut glyphs, buffer);
        }

        let current_size = self.current_size();
        font_shaper.layout(
            current_size,
            glyph_width,
            self.info().1,
            &mut glyphs,
            &clusters,
        );

        font_shaper.build_blob(&glyphs)
    }

    pub fn shape_cached(
        &mut self,
        word: RenderedWord<'_>,
        style: CoarseStyle,
    ) -> &Option<TextBlob> {
        tracy_zone!("shape_cached");
        let text = word.text();
        let key = ShapeKey::new(text.to_string(), style);

        if !self.blob_cache.contains(&key) {
            trace!("Shaping text: {text:?}");
            let blob = self.shape(word, style);
            self.blob_cache.put(key.clone(), blob);
        }

        self.blob_cache.get(&key).unwrap()
    }
}

struct FontShaper {
    used_fonts: Vec<Font>,
    blob_builder: TextBlobBuilder,
}

impl FontShaper {
    fn new() -> Self {
        Self {
            used_fonts: Vec::new(),
            blob_builder: TextBlobBuilder::new(),
        }
    }

    fn shape(
        &mut self,
        font_pair: &Rc<FontPair>,
        clusters: &mut [GraphemeCluster],
        glyphs: &mut Vec<Glyph>,
        buffer: UnicodeBuffer,
    ) -> UnicodeBuffer {
        // Don't shape the same font twice (it will never succed anyway)
        if self
            .used_fonts
            .iter()
            .any(|f| f.font_pair.key == font_pair.key)
        {
            return buffer;
        }
        let start = glyphs.len();
        let ret = shape_font(font_pair, clusters, glyphs, buffer);
        self.used_fonts.push(Font {
            font_pair: font_pair.clone(),
            glyph_range: start..glyphs.len(),
            scaled_size: 1.0,
        });
        ret
    }

    fn layout(
        &mut self,
        current_size: f32,
        glyph_width: f32,
        advance: f32,
        glyphs: &mut [Glyph],
        clusters: &[GraphemeCluster],
    ) {
        let mut font_iter = self.used_fonts.iter_mut();
        let mut font = font_iter.next();
        let mut glyph_nr = 0;
        for cluster in clusters {
            while glyph_nr >= font.as_ref().unwrap().glyph_range.end {
                font = font_iter.next();
                if font.is_none() {
                    return;
                }
            }
            //Align to the grid at the start of each cluster
            let mut current_pos = glyph_width * cluster.cell_nr as f32;

            let current_font = font.as_mut().unwrap();

            let font_info = &current_font.font_pair.font_info;

            let scale = advance / (font_info.advance * current_size);
            log::info!("Scale {scale}");
            let scaled_size = current_size * scale;
            current_font.scaled_size = scaled_size;

            let glyphs = &mut glyphs[cluster.glyphs.clone()];
            for glyph in glyphs.iter_mut() {
                glyph.position = match glyph.position {
                    GlyphPosition::Relative(harfrust::GlyphPosition {
                        x_offset,
                        y_offset,
                        x_advance,
                        ..
                    }) => {
                        let base_pos = current_pos;
                        current_pos += font_info.scale_offset(x_advance, scaled_size);
                        GlyphPosition::Absolute(
                            (
                                base_pos + font_info.scale_offset(x_offset, scaled_size),
                                font_info.scale_offset(y_offset, scaled_size),
                            )
                                .into(),
                        )
                    }
                    _ => {
                        panic!("We should only have relative positions here");
                    }
                }
            }
            glyph_nr += glyphs.len();
        }
    }

    fn build_blob(&mut self, glyphs: &[Glyph]) -> Option<TextBlob> {
        for Font {
            font_pair,
            glyph_range,
            scaled_size,
        } in &self.used_fonts
        {
            let (dest_glyphs, dest_positions) = self.blob_builder.alloc_run_pos(
                &font_pair.skia_font.with_size(*scaled_size).unwrap(),
                glyph_range.len(),
                None,
            );
            let font_glyphs = &glyphs[glyph_range.clone()];
            for (dest_glyph, dest_position, source) in
                izip!(dest_glyphs, dest_positions, font_glyphs)
            {
                *dest_glyph = source.glyph_id as u16;
                match source.position {
                    GlyphPosition::Absolute(pos) => *dest_position = pos,
                    GlyphPosition::Relative(..) => *dest_position = Point::default(),
                }
            }
        }
        self.blob_builder.make()
    }
}

fn shape_font(
    font_pair: &Rc<FontPair>,
    clusters: &mut [GraphemeCluster],
    glyphs: &mut Vec<Glyph>,
    mut buffer: UnicodeBuffer,
) -> UnicodeBuffer {
    let mut remaining_clusters = clusters;
    loop {
        let first_invalid = remaining_clusters.iter().find_position(|c| !c.complete);
        if first_invalid.is_none() {
            return buffer;
        }
        let first_invalid = first_invalid.unwrap().0;
        remaining_clusters = &mut remaining_clusters[first_invalid..];

        let first_valid = remaining_clusters
            .iter()
            .find_position(|c| c.complete)
            .map(|(pos, _)| pos)
            .unwrap_or(remaining_clusters.len());
        let (incomplete_range, rest) = remaining_clusters.split_at_mut(first_valid);
        remaining_clusters = rest;

        for (i, cluster) in incomplete_range.iter().enumerate() {
            for (j, c) in cluster.text.chars().enumerate() {
                buffer.add(c, (((i << 16) & 0xFFFF0000) | j) as u32);
            }
        }

        buffer.guess_segment_properties();
        // HACK: Force left to right direction for now
        // Bidi or right to left will require re-mapping the cell positions
        // So, it's a lot more complex
        buffer.set_direction(harfrust::Direction::LeftToRight);
        let shaper = font_pair.shaper();
        let result = shaper.shape(buffer, &font_pair.features);

        for glyph_cluster in GlyphClusterIterator::new(&result) {
            let complete = !glyph_cluster
                .glyph_infos
                .iter()
                .any(|info| info.glyph_id == 0);
            if !complete {
                continue;
            }
            let graphemes = &mut incomplete_range[glyph_cluster.graphemes];
            for grapheme in graphemes.iter_mut() {
                grapheme.complete = true;
            }
            let start = glyphs.len();
            glyphs.extend(
                zip(glyph_cluster.glyph_infos, glyph_cluster.glyph_positions).map(
                    |(info, position)| Glyph {
                        glyph_id: info.glyph_id,
                        position: GlyphPosition::Relative(*position),
                    },
                ),
            );
            graphemes[0].glyphs = start..glyphs.len();
        }
        buffer = result.clear();
    }
}

impl EmojiDetector {
    fn is_color_emoji(&self, mut cluster_chars: impl Iterator<Item = char>) -> bool {
        const VARIANT_SELECTOR_PREFER_TEXT: char = '\u{FE0E}';
        const VARIANT_SELECTOR_PREFER_EMOJI: char = '\u{FE0F}';
        // NOTE: cluster.info().is_emoji() could be used, but it returns wrong for "#️", for
        // example, which is an emoji with text presentation as default. So use icu_properties
        // for detecting that as well.
        let first_char = cluster_chars.next().unwrap();
        if self.emoji.contains(first_char) {
            let mut color_emoji_preference = None;
            if let Some(second_char) = cluster_chars.next() {
                if second_char == VARIANT_SELECTOR_PREFER_TEXT {
                    color_emoji_preference = Some(false);
                }
                // Modifiers like skin color should also force emoji representation
                else if second_char == VARIANT_SELECTOR_PREFER_EMOJI
                    || self.emoji_modifier.contains(second_char)
                {
                    color_emoji_preference = Some(true);
                }
            }
            // Use the unicde default presentation when no preference is given
            return color_emoji_preference
                .unwrap_or_else(|| self.emoji_presentation.contains(first_char));
        }
        false
    }
}
