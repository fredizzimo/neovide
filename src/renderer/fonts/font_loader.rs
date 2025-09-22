use std::{
    fmt::{Display, Formatter},
    num::NonZeroUsize,
    rc::Rc,
};

use log::trace;
use lru::LruCache;
use skia_safe::{
    font::Edging as SkiaEdging, Data, Font, FontHinting as SkiaHinting, FontMgr, FontStyle, Typeface
};
use swash::{shape::ShapeContext, Metrics, text::{cluster::{CharCluster, Parser, Status, Token}, Script}};

use crate::{
    profiling::tracy_zone,
    renderer::fonts::{
        font_options::{CoarseStyle, FontDescription, FontEdging, FontHinting},
        swash_font::SwashFont,
    },
};

static DEFAULT_FONT: &[u8] = include_bytes!("../../../assets/fonts/FiraCodeNerdFont-Regular.ttf");
static LAST_RESORT_FONT: &[u8] = include_bytes!("../../../assets/fonts/LastResort-Regular.ttf");

pub struct FontPair {
    pub key: FontKey,
    pub skia_font: Font,
    pub swash_font: SwashFont,
    pub font_info: Option<(Metrics, f32)>,
}

fn info_for_font(shape_context: &mut ShapeContext, emoji: bool, font: &SwashFont) -> (Metrics, f32) {
    let mut shaper = shape_context.builder(font.as_ref()).build();
    // Note the second character is a a double width space
    let metrics = shaper.metrics();
    let mut advance = metrics.average_width;
    if emoji {
        shaper.add_str("☺️");
        shaper.shape_with(|cluster| {
            advance = cluster.glyphs.first().map_or(metrics.average_width, |g| {
                g.advance / metrics.units_per_em as f32
            });
        });
        advance /= 2.0;
    }
    else {

        let double_width_space = '　';
        let token = Token {
            ch: double_width_space,
            offset: 0,
            len: double_width_space.len_utf8() as u8,
            info: double_width_space.into(),
            data: 0,
        };
        let tokens = [token];
        let mut parser = Parser::new(Script::Latin, tokens.into_iter());
        let mut cluster = CharCluster::new();
        parser.next(&mut cluster);
        let charmap = font.as_ref().charmap();
        let mut has_double_width_space = false;
        match cluster.map(|ch| charmap.map(ch)) {
            Status::Complete => {has_double_width_space = true}
            Status::Keep => {},
            Status::Discard => {}
        }
        log::info!("Has double width space {has_double_width_space}");
        if has_double_width_space {
            shaper.add_str("　");
        } else {
            shaper.add_str("M");
        }
        shaper.shape_with(|cluster| {
            advance = cluster.glyphs.first().map_or(metrics.average_width, |g| {
                g.advance / metrics.units_per_em as f32
            });
        });
        if has_double_width_space {
            advance /= 2.0;
        }
    }
    log::info!("{metrics:#?}");
    log::info!("Advance: {advance}");
    (metrics, advance)
}

impl FontPair {
    fn new(
        key: FontKey,
        typeface: Typeface,
        emoji: bool,
        shaper: Option<&mut ShapeContext>,
    ) -> Option<FontPair> {
        log::info!("Loading font pair {}", typeface.family_name());
        let (font_data, index) = typeface.to_font_data()?;
        // Only the lower 16 bits are part of the index, the rest indicates named instances. But we
        // don't care about those here, since we are just loading the font, so ignore them
        let index = index & 0xFFFF;
        let swash_font = SwashFont::from_data(font_data, index)?;
        let font_info = shaper.map(|shaper| info_for_font(shaper, emoji, &swash_font));
        let mut skia_font = Font::from_typeface(typeface, None);
        skia_font.set_subpixel(true);
        skia_font.set_baseline_snap(true);
        skia_font.set_hinting(font_hinting(&key.hinting));
        skia_font.set_edging(font_edging(&key.edging));

        Some(Self {
            key,
            skia_font,
            swash_font,
            font_info,
        })
    }
}

impl PartialEq for FontPair {
    fn eq(&self, other: &Self) -> bool {
        self.swash_font.key == other.swash_font.key
    }
}

#[derive(Debug, Default, Hash, PartialEq, Eq, Clone)]
pub struct FontKey {
    // TODO(smolck): Could make these private and add constructor method(s)?
    // Would theoretically make things safer I guess, but not sure . . .
    pub font_desc: Option<FontDescription>,
    pub hinting: FontHinting,
    pub edging: FontEdging,
}

