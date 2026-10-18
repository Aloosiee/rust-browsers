//! Font discovery, glyph caching and text measurement.
//!
//! Looks for common sans, serif and monospace families in the platform font directories and
//! falls back per character to any loaded font that has a glyph.

use crate::style::Family;
use fontdue::{Font, FontSettings, Metrics};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub type FontId = usize;

/// Loaded fonts plus caches for advances, rasterized glyphs and per-character fallbacks.
pub struct FontStore {
    fonts: Vec<Font>,
    variants: HashMap<(Family, bool, bool), FontId>,
    advances: RefCell<HashMap<(FontId, char, u32), f32>>,
    glyphs: RefCell<HashMap<(FontId, char, u32), Rc<(Metrics, Vec<u8>)>>>,
    fallback: RefCell<HashMap<(FontId, char), FontId>>,
}

fn candidates(family: Family, bold: bool, italic: bool) -> Vec<PathBuf> {
    let style_idx = (bold as usize) + 2 * (italic as usize);
    let linux_dirs = [
        "/usr/share/fonts/truetype/dejavu",
        "/usr/share/fonts/dejavu",
        "/usr/share/fonts/TTF",
        "/usr/share/fonts/truetype/liberation",
        "/usr/share/fonts/truetype/liberation2",
        "/usr/share/fonts/liberation",
        "/usr/share/fonts/liberation-sans",
        "/usr/share/fonts/liberation-serif",
        "/usr/share/fonts/liberation-mono",
        "/usr/share/fonts/truetype/noto",
        "/usr/share/fonts/noto",
        "/usr/local/share/fonts",
    ];
    let names: [&[&str]; 4] = match family {
        Family::Sans => [
            &[
                "DejaVuSans.ttf",
                "LiberationSans-Regular.ttf",
                "NotoSans-Regular.ttf",
                "Arial.ttf",
                "arial.ttf",
            ],
            &[
                "DejaVuSans-Bold.ttf",
                "LiberationSans-Bold.ttf",
                "NotoSans-Bold.ttf",
                "Arial Bold.ttf",
                "arialbd.ttf",
            ],
            &[
                "DejaVuSans-Oblique.ttf",
                "LiberationSans-Italic.ttf",
                "NotoSans-Italic.ttf",
                "Arial Italic.ttf",
                "ariali.ttf",
            ],
            &[
                "DejaVuSans-BoldOblique.ttf",
                "LiberationSans-BoldItalic.ttf",
                "NotoSans-BoldItalic.ttf",
                "Arial Bold Italic.ttf",
                "arialbi.ttf",
            ],
        ],
        Family::Serif => [
            &[
                "DejaVuSerif.ttf",
                "LiberationSerif-Regular.ttf",
                "NotoSerif-Regular.ttf",
                "Times New Roman.ttf",
                "times.ttf",
            ],
            &[
                "DejaVuSerif-Bold.ttf",
                "LiberationSerif-Bold.ttf",
                "NotoSerif-Bold.ttf",
                "Times New Roman Bold.ttf",
                "timesbd.ttf",
            ],
            &[
                "DejaVuSerif-Italic.ttf",
                "LiberationSerif-Italic.ttf",
                "NotoSerif-Italic.ttf",
                "Times New Roman Italic.ttf",
                "timesi.ttf",
            ],
            &[
                "DejaVuSerif-BoldItalic.ttf",
                "LiberationSerif-BoldItalic.ttf",
                "NotoSerif-BoldItalic.ttf",
                "Times New Roman Bold Italic.ttf",
                "timesbi.ttf",
            ],
        ],
        Family::Mono => [
            &[
                "DejaVuSansMono.ttf",
                "LiberationMono-Regular.ttf",
                "NotoSansMono-Regular.ttf",
                "Courier New.ttf",
                "consola.ttf",
                "cour.ttf",
            ],
            &[
                "DejaVuSansMono-Bold.ttf",
                "LiberationMono-Bold.ttf",
                "NotoSansMono-Bold.ttf",
                "Courier New Bold.ttf",
                "consolab.ttf",
                "courbd.ttf",
            ],
            &[
                "DejaVuSansMono-Oblique.ttf",
                "LiberationMono-Italic.ttf",
                "Courier New Italic.ttf",
                "consolai.ttf",
                "couri.ttf",
            ],
            &[
                "DejaVuSansMono-BoldOblique.ttf",
                "LiberationMono-BoldItalic.ttf",
                "Courier New Bold Italic.ttf",
                "consolaz.ttf",
                "courbi.ttf",
            ],
        ],
    };
    let mut dirs: Vec<PathBuf> = linux_dirs.iter().map(PathBuf::from).collect();
    dirs.push(PathBuf::from("/System/Library/Fonts/Supplemental"));
    dirs.push(PathBuf::from("/Library/Fonts"));
    if let Ok(windir) = std::env::var("WINDIR") {
        dirs.push(Path::new(&windir).join("Fonts"));
    }
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(Path::new(&home).join(".fonts"));
        dirs.push(Path::new(&home).join(".local/share/fonts"));
        dirs.push(Path::new(&home).join("Library/Fonts"));
    }
    let mut out = Vec::new();
    for name in names[style_idx] {
        for d in &dirs {
            out.push(d.join(name));
        }
    }
    out
}

