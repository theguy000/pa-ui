//! The composer's rich-text canvas.
//!
//! A native text input paints its whole text in one colour, which cannot express
//! "this word is a command". So the words are shaped and rasterised here with
//! `cosmic-text` — colour per span — and the input above them is kept for what it
//! is genuinely good at: keys, IME, clipboard, selection, undo. Its glyphs are
//! drawn transparent and its caret is width zero, so what the eye sees is this
//! canvas and the caret this module places.
//!
//! Both come out of the same `layout_runs()` pass, so the caret sits on the glyph
//! it belongs to by construction rather than by matching two layouts by eye.

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// Prose.
const COLOR_NORMAL: Color = Color::rgb(0xe7, 0xe5, 0xdf);
/// A command the agent or the window will run.
const COLOR_COMMAND: Color = Color::rgb(0xd8, 0xc0, 0x6a);

/// Where the caret goes, in the canvas' own pixels.
pub struct Caret {
    pub x: f32,
    pub y: f32,
    pub height: f32,
    pub content_height: f32,
}

/// One coloured run of the message.
#[derive(Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub command: bool,
}

/// Split `text` into coloured runs: a whitespace-delimited word that starts with
/// `/` and names a command is a command run, everything else is prose, and prose
/// runs are merged so the text is covered without gaps.
///
/// `is_command` is asked about the bare name, without the slash.
#[must_use]
pub fn spans(text: &str, is_command: impl Fn(&str) -> bool) -> Vec<Span> {
    // every whitespace-delimited word, in order
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut open: Option<usize> = None;
    for (index, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if let Some(start) = open.take() {
                words.push((start, index));
            }
        } else if open.is_none() {
            open = Some(index);
        }
    }
    if let Some(start) = open {
        words.push((start, text.len()));
    }

    let mut out: Vec<Span> = Vec::new();
    let mut prose_start = 0usize;
    for (start, end) in words {
        let word = text.get(start..end).unwrap_or_default();
        let command = word
            .strip_prefix('/')
            .is_some_and(|name| !name.is_empty() && is_command(name));
        if !command {
            continue;
        }
        if start > prose_start {
            out.push(Span {
                start: prose_start,
                end: start,
                command: false,
            });
        }
        out.push(Span {
            start,
            end,
            command: true,
        });
        prose_start = end;
    }
    if prose_start < text.len() || out.is_empty() {
        out.push(Span {
            start: prose_start,
            end: text.len(),
            command: false,
        });
    }
    out
}

/// The shaped, coloured message.
pub struct Canvas {
    fonts: FontSystem,
    cache: SwashCache,
    buffer: Buffer,
    metrics: Metrics,
}

impl Canvas {
    #[must_use]
    pub fn new(font_size: f32, line_height: f32) -> Self {
        let mut fonts = FontSystem::new();
        let metrics = Metrics::new(font_size, line_height);
        let mut buffer = Buffer::new(&mut fonts, metrics);
        buffer.set_wrap(&mut fonts, Wrap::WordOrGlyph);
        Self {
            fonts,
            cache: SwashCache::new(),
            buffer,
            metrics,
        }
    }

