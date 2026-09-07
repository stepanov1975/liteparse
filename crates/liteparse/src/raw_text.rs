//! Raw text items: pdfium's text runs with fixed, heuristic-free segmentation.
//!
//! [`extract_raw_text_items`] is the low-level counterpart of the segmenter in
//! [`crate::extract`]. It applies no layout heuristics — no gap-based line or word
//! merging, no invisible-text skip, no ligature expansion, no punctuation
//! normalisation, no dedup — so its output is a stable function of what pdfium
//! reports and a consumer can build its own segmentation on top. Every glyph pdfium
//! reports lands in exactly one item, in page order, and a glyph is never dropped on
//! its own.
//!
//! Rules:
//!
//! - An item is a run of glyphs from one text object at one angle. A glyph whose
//!   text object differs from the item's first glyph, or whose angle differs from it
//!   by more than 0.01 rad, starts a new item.
//! - A pdfium-generated `\r` or `\n` closes the open item and is dropped.
//! - Every other generated glyph, and every ASCII whitespace glyph (`\t`..`\r` and
//!   space; U+00A0, U+3000 and friends do not count), is appended and then closes the
//!   item. A pdfium-synthesised space is therefore always an item's last glyph, which
//!   is what [`RawTextItem::trailing_space_generated`] reports.
//! - Bounds are the union of the glyphs' loose boxes. Angle, colours, marked-content
//!   id, font and font metrics come from the first glyph. `text_width` sums per-glyph
//!   advance widths.
//! - A font is "buggy" when it is embedded and its name/type match the subset-font
//!   heuristic, or when any non-generated glyph decodes to a control or private-use
//!   codepoint. A glyph pdfium cannot map to Unicode decodes to 0 and therefore counts,
//!   so every Type3 item is buggy.
//! - With a [`GlyphResolver`], every non-generated glyph of a buggy item is re-decoded:
//!   Type3 glyph names first, then the resolver on the glyph outline. An unrecognised
//!   glyph becomes a space. Without a resolver the text is left as pdfium decoded it.
//! - The text is built from the glyph codepoints with C-string semantics: a 0
//!   codepoint (a glyph with no Unicode mapping) terminates the text early, and a
//!   codepoint that is not a Unicode scalar value (a surrogate, or above U+10FFFF)
//!   drops the whole item.
//!
//! Coordinates go through [`Page::bounds_to_viewport`] per glyph — the exact
//! `FPDF_PageToDevice` quantisation — rather than the affine
//! [`Page::viewport_transform`] approximation, so boxes are reproducible to the last
//! digit. The viewport is scaled by the page's `/UserUnit`, as everywhere in liteparse.

use pdfium::{Font, FontType, Page, RectF, TextPage};

use crate::GlyphResolver;
use crate::extract::{
    CharInfoChunks, CharView, decompose_scale, is_buggy_codepoint, is_buggy_font,
};
use crate::glyph_names::resolve_glyph_name_codepoint;

/// Angle difference, in radians, at which a glyph starts a new item.
const ANGLE_SPLIT_RADIANS: f32 = 0.01;

/// One raw text item, in viewport space (top-left origin, 72 dpi points).
#[derive(Debug, Clone, PartialEq)]
pub struct RawTextItem {
    /// The glyph codepoints as UTF-8. Whitespace is kept, including a trailing
    /// space; see the module docs for the NUL and invalid-scalar semantics.
    pub text: String,
    /// One entry per glyph in page order: the raw char code from the content
    /// stream, 0 for a pdfium-generated glyph.
    pub char_codes: Vec<u32>,
    /// One entry per glyph, parallel to `char_codes`, for Type3 fonts only: the
    /// PostScript glyph name the font binds to that code, `""` where it binds
    /// none. `None` for every other font type.
    pub glyph_names: Option<Vec<String>>,
    /// Counter-clockwise rotation in radians with the page rotation folded in,
    /// in `[0, 2π)`. Kept in radians so a consumer can convert in the precision
    /// it needs.
    pub angle_radians: f32,
    /// Sum of the glyphs' advance widths in text space.
    pub text_width: f32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Marked-content id of the first glyph's text object, when it has one.
    pub mcid: Option<i32>,
    /// Base font name with pdfium's subset-tag stripping; `""` when the first
    /// glyph has no text object or its object has no font.
    pub font_name: String,
    pub font_size: f32,
    pub font_weight: i32,
    /// `font_size` scaled by the vertical scale of the text matrix; 0 when the
    /// first glyph has no text object.
    pub font_height: f32,
    /// 0 when the font exposes no metrics.
    pub font_ascent: f32,
    /// 0 when the font exposes no metrics.
    pub font_descent: f32,
    pub font_is_buggy: bool,
    /// The last glyph is a pdfium-synthesised space, not a space glyph from the
    /// content stream.
    pub trailing_space_generated: bool,
    /// First glyph's fill colour packed as ARGB, when its colour space is
    /// reportable as RGB.
    pub fill_color: Option<u32>,
    /// First glyph's stroke colour packed as ARGB, when reportable.
    pub stroke_color: Option<u32>,
}

