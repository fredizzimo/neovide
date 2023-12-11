use std::{
    cell::RefCell,
    collections::HashMap,
    ops::DerefMut,
    sync::{Arc, Mutex},
};

use itertools::Itertools;
use log::{debug, error, trace, warn};
use quick_cache::sync::Cache;
use skia_safe::{
    graphics::{font_cache_limit, font_cache_used, set_font_cache_limit},
    TextBlob, TextBlobBuilder,
};
use swash::{
    shape::ShapeContext,
    text::{
        cluster::{CharCluster, Parser, Status, Token},
        Script,
    },
    Metrics,
};
use thread_local::ThreadLocal;
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    error_msg,
    profiling::tracy_zone,
    renderer::fonts::{font_loader::*, font_options::*},
};

#[derive(new, Clone, Hash, PartialEq, Eq, Debug)]
struct ShapeKey {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone, Hash, PartialEq, Eq, Debug)]
struct StyleKey {
    pub bold: bool,
    pub italic: bool,
}

impl From<&FontKey> for StyleKey {
    fn from(font_key: &FontKey) -> Self {
        Self {
            bold: font_key.bold,
            italic: font_key.italic,
        }
    }
}

type Fallbacks = Vec<Arc<FontPair>>;

struct ThreadLocalState {
    shape_context: ShapeContext,
    fonts: HashMap<StyleKey, Fallbacks>,
    last_resort: Option<Arc<FontPair>>,
}

const CACHE_SIZE: usize = 10000;

pub struct CachingShaper {
    options: FontOptions,
    blob_cache: Cache<ShapeKey, Arc<Vec<TextBlob>>>,
    scale_factor: f32,
    fudge_factor: f32,
    linespace: i64,
    font_info: Option<(Metrics, f32)>,
    thread_state: ThreadLocal<RefCell<ThreadLocalState>>,
    font_loader: Mutex<FontLoader>,
}

impl CachingShaper {
    pub fn new(scale_factor: f32) -> CachingShaper {
        let options = FontOptions::default();
        let font_size = options.size * scale_factor;
        let mut shaper = CachingShaper {
            options,
            blob_cache: Cache::new(CACHE_SIZE),
            scale_factor,
            fudge_factor: 1.0,
            linespace: 0,
            font_info: None,
            thread_state: ThreadLocal::default(),
            font_loader: FontLoader::new(font_size).into(),
        };
        shaper.reset_font_loader();
        shaper
    }

    fn get_thread_state(&self) -> &RefCell<ThreadLocalState> {
        self.thread_state.get_or(|| {
            RefCell::new(ThreadLocalState {
                shape_context: ShapeContext::new(),
                fonts: HashMap::new(),
                last_resort: None,
            })
        })
    }

    fn current_font_pair(&mut self) -> Arc<FontPair> {
        self.font_loader
            .get_mut()
            .unwrap()
            .get_or_load(&FontKey {
                italic: false,
                bold: false,
                family_name: self.options.primary_font(),
                hinting: self.options.hinting.clone(),
                edging: self.options.edging.clone(),
            })
            .unwrap_or_else(|| {
                self.font_loader
                    .get_mut()
                    .unwrap()
                    .get_or_load(&FontKey::default())
                    .expect("Could not load default font")
            })
    }

    pub fn current_size(&self) -> f32 {
        self.options.size * self.scale_factor * self.fudge_factor
    }

    pub fn update_scale_factor(&mut self, scale_factor: f32) {
        debug!("scale_factor changed: {:.2}", scale_factor);
        self.scale_factor = scale_factor;
        self.reset_font_loader();
    }

    pub fn update_font(&mut self, guifont_setting: &str) {
        debug!("Updating font: {}", guifont_setting);

        let options = match FontOptions::parse(guifont_setting) {
            Ok(opt) => opt,
            Err(msg) => {
                error_msg!("Failed to parse guifont: {}", msg);
                return;
            }
        };

        let failed_fonts = {
            options
                .font_list
                .iter()
                .filter(|font| {
                    let key = FontKey {
                        italic: false,
                        bold: false,
                        family_name: Some((*font).clone()),
                        hinting: options.hinting.clone(),
                        edging: options.edging.clone(),
                    };
                    self.font_loader
                        .get_mut()
                        .unwrap()
                        .get_or_load(&key)
                        .is_none()
                })
                .collect_vec()
        };

        if !failed_fonts.is_empty() {
            error_msg!(
                "Font can't be updated to: {}\n\
                Following fonts couldn't be loaded: {}",
                guifont_setting,
                failed_fonts.iter().join(", "),
            );
        }

        if failed_fonts.len() != options.font_list.len() {
            debug!("Font updated to: {}", guifont_setting);
            self.options = options;
            self.reset_font_loader();
        }
    }