    /// Shape `text` at `width` and paint it, returning the picture and the caret
    /// for `caret_byte`.
    pub fn render(
        &mut self,
        text: &str,
        caret_byte: usize,
        width: f32,
        is_command: impl Fn(&str) -> bool,
    ) -> (Image, Caret) {
        let width = width.max(32.0);
        self.buffer
            .set_size(&mut self.fonts, Some(width), None);

        let attrs_for = |command: bool| {
            Attrs::new()
                .family(Family::SansSerif)
                .color(if command { COLOR_COMMAND } else { COLOR_NORMAL })
        };
        let runs: Vec<(&str, Attrs)> = spans(text, is_command)
            .into_iter()
            .map(|span| {
                (
                    text.get(span.start..span.end).unwrap_or_default(),
                    attrs_for(span.command),
                )
            })
            .collect();
        self.buffer.set_rich_text(
            &mut self.fonts,
            runs,
            attrs_for(false),
            Shaping::Advanced,
        );
        self.buffer.shape_until_scroll(&mut self.fonts, false);

        let caret_byte = caret_byte.min(text.len());
        let mut lines = 0usize;
        let mut caret = None;
        let mut last_x = 0.0f32;
        let mut last_line = 0.0f32;
        for run in self.buffer.layout_runs() {
            lines += 1;
            last_line = run.line_top;
            for glyph in run.glyphs.iter() {
                last_x = glyph.x + glyph.w;
                if caret.is_none() && glyph.start <= caret_byte && caret_byte < glyph.end {
                    caret = Some((glyph.x, run.line_top));
                }
            }
            if caret.is_none() && caret_byte == run.text.len() {
                caret = Some((last_x, run.line_top));
            }
        }
        let content_height = (lines.max(1) as f32 * self.metrics.line_height).ceil();
        let (caret_x, caret_y) = caret.unwrap_or((last_x, last_line));

        let image_w = width.ceil().max(1.0) as u32;
        let image_h = content_height.max(1.0) as u32;
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(image_w, image_h);
        {
            let stride = image_w as usize;
            let frame = pixels.make_mut_slice();
            self.buffer.draw(
                &mut self.fonts,
                &mut self.cache,
                COLOR_NORMAL,
                |x, y, w, h, color| {
                    let alpha = color.a();
                    if alpha == 0 || x < 0 || y < 0 {
                        return;
                    }
                    for py in y..y.saturating_add(h as i32) {
                        if py as u32 >= image_h {
                            continue;
                        }
                        for px in x..x.saturating_add(w as i32) {
                            if px as u32 >= image_w {
                                continue;
                            }
                            let index = py as usize * stride + px as usize;
                            let under = frame[index];
                            let a = u32::from(alpha);
                            let inv = 255 - a;
                            frame[index] = Rgba8Pixel {
                                r: ((u32::from(color.r()) * a
                                    + u32::from(under.r) * inv)
                                    / 255) as u8,
                                g: ((u32::from(color.g()) * a
                                    + u32::from(under.g) * inv)
                                    / 255) as u8,
                                b: ((u32::from(color.b()) * a
                                    + u32::from(under.b) * inv)
                                    / 255) as u8,
                                a: (a + u32::from(under.a) * inv / 255) as u8,
                            };
                        }
                    }
                },
            );
        }

        (
            Image::from_rgba8(pixels),
            Caret {
                x: caret_x,
                y: caret_y,
                height: self.metrics.line_height,
                content_height,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{spans, Span};

    fn known(name: &str) -> bool {
        matches!(name, "refine" | "compact")
    }

    #[test]
    fn a_command_at_the_start_is_one_gold_run() {
        assert_eq!(
            spans("/refine tighten this", known),
            vec![
                Span { start: 0, end: 7, command: true },
                Span { start: 7, end: 20, command: false },
            ]
        );
    }

    #[test]
    fn a_command_between_words_is_found_too() {
        let text = "please /compact the log";
        let found = spans(text, known);
        let gold: Vec<&str> = found
            .iter()
            .filter(|span| span.command)
            .map(|span| &text[span.start..span.end])
            .collect();
        assert_eq!(gold, vec!["/compact"]);
    }

    #[test]
    fn an_unknown_word_after_a_slash_stays_prose() {
        assert!(spans("/nonsense here", known).iter().all(|span| !span.command));
        assert!(spans("a/b is a path", known).iter().all(|span| !span.command));
    }

    #[test]
    fn the_runs_cover_the_text_without_gaps() {
        let text = "  /refine   the  notes /compact ";
        let found = spans(text, known);
        assert_eq!(found.first().map(|span| span.start), Some(0));
        assert_eq!(found.last().map(|span| span.end), Some(text.len()));
        for pair in found.windows(2) {
            assert_eq!(pair[0].end, pair[1].start, "a gap between {pair:?}");
        }
    }
}

#[cfg(test)]
mod canvas_probe {
    use super::*;

    /// Probe: does the canvas actually contain glyphs, and are the command ones
    /// gold? Writes a PPM-free raw dump the caller can inspect.
    #[test]
    #[ignore = "writes a file"]
    fn painted_pixels() {
        let mut canvas = Canvas::new(13.0, 18.0);
        let (image, caret) = canvas.render(
            "hello /refine tighten this",
            8,
            400.0,
            |name| name == "refine",
        );
        let buffer = image.to_rgba8().expect("rgba8");
        let (w, h) = (buffer.width(), buffer.height());
        let mut opaque = 0usize;
        let mut gold = 0usize;
        let mut bytes = Vec::with_capacity((w * h * 4) as usize);
        for pixel in buffer.as_slice() {
            bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
            if pixel.a > 8 {
                opaque += 1;
                if pixel.r > 150 && pixel.g > 120 && pixel.b < 140 {
                    gold += 1;
                }
            }
        }
        std::fs::write(std::env::temp_dir().join("canvas.raw"), &bytes).expect("write");
        eprintln!("canvas {w}x{h} opaque={opaque} gold={gold} caret=({},{})", caret.x, caret.y);
        assert!(opaque > 50, "the canvas painted almost nothing");
        assert!(gold > 5, "no gold pixels: the command is not coloured");
    }
}