/// Everything read per glyph before items are formed.
struct Glyph {
    index: i32,
    text_object: Option<pdfium::pdfium_sys::FPDF_PAGEOBJECT>,
    generated: bool,
    /// 0 for a generated glyph, which has no source char code.
    char_code: u32,
    /// 0 when pdfium reports a unicode-map error for the glyph.
    unicode: u32,
    /// Loose glyph box in viewport space.
    loose: RectF,
    /// Normalised angle in radians, `[0, 2π)`.
    angle: f32,
}

/// Extract the raw text items of `text_page`. `view_box` is the page's crop box
/// ([`Page::view_box`]); `glyph_resolver` is the optional outline-based recovery
/// for buggy fonts (see the module docs), `None` to leave buggy text as decoded.
pub fn extract_raw_text_items(
    page: &Page,
    text_page: &TextPage,
    view_box: &RectF,
    glyph_resolver: Option<&dyn GlyphResolver>,
) -> Vec<RawTextItem> {
    let char_count = text_page.char_count();
    let page_rotation = page.rotation();
    let mut chunks = CharInfoChunks::new(text_page);
    let mut items = Vec::new();
    let mut run: Vec<Glyph> = Vec::new();

    let flush = |run: &mut Vec<Glyph>, items: &mut Vec<RawTextItem>| {
        if run.is_empty() {
            return;
        }
        if let Some(item) = build_item(text_page, run, glyph_resolver) {
            items.push(item);
        }
        run.clear();
    };

    for i in 0..char_count {
        let ch = text_page.char_at_unchecked(i);
        let cv = CharView {
            ch: &ch,
            rec: chunks.as_mut().and_then(|chunks| chunks.record(i)),
        };
        let glyph = load_glyph(page, view_box, page_rotation, &cv, i);

        if let Some(first) = run.first()
            && (first.text_object != glyph.text_object
                || (first.angle - glyph.angle).abs() > ANGLE_SPLIT_RADIANS)
        {
            flush(&mut run, &mut items);
        }

        if glyph.generated && matches!(glyph.unicode, 0x0A | 0x0D) {
            flush(&mut run, &mut items);
            continue;
        }

        let closes_item = glyph.generated || is_c_locale_space(glyph.unicode);
        run.push(glyph);
        if closes_item {
            flush(&mut run, &mut items);
        }
    }
    flush(&mut run, &mut items);
    items
}

/// The per-glyph reads. A glyph whose loose box cannot be read falls back to its
/// strict box.
fn load_glyph(
    page: &Page,
    view_box: &RectF,
    page_rotation: i32,
    cv: &CharView<'_, '_>,
    index: i32,
) -> Glyph {
    let generated = cv.is_generated();
    let char_code = if generated { 0 } else { cv.char_code() };
    let unicode = if cv.has_unicode_map_error() {
        0
    } else {
        cv.unicode()
    };
    let strict = cv.strict_char_box().unwrap_or_default();
    let loose = cv.loose_char_box().unwrap_or(strict);
    Glyph {
        index,
        text_object: cv.text_object(),
        generated,
        char_code,
        unicode,
        loose: page.bounds_to_viewport(view_box, &loose),
        angle: normalize_angle(cv.ch.angle(), page_rotation),
    }
}

/// Fold the page's `/Rotate` into pdfium's counter-clockwise glyph angle and
/// wrap it into `[0, 2π)`. Computed in f64 so the wrap does not lose precision.
fn normalize_angle(angle_radians: f32, page_rotation: i32) -> f32 {
    use std::f64::consts::PI;
    let mut angle = f64::from(angle_radians);
    match page_rotation {
        1 => angle -= 3.0 * PI / 2.0,
        2 => angle -= PI,
        3 => angle -= PI / 2.0,
        _ => {}
    }
    angle %= 2.0 * PI;
    if angle < 0.0 {
        angle += 2.0 * PI;
    }
    angle as f32
}

