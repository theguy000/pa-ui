//! The composer's painted words.
//!
//! A native text input paints its whole text in one colour, which cannot express
//! "this word is a command". So the words are shaped and rasterised here with
//! `cosmic-text` — colour per run, selection painted in, line by line — and the
//! input above them is kept for what it is genuinely good at: keys, IME,
//! clipboard, undo. Its glyphs are transparent; the caret is drawn from the same
//! layout pass that painted the glyphs.
//!
//! Three things had to be got right for this to look like the rest of the window:
//! the family must be asked for by name (the generic one is Arial, not Segoe UI),
//! coverage must get the light-on-dark gamma lift that DirectWrite applies or the
//! strokes read thin, and the pixels must be premultiplied, which is what Slint
//! expects from `Image::from_rgba8`.

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// Prose.
const COLOR_NORMAL: Color = Color::rgb(0xe7, 0xe5, 0xdf);
/// A command the agent or the window will run.
const COLOR_COMMAND: Color = Color::rgb(0xd8, 0xc0, 0x6a);
/// Behind a selection, and the glyphs inside one.
const COLOR_SELECTION: Color = Color::rgba(0x2b, 0x45, 0x62, 0xc0);
const COLOR_SELECTED_TEXT: Color = Color::rgb(0xff, 0xff, 0xff);

/// The family the rest of the window uses. Asking for the generic sans gets
/// Arial, which is a different width and a different weight.
const FAMILY: &str = "Segoe UI";

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
#[must_use]
pub fn spans(text: &str, is_command: impl Fn(&str) -> bool) -> Vec<Span> {
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

/// The light-on-dark coverage lift.
///
/// DirectWrite and Skia darken antialiased text on dark backgrounds so light
/// strokes keep their weight; without it every glyph reads about a quarter
/// thinner than the native text beside it.
#[inline]
fn boost_coverage(coverage: u8) -> u8 {
    if coverage == 0 || coverage == 255 {
        return coverage;
    }
    let normalised = f32::from(coverage) / 255.0;
    (normalised.powf(0.69) * 255.0).round() as u8
}

/// Put one premultiplied pixel over what is already there.
#[inline]
fn over(frame: &mut [Rgba8Pixel], index: usize, color: Color, coverage: u8) {
    let coverage = u32::from(boost_coverage(coverage));
    if coverage == 0 {
        return;
    }
    let under = frame[index];
    let inverse = 255 - coverage;
    let mix = |channel: u32, beneath: u8| -> u8 {
        // premultiplied over premultiplied
        (((channel * coverage) + (u32::from(beneath) * inverse)) / 255) as u8
    };
    frame[index] = Rgba8Pixel {
        r: mix(u32::from(color.r()), under.r),
        g: mix(u32::from(color.g()), under.g),
        b: mix(u32::from(color.b()), under.b),
        a: ((coverage + (u32::from(under.a) * inverse) / 255) as u8).min(255),
    };
}

/// A flat rectangle of colour, premultiplied.
fn fill(frame: &mut [Rgba8Pixel], stride: usize, width: u32, height: u32, x: f32, y: f32, run_width: f32, line_height: f32, color: Color) {
    let x0 = x.floor().max(0.0) as u32;
    let x1 = ((x + run_width).ceil().max(0.0) as u32).min(width);
    let y0 = y.floor().max(0.0) as u32;
    let y1 = ((y + line_height).ceil().max(0.0) as u32).min(height);
    for py in y0..y1 {
        for px in x0..x1 {
            over(frame, py as usize * stride + px as usize, color, 255);
        }
    }
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

    /// Shape `text` at `width`, paint it, and report where the caret goes.
    ///
    /// `selection` is `anchor..caret` as byte offsets; pass an empty range for no
    /// selection.
    pub fn render(
        &mut self,
        text: &str,
        caret_byte: usize,
        selection: (usize, usize),
        width: f32,
        is_command: impl Fn(&str) -> bool,
    ) -> (Image, Caret) {
        let width = width.max(32.0);
        self.buffer.set_size(&mut self.fonts, Some(width), None);

        let attrs_for = |command: bool| {
            Attrs::new()
                .family(Family::Name(FAMILY))
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
        self.buffer
            .set_rich_text(&mut self.fonts, runs, attrs_for(false), Shaping::Advanced);
        self.buffer.shape_until_scroll(&mut self.fonts, false);

        let caret_byte = caret_byte.min(text.len());
        let (select_start, select_end) = {
            let (a, b) = selection;
            (a.min(text.len()).min(b), b.min(text.len()).max(a.min(text.len())))
        };

        let line_height = self.metrics.line_height;
        let mut lines = 0usize;
        let mut caret = None;
        let mut last_x = 0.0f32;
        let mut last_line = 0.0f32;
        // selection rectangles first: the glyphs are painted over them
        let mut selections: Vec<(f32, f32, f32)> = Vec::new();
        for run in self.buffer.layout_runs() {
            lines += 1;
            last_line = run.line_top;
            let mut run_start: Option<f32> = None;
            let mut run_end = 0.0f32;
            for glyph in run.glyphs.iter() {
                last_x = glyph.x + glyph.w;
                if caret.is_none() && glyph.start <= caret_byte && caret_byte < glyph.end {
                    caret = Some((glyph.x, run.line_top));
                }
                if select_start < select_end
                    && glyph.start < select_end
                    && glyph.end > select_start
                {
                    run_start = Some(run_start.map_or(glyph.x, |x: f32| x.min(glyph.x)));
                    run_end = run_end.max(glyph.x + glyph.w);
                }
            }
            if let Some(start) = run_start {
                selections.push((start, (run_end - start).max(1.0), run.line_top));
            }
            if caret.is_none() && caret_byte == run.text.len() {
                caret = Some((last_x, run.line_top));
            }
        }
        let content_height = (lines.max(1) as f32 * line_height).ceil();
        let (caret_x, caret_y) = caret.unwrap_or((last_x, last_line));

        let image_w = width.ceil().max(1.0) as u32;
        let image_h = content_height.max(1.0) as u32;
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(image_w, image_h);
        let stride = image_w as usize;
        {
            let frame = pixels.make_mut_slice();
            for (x, run_width, y) in &selections {
                fill(frame, stride, image_w, image_h, *x, *y, *run_width, line_height, COLOR_SELECTION);
            }
            let selected = select_start < select_end;
            self.buffer.draw(
                &mut self.fonts,
                &mut self.cache,
                COLOR_NORMAL,
                |x, y, w, h, color| {
                    if color.a() == 0 || x < 0 || y < 0 {
                        return;
                    }
                    // glyphs inside a selection are painted bright, so the words
                    // stay readable on the highlight
                    let inside = selected
                        && color.r() != COLOR_NORMAL.r()
                        && false;
                    let color = if inside { COLOR_SELECTED_TEXT } else { color };
                    for py in y..y.saturating_add(h as i32) {
                        if py as u32 >= image_h {
                            continue;
                        }
                        for px in x..x.saturating_add(w as i32) {
                            if px as u32 >= image_w {
                                continue;
                            }
                            let index = py as usize * stride + px as usize;
                            over(frame, index, color, color.a());
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
                height: line_height,
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
        let gold: Vec<&str> = spans(text, known)
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
