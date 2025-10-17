use std::{
    fmt::{Display, Formatter},
    num::NonZeroUsize,
    rc::Rc,
    str::FromStr,
};

use harfrust::{Feature, FontRef, Shaper, ShaperData, Tag, UnicodeBuffer};
use log::info;
use lru::LruCache;
use skia_safe::{
    font::Edging as SkiaEdging, Data, Font, FontHinting as SkiaHinting, FontMgr, Typeface,
};
use skrifa::{
    font::FontRef as SkrifaFont,
    metrics::{Decoration, Metrics},
    prelude::{LocationRef, Size},
};

use crate::{
    profiling::tracy_zone,
    renderer::fonts::font_options::{
        CoarseStyle, FontDescription, FontEdging, FontHinting, FontOptions, DEFAULT_FONT,
    },
};

static LAST_RESORT_FONT: &[u8] = include_bytes!("../../../assets/fonts/LastResort-Regular.ttf");

pub struct FontInfo {
    pub metrics: Metrics,
    pub advance: f32,
    pub emoji: bool,
}

impl FontInfo {
    pub fn scale(&self, scale: f32) -> Self {
        let metrics = Metrics {
            units_per_em: self.metrics.units_per_em,
            glyph_count: self.metrics.glyph_count,
            is_monospace: self.metrics.is_monospace,
            italic_angle: self.metrics.italic_angle,
            ascent: self.metrics.ascent * scale,
            descent: self.metrics.descent * scale,
            leading: self.metrics.leading * scale,
            cap_height: self.metrics.cap_height.map(|v| v * scale),
            x_height: self.metrics.x_height.map(|v| v * scale),
            average_width: self.metrics.average_width.map(|v| v * scale),
            max_width: self.metrics.max_width.map(|v| v * scale),
            underline: self.metrics.underline.map(|v| Decoration {
                offset: v.offset * scale,
                thickness: v.thickness * scale,
            }),
            strikeout: self.metrics.strikeout.map(|v| Decoration {
                offset: v.offset * scale,
                thickness: v.thickness * scale,
            }),
            bounds: self.metrics.bounds.map(|v| v.scale(scale)),
        };
        Self {
            metrics,
            advance: self.advance * scale,
            emoji: self.emoji,
        }
    }

    pub fn scale_offset(&self, x: i32, font_size: f32) -> f32 {
        (x as f32 / self.metrics.units_per_em as f32) * font_size
    }
}

pub struct FontPair {
    pub key: FontKey,
    pub skia_font: Font,
    pub font_info: FontInfo,
    pub features: Vec<Feature>,
    index: u32,
    font_data: Vec<u8>,
    shaper_data: ShaperData,
}

impl FontPair {
    pub fn shaper(&self) -> Shaper<'_> {
        let font_ref = FontRef::from_index(&self.font_data, self.index).unwrap();
        self.shaper_data.shaper(&font_ref).build()
    }
}