/// ASCII whitespace only — deliberately not Unicode `White_Space`, so non-breaking
/// and ideographic spaces stay inside their items.
fn is_c_locale_space(unicode: u32) -> bool {
    matches!(unicode, 0x09..=0x0D | 0x20)
}

fn pack_argb(color: pdfium::Color) -> u32 {
    u32::from_be_bytes([color.a, color.r, color.g, color.b])
}

/// Turn a run of glyphs into an item. `None` when a codepoint is not a Unicode
/// scalar value (see [`c_string_from_utf32`]).
fn build_item(
    text_page: &TextPage,
    glyphs: &[Glyph],
    glyph_resolver: Option<&dyn GlyphResolver>,
) -> Option<RawTextItem> {
    let first = glyphs.first()?;
    let first_char = text_page.char_at_unchecked(first.index);
    let font_size = first_char.font_size() as f32;
    let font = first
        .text_object
        .and_then(|obj| unsafe { Font::from_text_object(obj) });

    let mut bounds = first.loose;
    let mut text_width = 0.0f32;
    let mut font_name = String::new();
    let mut font_ascent = 0.0f32;
    let mut font_descent = 0.0f32;
    let mut font_is_embedded = false;
    let mut font_is_buggy = false;
    if let Some(font) = &font {
        font_name = font.base_name().unwrap_or_default();
        font_ascent = font.ascent(font_size).unwrap_or(0.0);
        font_descent = font.descent(font_size).unwrap_or(0.0);
        font_is_embedded = font.is_embedded();
        font_is_buggy = font_is_embedded && is_buggy_font(&font_name, font.font_type());
        if let Some(width) = font.glyph_width_from_char_code(first.char_code, font_size) {
            text_width += width;
        }
    }
    let font_height = if first.text_object.is_some() {
        let scale_y = first_char
            .matrix()
            .map(|matrix| decompose_scale(&matrix).1)
            .unwrap_or(1.0);
        font_size * scale_y
    } else {
        0.0
    };

    for glyph in &glyphs[1..] {
        bounds.left = bounds.left.min(glyph.loose.left);
        bounds.top = bounds.top.min(glyph.loose.top);
        bounds.right = bounds.right.max(glyph.loose.right);
        bounds.bottom = bounds.bottom.max(glyph.loose.bottom);
        if let Some(font) = &font {
            let width = if glyph.generated {
                font.glyph_width(glyph.unicode, font_size)
            } else {
                font.glyph_width_from_char_code(glyph.char_code, font_size)
            };
            if let Some(width) = width {
                text_width += width;
            }
        }
    }
    // A control / private-use decode in an embedded font flags the whole item.
    // `unicode` is 0 for an unmapped glyph, so those count too.
    if font_is_embedded && !font_is_buggy {
        font_is_buggy = glyphs
            .iter()
            .any(|glyph| !glyph.generated && is_buggy_codepoint(glyph.unicode));
    }

    let mut codepoints: Vec<u32> = glyphs.iter().map(|glyph| glyph.unicode).collect();
    if font_is_buggy && let (Some(font), Some(resolver)) = (&font, glyph_resolver) {
        let is_type3 = font.font_type() == FontType::Type3;
        for (glyph, codepoint) in glyphs.iter().zip(codepoints.iter_mut()) {
            if glyph.generated {
                continue;
            }
            let identified = is_type3
                .then(|| type3_glyph_codepoint(font, glyph.char_code))
                .flatten()
                .or_else(|| identify_glyph(resolver, font, glyph.char_code));
            *codepoint = identified.unwrap_or(u32::from(' '));
        }
    }
    let text = c_string_from_utf32(&codepoints)?;

    let glyph_names = font
        .as_ref()
        .filter(|font| font.font_type() == FontType::Type3)
        .map(|font| {
            glyphs
                .iter()
                .map(|glyph| font.char_glyph_name(glyph.char_code).unwrap_or_default())
                .collect()
        });

    Some(RawTextItem {
        text,
        char_codes: glyphs.iter().map(|glyph| glyph.char_code).collect(),
        glyph_names,
        angle_radians: first.angle,
        text_width,
        x: bounds.left,
        y: bounds.top,
        width: bounds.right - bounds.left,
        height: bounds.bottom - bounds.top,
        mcid: first_char.marked_content_id(),
        font_name,
        font_size,
        font_weight: first_char.font_weight(),
        font_height,
        font_ascent,
        font_descent,
        font_is_buggy,
        trailing_space_generated: glyphs.last().is_some_and(|glyph| glyph.generated),
        fill_color: first_char.fill_color().map(pack_argb),
        stroke_color: first_char.stroke_color().map(pack_argb),
    })
}