    pub fn update_linespace(&mut self, linespace: i64) {
        debug!("Updating linespace: {}", linespace);

        let font_height = self.font_base_dimensions().1;
        let impossible_linespace = font_height as i64 + linespace <= 0;

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
        self.fudge_factor = 1.0;
        self.font_info = None;
        let mut font_size = self.current_size();
        debug!("Original font_size: {:.2}px", font_size);

        *self.font_loader.get_mut().unwrap() = FontLoader::new(font_size);
        self.update_info();
        let (metrics, font_width) = self.info();

        debug!("Original font_width: {:.2}px", font_width);

        if !self.options.allow_float_size {
            // Calculate the new fudge factor required to scale the font width to the nearest exact pixel
            debug!(
                "Font width: {:.2}px (avg: {:.2}px)",
                font_width, metrics.average_width
            );
            self.fudge_factor = font_width.round() / font_width;
            debug!("Fudge factor: {:.2}", self.fudge_factor);
            font_size = self.current_size();
            debug!("Fudged font size: {:.2}px", font_size);
            debug!("Fudged font width: {:.2}px", self.info().1);
            *self.font_loader.get_mut().unwrap() = FontLoader::new(font_size);
        }
        self.thread_state = ThreadLocal::default();
        self.blob_cache = Cache::new(CACHE_SIZE);
    }

    pub fn font_names(&self) -> Vec<String> {
        self.font_loader.lock().unwrap().font_names()
    }

    fn update_info(&mut self) {
        let font_pair = self.current_font_pair();
        let size = self.current_size();
        self.font_info = {
            let local_state = self.get_thread_state();
            let mut local_state = local_state.borrow_mut();
            let mut shaper = local_state
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
            Some((metrics, advance))
        };
    }

    fn info(&self) -> (Metrics, f32) {
        self.font_info.unwrap()
    }

    fn metrics(&self) -> Metrics {
        self.info().0
    }

    pub fn font_base_dimensions(&self) -> (u64, u64) {
        let (metrics, glyph_advance) = self.info();

        let bare_font_height = (metrics.ascent + metrics.descent + metrics.leading).ceil();
        let font_height = bare_font_height as i64 + self.linespace;
        let font_width = (glyph_advance + self.options.width + 0.5).floor() as u64;

        (
            font_width,
            font_height as u64, // assuming that linespace is checked on receive for
                                // validity
        )
    }

    pub fn underline_position(&self) -> u64 {
        self.metrics().underline_offset as u64
    }

    pub fn y_adjustment(&self) -> u64 {
        let metrics = self.metrics();
        (metrics.ascent + metrics.leading + self.linespace as f32 / 2.).ceil() as u64
    }