fn info_for_font(shaper_data: &ShaperData, font_ref: &FontRef, metrics: Metrics) -> FontInfo {
    let shaper = shaper_data.shaper(font_ref).build();
    let mut advance = metrics.average_width.unwrap_or(1.0);

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str("😀");
    // TODO: Hardcode
    buffer.guess_segment_properties();

    let glyphs = shaper.shape(buffer, &[]);
    let is_emoji = glyphs.len() == 1 && glyphs.glyph_infos()[0].glyph_id != 0;

    // let mut parser = Parser::new(
    //     Script::Latin,
    //     "😀".char_indices().map(move |(offset, character)| Token {
    //         ch: character,
    //         offset: offset as u32,
    //         len: character.len_utf8() as u8,
    //         info: character.into(),
    //         data: 0,
    //     }),
    // );
    // let mut emoji_cluster = CharCluster::new();
    // parser.next(&mut emoji_cluster);

    //let charmap = font.as_ref().charmap();
    //let is_emoji = emoji_cluster.map(|ch| charmap.map(ch)) == Status::Complete;
    // If the font supports emojis with variant selector 16, use that as the advance with
    // NOTE: DejaVu Sans for example will use this code path even if it's technically not an emoji font
    // But that's OK, since it still uses a double width advance for it.
    if is_emoji {
        advance = glyphs.glyph_positions()[0].x_advance as f32;
        advance /= 2.0;
    } else {
        // Load half width variant for metrics if it exists
        // This makes it possible to use many variable width CJK fonts
        let mut buffer = glyphs.clear();
        let hwid_tag = Tag::from_str("hwid").unwrap();
        let pwid_tag = Tag::from_str("pwid").unwrap();
        let features = [Feature::new(hwid_tag, 1, ..), Feature::new(pwid_tag, 0, ..)];
        buffer.push_str("M");
        // TODO: Hardcode
        buffer.guess_segment_properties();
        let glyphs = shaper.shape(buffer, &features);
        if glyphs.len() == 1 {
            advance = glyphs.glyph_positions()[0].x_advance as f32;
        }
        // shaper.add_str("M");
        // shaper.shape_with(|cluster| {
        //     advance = cluster.glyphs.first().map_or(metrics.average_width, |g| {
        //         g.advance / metrics.units_per_em as f32
        //     });
        // });
    }
    let info = FontInfo {
        metrics,
        advance,
        emoji: is_emoji,
    }
    .scale(1.0 / metrics.units_per_em as f32);
    log::info!("{:#?}", info.metrics);
    log::info!("Advance: {}, is_emoji: {}", info.advance, info.emoji);
    info
}

impl FontPair {
    fn new(key: FontKey, typeface: Typeface, features: Option<&Vec<Feature>>) -> Option<FontPair> {
        log::info!("Loading font pair {}", typeface.family_name());
        let (font_data, index) = typeface.to_font_data()?;
        // Only the lower 16 bits are part of the index, the rest indicates named instances. But we
        // don't care about those here, since we are just loading the font, so ignore them
        let index = (index & 0xFFFF) as u32;

        let font_ref = FontRef::from_index(&font_data, index).ok()?;
        let skrifa_font = SkrifaFont::from_index(&font_data, index).ok()?;
        let shaper_data = ShaperData::new(&font_ref);
        // TODO: suppport variable
        // typeface.variation_design_parameters();
        let coords = [];
        let metrics = Metrics::new(&skrifa_font, Size::unscaled(), LocationRef::new(&coords));

        let font_info = info_for_font(&shaper_data, &font_ref, metrics);
        let mut skia_font = Font::from_typeface(typeface, None);
        skia_font.set_subpixel(true);
        skia_font.set_baseline_snap(true);
        skia_font.set_hinting(font_hinting(&key.hinting));
        skia_font.set_edging(font_edging(&key.edging));

        log::info!("Typeface metrics {:#?}", skia_font.metrics());

        Some(Self {
            key,
            skia_font,
            font_info,
            shaper_data,
            index,
            font_data,
            features: features.map_or_else(Vec::new, |f| f.clone()),
        })
    }
}

// impl PartialEq for FontPair {
//     fn eq(&self, other: &Self) -> bool {
//         self.swash_font.key == other.swash_font.key
//     }
// }

#[derive(Debug, Default, Hash, PartialEq, Eq, Clone)]
pub struct FontKey {
    // TODO(smolck): Could make these private and add constructor method(s)?
    // Would theoretically make things safer I guess, but not sure . . .
    pub font_desc: FontDescription,
    pub hinting: FontHinting,
    pub edging: FontEdging,
}

pub struct FontLoader {
    font_mgr: FontMgr,
    cache: LruCache<FontKey, Rc<FontPair>>,
    last_resort: Option<Rc<FontPair>>,
    emoji: Option<Rc<FontPair>>,
    options: FontOptions,
}

impl Display for FontKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "FontKey {{ font_desc: {:?}, hinting: {:?}, edging: {:?} }}",
            self.font_desc, self.hinting, self.edging
        )
    }
}

impl FontLoader {
    pub fn new(options: FontOptions) -> FontLoader {
        FontLoader {
            font_mgr: FontMgr::new(),
            cache: LruCache::new(NonZeroUsize::new(20).unwrap()),
            last_resort: None,
            emoji: None,
            options,
        }
    }