/// The codepoint a Type3 font's own `/Encoding` `/Differences` name resolves to.
/// A Type3 font has no font program, so this name is the only statement the
/// document makes about what the glyph means.
fn type3_glyph_codepoint(font: &Font, char_code: u32) -> Option<u32> {
    let name = font.char_glyph_name(char_code)?;
    resolve_glyph_name_codepoint(&name)
}

/// The resolver's answer for the glyph outline. A glyph with no outline
/// (whitespace, non-rendered) is unidentified.
fn identify_glyph(resolver: &dyn GlyphResolver, font: &Font, char_code: u32) -> Option<u32> {
    let segments = font.glyph_path_segments(char_code, crate::GLYPH_RESOLVER_FONT_SIZE)?;
    resolver.resolve_codepoint(&segments)
}

/// Build the item text from raw codepoints with C-string semantics: `None` when
/// any value is not a Unicode scalar value (the whole item is dropped), otherwise
/// the text up to the first 0 — a 0 is an unmapped glyph, and it terminates the
/// string rather than being replaced.
fn c_string_from_utf32(codepoints: &[u32]) -> Option<String> {
    let mut chars = Vec::with_capacity(codepoints.len());
    for &codepoint in codepoints {
        match char::from_u32(codepoint) {
            Some(c) => chars.push(c),
            None => return None,
        }
    }
    Some(chars.into_iter().take_while(|&c| c != '\0').collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_locale_space_is_ascii_only() {
        for space in [0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x20] {
            assert!(is_c_locale_space(space), "{space:#x}");
        }
        for not_space in [
            0x00, 0x08, 0x0E, 0x1F, 0x21, 0x85, 0xA0, 0x2003, 0x202F, 0x3000,
        ] {
            assert!(!is_c_locale_space(not_space), "{not_space:#x}");
        }
    }

    #[test]
    fn utf32_conversion_keeps_c_string_semantics() {
        assert_eq!(
            c_string_from_utf32(&[0x48, 0x69, 0x20]).as_deref(),
            Some("Hi ")
        );
        // A 0 terminates the string; what follows is dropped, not the item.
        assert_eq!(
            c_string_from_utf32(&[0x48, 0x00, 0x69]).as_deref(),
            Some("H")
        );
        assert_eq!(c_string_from_utf32(&[0x00]).as_deref(), Some(""));
        assert_eq!(c_string_from_utf32(&[]).as_deref(), Some(""));
        // A surrogate or out-of-range value drops the item, even after a 0.
        assert_eq!(c_string_from_utf32(&[0x48, 0xD800]), None);
        assert_eq!(c_string_from_utf32(&[0x48, 0x00, 0x110000]), None);
        // Noncharacters are valid scalars and pass through.
        assert_eq!(c_string_from_utf32(&[0xFFFE]).as_deref(), Some("\u{FFFE}"));
    }

    #[test]
    fn angle_folds_page_rotation_and_wraps() {
        use std::f32::consts::PI;
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(close(normalize_angle(0.0, 0), 0.0));
        assert!(close(normalize_angle(PI / 2.0, 0), PI / 2.0));
        // /Rotate 90: pdfium's angle minus 3π/2, wrapped up into [0, 2π).
        assert!(close(normalize_angle(0.0, 1), PI / 2.0));
        assert!(close(normalize_angle(0.0, 2), PI));
        assert!(close(normalize_angle(0.0, 3), 3.0 * PI / 2.0));
        assert!(close(normalize_angle(-0.5, 0), 2.0 * PI - 0.5));
        assert!(close(normalize_angle(2.0 * PI + 0.25, 0), 0.25));
    }

    #[test]
    fn argb_packing_matches_fpdf_argb() {
        let color = pdfium::Color {
            r: 0x12,
            g: 0x34,
            b: 0x56,
            a: 0xFF,
        };
        assert_eq!(pack_argb(color), 0xFF12_3456);
    }
}