pub struct FontLoader {
    font_mgr: FontMgr,
    cache: LruCache<FontKey, Rc<FontPair>>,
    last_resort: Option<Rc<FontPair>>,
    emoji: Option<Rc<FontPair>>,
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
    pub fn new() -> FontLoader {
        FontLoader {
            font_mgr: FontMgr::new(),
            cache: LruCache::new(NonZeroUsize::new(20).unwrap()),
            last_resort: None,
            emoji: None,
        }
    }

    fn load(&mut self, font_key: FontKey, shaper: Option<&mut ShapeContext>) -> Option<FontPair> {
        tracy_zone!("load_font");
        trace!("Loading font {font_key:?}");
        if let Some(desc) = &font_key.font_desc {
            let (family, style) = desc.as_family_and_font_style();
            let typeface = self.font_mgr.match_family_style(family, style)?;
            FontPair::new(font_key, typeface, false, shaper)
        } else {
            let data = Data::new_copy(DEFAULT_FONT);
            let typeface = self.font_mgr.new_from_data(&data, 0)?;
            FontPair::new(font_key, typeface, false, shaper)
        }
    }

    pub fn get_or_load(
        &mut self,
        font_key: &FontKey,
        shaper: Option<&mut ShapeContext>,
    ) -> Option<Rc<FontPair>> {
        if let Some(cached) = self.cache.get(font_key) {
            return Some(cached.clone());
        }

        let loaded_font = self.load(font_key.clone(), shaper)?;
        let font_rc = Rc::new(loaded_font);
        self.cache.put(font_key.clone(), font_rc.clone());

        Some(font_rc)
    }

    pub fn load_font_for_character(
        &mut self,
        coarse_style: CoarseStyle,
        character: char,
        shaper: Option<&mut ShapeContext>,
    ) -> Option<Rc<FontPair>> {
        let font_style = coarse_style.into();
        let typeface =
            self.font_mgr
                .match_family_style_character("", font_style, &[], character as i32)?;

        let font_key = FontKey {
            font_desc: Some(FontDescription {
                family: typeface.family_name(),
                style: coarse_style.name().map(str::to_string),
            }),
            hinting: FontHinting::default(),
            edging: FontEdging::default(),
        };

        let font_pair = Rc::new(FontPair::new(font_key.clone(), typeface, false, shaper)?);

        self.cache.put(font_key, font_pair.clone());

        Some(font_pair)
    }

    pub fn get_or_load_last_resort(
        &mut self,
        shaper: Option<&mut ShapeContext>,
    ) -> Option<Rc<FontPair>> {
        if self.last_resort.is_some() {
            self.last_resort.clone()
        } else {
            let font_key = FontKey::default();
            let data = Data::new_copy(LAST_RESORT_FONT);

            let typeface = self.font_mgr.new_from_data(&data, 0)?;
            let font_pair = Rc::new(FontPair::new(font_key, typeface, false, shaper)?);

            self.last_resort = Some(font_pair.clone());
            Some(font_pair)
        }
    }

    pub fn get_or_load_emoji(&mut self, character: char, shaper: Option<&mut ShapeContext>) -> Option<Rc<FontPair>>{
        if self.emoji.is_some() {
            return self.emoji.clone();
        }
        let mut typeface = None;
        #[cfg(target_os = "macos")]
        {
            typeface = self.font_mgr.match_family_style("Apple Color Emoji", FontStyle::normal);
        }
        if typeface.is_none() {
            let locale = "und-Zsye";
            typeface =
                self.font_mgr
                    .match_family_style_character("", FontStyle::normal(), &[locale], character as i32);
        }
        let typeface = typeface?;

        let font_key = FontKey {
            font_desc: Some(FontDescription {
                family: typeface.family_name(),
                style: None,
            }),
            hinting: FontHinting::default(),
            edging: FontEdging::default(),
        };

        let font_pair = Rc::new(FontPair::new(font_key.clone(), typeface, true, shaper)?);

        self.emoji = Some(font_pair.clone());
        Some(font_pair)
    }

    pub fn loaded_fonts(&self) -> Vec<Rc<FontPair>> {
        self.cache.iter().map(|(_, v)| v.clone()).collect()
    }

    pub fn refresh(&mut self, font_pair: &FontPair) {
        self.cache.get(&font_pair.key);
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