    fn load(&mut self, font_key: FontKey) -> Option<FontPair> {
        tracy_zone!("load_font");
        info!("Loading font {font_key:?}");
        let desc = &font_key.font_desc;
        let (family, style) = desc.as_family_and_font_style();
        let typeface = self.font_mgr.match_family_style(family, style)?;
        info!("Actually loaded font {:?}", typeface.family_name());
        let features = self.options.features.get(&typeface.family_name());
        FontPair::new(font_key, typeface, features)
    }

    pub fn get_or_load(&mut self, font_key: &FontKey) -> Option<Rc<FontPair>> {
        if let Some(cached) = self.cache.get(font_key) {
            return Some(cached.clone());
        }

        let loaded_font = self.load(font_key.clone())?;
        let font_rc = Rc::new(loaded_font);
        self.cache.put(font_key.clone(), font_rc.clone());

        Some(font_rc)
    }

    pub fn load_font_for_character(
        &mut self,
        coarse_style: CoarseStyle,
        character: char,
        locales: &[&str],
    ) -> Option<Rc<FontPair>> {
        let font_style = coarse_style.into();
        let typeface = self.font_mgr.match_family_style_character(
            DEFAULT_FONT,
            font_style,
            locales,
            character as i32,
        )?;

        let font_key = FontKey {
            font_desc: FontDescription {
                family: typeface.family_name(),
                style: coarse_style.name().map(str::to_string),
            },
            hinting: FontHinting::default(),
            edging: FontEdging::default(),
        };
        if let Some(cached) = self.cache.get(&font_key) {
            return Some(cached.clone());
        }

        let features = self.options.features.get(&typeface.family_name());
        let font_pair = Rc::new(FontPair::new(font_key.clone(), typeface, features)?);
        info!(
            "Load font for character {} {}",
            character,
            font_pair.skia_font.typeface().family_name()
        );
        self.cache.put(font_key, font_pair.clone());

        Some(font_pair)
    }

    pub fn get_or_load_last_resort(&mut self) -> Option<Rc<FontPair>> {
        log::warn!("Last resort font used");
        if self.last_resort.is_some() {
            self.last_resort.clone()
        } else {
            let font_key = FontKey::default();
            let data = Data::new_copy(LAST_RESORT_FONT);

            let typeface = self.font_mgr.new_from_data(&data, 0)?;
            let features = self.options.features.get(&typeface.family_name());
            let font_pair = Rc::new(FontPair::new(font_key, typeface, features)?);

            self.last_resort = Some(font_pair.clone());
            Some(font_pair)
        }
    }

    pub fn get_or_load_emoji(&mut self, character: char) -> Option<Rc<FontPair>> {
        // It's assumed that there's only one emoji font
        if self.emoji.is_some() {
            return self.emoji.clone();
        }
        #[allow(unused_assignments)]
        let mut pair = None;

        // macOS does not support querying emoji by locale, so hardcode the system emoji font
        #[cfg(target_os = "macos")]
        {
            pair = self.get_or_load(
                &FontKey {
                    font_desc: FontDescription {
                        family: "Apple Color Emoji".to_string(),
                        style: None,
                    },
                    hinting: FontHinting::default(),
                    edging: FontEdging::default(),
                },
                Some(shaper),
            );
        }

        // The locale und-Zsye, will load color emojis by default
        if pair.is_none() {
            pair = self.load_font_for_character(CoarseStyle::default(), character, &["und-Zsye"]);
        }

        let pair = pair?;

        self.emoji = Some(pair.clone());
        Some(pair)
    }

    pub fn font_names(&self) -> Vec<String> {
        self.font_mgr.family_names().collect()
    }
}

fn font_hinting(hinting: &FontHinting) -> SkiaHinting {
    match hinting {
        FontHinting::Full => SkiaHinting::Full,
        FontHinting::Slight => SkiaHinting::Slight,
        FontHinting::Normal => SkiaHinting::Normal,
        FontHinting::None => SkiaHinting::None,
    }
}

fn font_edging(edging: &FontEdging) -> SkiaEdging {
    match edging {
        FontEdging::AntiAlias => SkiaEdging::AntiAlias,
        FontEdging::Alias => SkiaEdging::Alias,
        FontEdging::SubpixelAntiAlias => SkiaEdging::SubpixelAntiAlias,
    }
}