    fn get_fallback_list<'a>(
        &'a self,
        font_key: &FontKey,
        fonts: &'a mut HashMap<StyleKey, Fallbacks>,
    ) -> &'a mut Fallbacks {
        fonts.entry(font_key.into()).or_insert_with(|| {
            let mut font_loader = self.font_loader.lock().unwrap();
            // Add all the configured fonts and the Neovide default fonts
            // System fallback fonts will be added on demand
            self.options
                .font_list
                .iter()
                .map(|font_name| FontKey {
                    italic: font_key.italic,
                    bold: font_key.bold,
                    family_name: Some(font_name.clone()),
                    hinting: self.options.hinting.clone(),
                    edging: self.options.edging.clone(),
                })
                .chain([FontKey {
                    italic: font_key.italic,
                    bold: font_key.bold,
                    family_name: None,
                    hinting: self.options.hinting.clone(),
                    edging: self.options.edging.clone(),
                }])
                .filter_map(|font_key| font_loader.get_or_load(&font_key))
                .collect()
        })
    }

    fn load_fallback(
        &self,
        font_key: &FontKey,
        fallbacks: &mut Fallbacks,
        failed_characters: &Vec<char>,
    ) -> bool {
        tracy_zone!("load fallback");
        let mut font_loader = self.font_loader.lock().unwrap();
        let bold = font_key.bold;
        let italic = font_key.italic;
        // Try to load fonts for all failing characters in order, until it succeeds
        for ch in failed_characters {
            if let Some(font) = font_loader.load_font_for_character(bold, italic, *ch) {
                // Don't use the same font twice
                if fallbacks.iter().any(|v| *v == font) {
                    continue;
                }
                fallbacks.push(font);
                return true;
            }
        }
        // No new fallback fonts found for any character
        false
    }

    fn parse_cluster(
        &self,
        cluster: &mut CharCluster,
        font_key: &FontKey,
        fallbacks: &mut Fallbacks,
        last_resort: &mut Option<Arc<FontPair>>,
    ) -> (CharCluster, Arc<FontPair>) {
        // Use the cluster.map function to select a viable font from the fallback list
        let mut best = None;
        // cluster.map takes a Fn, which does not allow us to modify the failed_characters array,
        // so use a RefCell
        let failed_characters = RefCell::new(Vec::new());
        for font_pair in fallbacks.iter() {
            let charmap = font_pair.swash_font.as_ref().charmap();
            match cluster.map(|ch| {
                let res = charmap.map(ch);
                if res == 0 {
                    failed_characters.borrow_mut().push(ch);
                }
                res
            }) {
                Status::Complete => {
                    return (cluster.to_owned(), font_pair.clone());
                }
                Status::Keep => best = Some(Arc::clone(font_pair)),
                Status::Discard => {}
            }
        }

        if self.load_fallback(font_key, fallbacks, &failed_characters.borrow()) {
            // If a new fallback font was found, retry to find the best mapping
            self.parse_cluster(cluster, font_key, fallbacks, last_resort)
        } else if let Some(best) = best {
            (cluster.to_owned(), best)
        } else {
            let fallback = if let Some(fallback) = last_resort {
                Arc::clone(fallback)
            } else {
                let mut font_loader = self.font_loader.lock().unwrap();
                let fallback = font_loader.get_or_load_last_resort();
                *last_resort = Some(Arc::clone(&fallback));
                fallback
            };
            (cluster.to_owned(), fallback)
        }
    }

    fn build_clusters(
        &self,
        text: &str,
        font_key: &FontKey,
        fallbacks: &mut Fallbacks,
        last_resort: &mut Option<Arc<FontPair>>,
    ) -> Vec<(Vec<CharCluster>, Arc<FontPair>)> {
        tracy_zone!("build_clusters");
        let mut cluster = CharCluster::new();

        // Enumerate the characters storing the glyph index in the user data so that we can position
        // glyphs according to Neovim's grid rules
        let mut character_index = 0;
        let mut parser = Parser::new(
            Script::Latin,
            text.graphemes(true)
                .enumerate()
                .flat_map(|(glyph_index, unicode_segment)| {
                    unicode_segment.chars().map(move |character| {
                        let token = Token {
                            ch: character,
                            offset: character_index as u32,
                            len: character.len_utf8() as u8,
                            info: character.into(),
                            data: glyph_index as u32,
                        };
                        character_index += 1;
                        token
                    })
                }),
        );

        let mut results = Vec::new();
        while parser.next(&mut cluster) {
            results.push(self.parse_cluster(&mut cluster, font_key, fallbacks, last_resort));
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

    pub fn adjust_font_cache_size(&self) {
        let current_font_cache_size = font_cache_limit() as f32;
        let percent_font_cache_used = font_cache_used() as f32 / current_font_cache_size;
        if percent_font_cache_used > 0.9 {
            warn!(
                "Font cache is {}% full, increasing cache size",
                percent_font_cache_used * 100.0
            );
            set_font_cache_limit((percent_font_cache_used * 1.5) as usize);
        }
    }

    pub fn shape(&self, text: String, bold: bool, italic: bool) -> Vec<TextBlob> {
        let current_size = self.current_size();
        let (glyph_width, ..) = self.font_base_dimensions();

        let mut resulting_blobs = Vec::new();

        trace!("Shaping text: {}", text);
        let mut thread_state = self.get_thread_state().borrow_mut();
        let thread_state = thread_state.deref_mut();

        let font_key = FontKey {
            italic: self.options.italic || italic,
            bold: self.options.bold || bold,
            family_name: None,
            hinting: self.options.hinting.clone(),
            edging: self.options.edging.clone(),
        };

        let fallbacks = self.get_fallback_list(&font_key, &mut thread_state.fonts);

        for (cluster_group, font_pair) in
            self.build_clusters(&text, &font_key, fallbacks, &mut thread_state.last_resort)
        {
            tracy_zone!("shape cluster");
            let mut shaper = thread_state
                .shape_context
                .builder(font_pair.swash_font.as_ref())
                .size(current_size)
                .build();

            let charmap = font_pair.swash_font.as_ref().charmap();
            for mut cluster in cluster_group {
                cluster.map(|ch| charmap.map(ch));
                shaper.add_cluster(&cluster);
            }

            let mut glyph_data = Vec::new();

            shaper.shape_with(|glyph_cluster| {
                for glyph in glyph_cluster.glyphs {
                    let position = ((glyph.data as u64 * glyph_width) as f32, glyph.y);
                    glyph_data.push((glyph.id, position));
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

        self.adjust_font_cache_size();

        resulting_blobs
    }

    pub fn shape_cached(&self, text: String, bold: bool, italic: bool) -> Arc<Vec<TextBlob>> {
        tracy_zone!("shape_cached");
        let key = ShapeKey::new(text.clone(), bold, italic);

        self.blob_cache
            .get_or_insert_with(&key, || -> Result<_, ()> {
                Ok(Arc::new(self.shape(text, bold, italic)))
            })
            .unwrap()
    }
}