fn find_any_ttf(dir: &Path, depth: usize) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            subdirs.push(p);
        } else if p.extension().map_or(false, |x| x.eq_ignore_ascii_case("ttf")) {
            return Some(p);
        }
    }
    if depth == 0 {
        return None;
    }
    subdirs.into_iter().find_map(|d| find_any_ttf(&d, depth - 1))
}

impl FontStore {
    /// Loads every variant it can find. Fails only if no TrueType font exists at all.
    pub fn load() -> Result<FontStore, String> {
        let mut fonts = Vec::new();
        let mut loaded: HashMap<PathBuf, FontId> = HashMap::new();
        let mut variants = HashMap::new();
        for family in [Family::Sans, Family::Serif, Family::Mono] {
            for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
                for path in candidates(family, bold, italic) {
                    if let Some(&id) = loaded.get(&path) {
                        variants.insert((family, bold, italic), id);
                        break;
                    }
                    let Ok(bytes) = std::fs::read(&path) else { continue };
                    if let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
                        fonts.push(font);
                        loaded.insert(path, fonts.len() - 1);
                        variants.insert((family, bold, italic), fonts.len() - 1);
                        break;
                    }
                }
            }
        }
        if fonts.is_empty() {
            let roots = ["/usr/share/fonts", "/System/Library/Fonts", "C:\\Windows\\Fonts"];
            for root in roots {
                if let Some(p) = find_any_ttf(Path::new(root), 3) {
                    if let Some(font) = std::fs::read(&p)
                        .ok()
                        .and_then(|b| Font::from_bytes(b, FontSettings::default()).ok())
                    {
                        fonts.push(font);
                        variants.insert((Family::Sans, false, false), 0);
                        break;
                    }
                }
            }
        }
        if fonts.is_empty() {
            return Err("No TrueType fonts found. Install DejaVu or Liberation fonts.".into());
        }
        Ok(FontStore {
            fonts,
            variants,
            advances: RefCell::new(HashMap::new()),
            glyphs: RefCell::new(HashMap::new()),
            fallback: RefCell::new(HashMap::new()),
        })
    }

    /// Best available font for a family and style, degrading weight, then slant, then family.
    pub fn font_for(&self, family: Family, bold: bool, italic: bool) -> FontId {
        let order = [
            (family, bold, italic),
            (family, bold, false),
            (family, false, italic),
            (family, false, false),
            (Family::Sans, bold, italic),
            (Family::Sans, bold, false),
            (Family::Sans, false, false),
            (Family::Serif, false, false),
            (Family::Mono, false, false),
        ];
        order.iter().find_map(|k| self.variants.get(k).copied()).unwrap_or(0)
    }

    /// Picks the font that will draw `c`, falling back to any loaded font that has the glyph.
    fn resolve(&self, font: FontId, c: char) -> FontId {
        if self.fonts[font].lookup_glyph_index(c) != 0 || c.is_control() {
            return font;
        }
        *self.fallback.borrow_mut().entry((font, c)).or_insert_with(|| {
            (0..self.fonts.len())
                .find(|&i| self.fonts[i].lookup_glyph_index(c) != 0)
                .unwrap_or(font)
        })
    }

    /// Quantizes a font size to half pixels so cache keys stay small.
    fn key(size: f32) -> u32 {
        (size * 2.0).round().max(2.0) as u32
    }

    pub fn advance(&self, font: FontId, c: char, size: f32) -> f32 {
        let k = Self::key(size);
        if let Some(&a) = self.advances.borrow().get(&(font, c, k)) {
            return a;
        }
        let f = self.resolve(font, c);
        let a = self.fonts[f].metrics(c, k as f32 / 2.0).advance_width;
        self.advances.borrow_mut().insert((font, c, k), a);
        a
    }

    /// Width of a string in pixels, without kerning.
    pub fn measure(&self, text: &str, font: FontId, size: f32) -> f32 {
        text.chars().map(|c| self.advance(font, c, size)).sum()
    }

    pub fn ascent_descent(&self, font: FontId, size: f32) -> (f32, f32) {
        let size = Self::key(size) as f32 / 2.0;
        match self.fonts[font].horizontal_line_metrics(size) {
            Some(m) => (m.ascent, -m.descent),
            None => (size * 0.8, size * 0.2),
        }
    }

    pub fn glyph(&self, font: FontId, c: char, size: f32) -> Rc<(Metrics, Vec<u8>)> {
        let k = Self::key(size);
        if let Some(g) = self.glyphs.borrow().get(&(font, c, k)) {
            return g.clone();
        }
        let f = self.resolve(font, c);
        let g = Rc::new(self.fonts[f].rasterize(c, k as f32 / 2.0));
        let mut cache = self.glyphs.borrow_mut();
        if cache.len() > 20000 {
            cache.clear();
        }
        cache.insert((font, c, k), g.clone());
        g
    }
}
