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
//! strokes read thin, and the buffer is premultiplied — `Image::from_rgba8` takes
//! straight alpha and would square the alpha if handed premultiplied pixels.

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

use crate::theme::{Resolved, Role};

/// The family the rest of the window uses. Asking for the generic sans gets
/// Arial, which is a different width and a different weight.
const FAMILY: &str = "Segoe UI";

/// A role's colour as `cosmic-text` spells it.
///
/// The canvas is the one consumer that cannot be handed a `slint::Color`: it
/// rasterises its own pixels, with its own font stack, and takes its colours as
/// four channels.
fn colour(theme: &Resolved, role: Role) -> Color {
    let value = theme.get(role);
    Color::rgba(value.r, value.g, value.b, value.a)
}

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
/// `((i / 255)^0.55 × 255)` for every coverage value, computed once.
///
/// 0.55 is the curve DirectWrite uses at 96 DPI for light-on-dark text; calling
/// `powf` per pixel of every glyph was the most expensive thing in a keystroke.
static BOOST_LUT: std::sync::LazyLock<[u8; 256]> = std::sync::LazyLock::new(|| {
    let mut table = [0u8; 256];
    for (index, slot) in table.iter_mut().enumerate() {
        let normalised = index as f32 / 255.0;
        *slot = (normalised.powf(0.55) * 255.0).round() as u8;
    }
    table
});

#[inline]
fn boost_coverage(coverage: u8) -> u8 {
    BOOST_LUT[coverage as usize]
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
        a: ((coverage + (u32::from(under.a) * inverse) / 255) as u8),
    };
}

/// A rectangle in the canvas' own pixels.
struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

/// A flat rectangle of colour, premultiplied.
fn fill(frame: &mut [Rgba8Pixel], stride: usize, width: u32, height: u32, rect: Rect, color: Color) {
    let x0 = rect.x.floor().max(0.0) as u32;
    let x1 = ((rect.x + rect.width).ceil().max(0.0) as u32).min(width);
    let y0 = rect.y.floor().max(0.0) as u32;
    let y1 = ((rect.y + rect.height).ceil().max(0.0) as u32).min(height);
    for py in y0..y1 {
        for px in x0..x1 {
            over(frame, py as usize * stride + px as usize, color, 255);
        }
    }
}

/// A whole-document byte offset, as the line it falls on and the offset inside
/// that line.
///
/// cosmic-text numbers a line's text, and every glyph in it, from the start of
/// that line - a document-wide offset can never be compared with a glyph's.
fn split_at_line(text: &str, byte: usize) -> (usize, usize) {
    // the caret can arrive one keystroke behind the text it belongs to, and a
    // half-a-character offset cannot be sliced out of a string
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    let before = &text[..byte];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    (line, byte - line_start)
}

/// The shaped, coloured message.
pub struct Canvas {
    fonts: FontSystem,
    cache: SwashCache,
    buffer: Buffer,
    metrics: Metrics,
    /// The draft's own words, and the one word in it that names a command the
    /// agent or the window will run.
    normal: Color,
    command: Color,
    /// Behind a selection. Laid down opaque, so the words inside it keep the
    /// draft's own ink.
    selection: Color,
}

impl Canvas {
    /// Build the painter in a theme's colours: they are asked for once, here, so
    /// a pass per keystroke never resolves a role.
    #[must_use]
    pub fn new(font_size: f32, line_height: f32, theme: &Resolved) -> Self {
        let mut fonts = FontSystem::new();
        let metrics = Metrics::new(font_size, line_height);
        let mut buffer = Buffer::new(&mut fonts, metrics);
        buffer.set_wrap(&mut fonts, Wrap::WordOrGlyph);
        Self {
            fonts,
            cache: SwashCache::new(),
            buffer,
            metrics,
            normal: colour(theme, Role::ComposerDraftInk),
            command: colour(theme, Role::ComposerDraftCommandInk),
            selection: colour(theme, Role::ComposerDraftSelectionWash),
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

        // copied out of `self` before the buffer is shaped: the pass below holds
        // the buffer mutably, and a colour is four channels
        let (normal, command, highlight) = (self.normal, self.command, self.selection);
        let attrs_for = |command_run: bool| {
            Attrs::new()
                .family(Family::Name(FAMILY))
                .color(if command_run { command } else { normal })
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
        let (caret_line, caret_index) = split_at_line(text, caret_byte);
        let select_from = split_at_line(text, select_start);
        let select_to = split_at_line(text, select_end);

        let line_height = self.metrics.line_height;
        let mut lines = 0usize;
        let mut caret = None;
        // where the caret goes when it sits past the last glyph of its own line
        let mut caret_at_line_end = None;
        let mut last_x = 0.0f32;
        let mut last_line = 0.0f32;
        // selection rectangles first: the glyphs are painted over them
        let mut selections: Vec<(f32, f32, f32)> = Vec::new();
        for run in self.buffer.layout_runs() {
            lines += 1;
            last_line = run.line_top;
            // the slice of the selection this visual run carries, clamped to
            // the run's own numbering: a selection that spans lines is one
            // rectangle per line, not one rectangle per document
            let selected = if select_start < select_end
                && run.line_i >= select_from.0
                && run.line_i <= select_to.0
            {
                let from = if run.line_i == select_from.0 { select_from.1 } else { 0 };
                let to = if run.line_i == select_to.0 { select_to.1 } else { run.text.len() };
                (from, to.max(from))
            } else {
                (0, 0)
            };
            let mut run_start: Option<f32> = None;
            let mut run_end = 0.0f32;
            for glyph in run.glyphs.iter() {
                last_x = glyph.x + glyph.w;
                if caret.is_none()
                    && run.line_i == caret_line
                    && glyph.start <= caret_index
                    && caret_index < glyph.end
                {
                    caret = Some((glyph.x, run.line_top));
                }
                if selected.0 < selected.1
                    && glyph.start < selected.1
                    && glyph.end > selected.0
                {
                    run_start = Some(run_start.map_or(glyph.x, |x: f32| x.min(glyph.x)));
                    run_end = run_end.max(glyph.x + glyph.w);
                }
            }
            if let Some(start) = run_start {
                selections.push((start, (run_end - start).max(1.0), run.line_top));
            }
            // The caret at the end of a line is past its last glyph, and every
            // wrapped run of that line answers to the same line index: the one
            // seen last - the bottom of the paragraph - is the one it belongs
            // to, so each run overwrites the answer.
            if run.line_i == caret_line {
                caret_at_line_end = Some((last_x, run.line_top));
            }
        }
        let content_height = (lines.max(1) as f32 * line_height).ceil();
        let (caret_x, caret_y) = caret.or(caret_at_line_end).unwrap_or((last_x, last_line));

        let image_w = width.ceil().max(1.0) as u32;
        let image_h = content_height.max(1.0) as u32;
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(image_w, image_h);
        let stride = image_w as usize;
        {
            let frame = pixels.make_mut_slice();
            for (x, run_width, y) in &selections {
                let selected = Rect { x: *x, y: *y, width: *run_width, height: line_height };
                fill(frame, stride, image_w, image_h, selected, highlight);
            }
            self.buffer.draw(
                &mut self.fonts,
                &mut self.cache,
                normal,
                |x, y, w, h, color| {
                    if color.a() == 0 || x < 0 || y < 0 {
                        return;
                    }
                    // the highlight sits under the glyphs; the words keep their
                    // own colour, which reads fine on it
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
            // premultiplied: `from_rgba8` would multiply the channels by alpha a
            // second time and crush every antialiased edge
            Image::from_rgba8_premultiplied(pixels),
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

#[cfg(test)]
mod caret_tests {
    use super::{split_at_line, Canvas};
    use crate::theme::Resolved;

    /// A canvas in the theme the window ships with.
    fn canvas() -> Canvas {
        Canvas::new(13.0, 18.0, &Resolved::shipped())
    }

    #[test]
    fn the_first_line_is_numbered_from_the_top() {
        assert_eq!(split_at_line("abc", 0), (0, 0));
        assert_eq!(split_at_line("abc", 3), (0, 3));
    }

    #[test]
    fn a_newline_restarts_the_count() {
        let text = "abc\ndef";
        assert_eq!(split_at_line(text, 3), (0, 3), "end of the first line");
        assert_eq!(split_at_line(text, 4), (1, 0), "start of the second");
        assert_eq!(split_at_line(text, 7), (1, 3), "end of the second");
    }

    #[test]
    fn empty_lines_are_lines_too() {
        let text = "a\n\nb";
        assert_eq!(split_at_line(text, 2), (1, 0));
        assert_eq!(split_at_line(text, 3), (2, 0));
    }

    #[test]
    fn an_offset_past_the_end_clamps() {
        assert_eq!(split_at_line("ab", 99), (0, 2));
        assert_eq!(split_at_line("", 5), (0, 0));
    }

    #[test]
    fn a_stale_offset_never_lands_inside_a_character() {
        // the "e" with an acute is two bytes and one character: a caret left
        // at byte 1 by an earlier edit must not panic the painter
        let text = "\u{e9}x\ny";
        assert_eq!(split_at_line(text, 1), (0, 0), "snapped back to the character start");
        assert_eq!(split_at_line(text, 2), (0, 2));
        assert_eq!(split_at_line(text, 4), (1, 0), "after the newline");
    }

    /// The caret used to be matched against glyph offsets as if they were
    /// document-wide, so on every line but the first it fell back to the end of
    /// the document: a paste put the caret somewhere the writer never clicked.
    #[test]
    fn a_caret_on_a_later_line_lands_between_its_own_glyphs() {
        let mut canvas = canvas();
        let text = "abc\ndefgh";
        let at = |canvas: &mut Canvas, byte: usize| {
            let (_, caret) = canvas.render(text, byte, (byte, byte), 400.0, |_| false);
            (caret.x, caret.y)
        };
        let (start_of_second, y_start) = at(&mut canvas, 5);
        let (inside_second, y_inside) = at(&mut canvas, 6);
        let (end_of_document, y_end) = at(&mut canvas, 9);
        assert_eq!((y_start, y_inside, y_end), (18.0, 18.0, 18.0), "all three are on the second line");
        assert!(
            start_of_second < inside_second && inside_second < end_of_document,
            "the caret inside the line sits between the line's own glyphs: {start_of_second} < {inside_second} < {end_of_document}"
        );
    }

    /// The same fallback put the caret on the first wrapped run of a paragraph
    /// instead of the last one.
    #[test]
    fn the_end_of_a_wrapped_line_belongs_to_its_bottom_run() {
        let mut canvas = canvas();
        let text = "one two three four five six seven eight nine ten eleven twelve";
        let (_, caret) = canvas.render(text, text.len(), (0, 0), 60.0, |_| false);
        assert!(
            caret.content_height > 18.0,
            "the text has to wrap for this to mean anything"
        );
        assert_eq!(
            caret.y,
            caret.content_height - 18.0,
            "the caret is on the last wrapped run, not the first"
        );
    }
}

#[cfg(test)]
mod theme_tests {
    use super::Canvas;
    use crate::theme::{light, Resolved, Role};

    /// The draft these cases paint: prose, a command word in the middle of it,
    /// and its first word selected.
    const DRAFT: &str = "please /refine this";

    /// What a theme paints for that draft, pixel for pixel, and where the caret
    /// lands: the buffer the canvas made, before Slint sees it.
    fn painted(theme: &Resolved) -> (Vec<u8>, (f32, f32, f32, f32)) {
        let mut canvas = Canvas::new(15.0, 20.0, theme);
        let (image, caret) =
            canvas.render(DRAFT, DRAFT.len(), (0, 6), 400.0, |name| name == "refine");
        let pixels = image
            .to_rgba8_premultiplied()
            .expect("a buffer this canvas made")
            .as_slice()
            .iter()
            .flat_map(|pixel| [pixel.r, pixel.g, pixel.b, pixel.a])
            .collect();
        (pixels, (caret.x, caret.y, caret.height, caret.content_height))
    }

    fn draws(pixels: &[u8], colour: [u8; 4]) -> bool {
        pixels.as_chunks::<4>().0.contains(&colour)
    }

    /// The coverage each pixel was painted at, which is the glyphs' own shape:
    /// alpha comes from the rasteriser and the selection, never from a colour.
    fn coverage(pixels: &[u8]) -> Vec<u8> {
        pixels.as_chunks::<4>().0.iter().map(|pixel| pixel[3]).collect()
    }

    /// The ticket's second promise: the same draft, twice, in two themes. The
    /// words come out in the theme's colours, and every glyph sits exactly where
    /// it sat — a theme that moved the layout would be a redesign, not a scheme.
    #[test]
    fn the_same_draft_under_two_themes_paints_the_same_glyphs_in_other_colours() {
        let (here, caret_here) = painted(&Resolved::shipped());
        let (there, caret_there) = painted(&light());

        assert_ne!(here, there, "the draft has to follow the theme");
        let (here_coverage, there_coverage) = (coverage(&here), coverage(&there));
        assert_eq!(here_coverage, there_coverage, "the glyphs have not moved");
        assert_eq!(caret_here, caret_there, "nor has the caret");

        // And the comparison is worth something: some pixel is a glyph at full
        // strength and some pixel is untouched.
        assert!(here_coverage.contains(&255) && here_coverage.contains(&0), "an empty pass");
    }

    /// The other half of the same promise: under the theme that ships, the canvas
    /// paints the pixels it painted before there was a theme. Read off the buffer
    /// rather than off the canvas, so this is a check of the drawing.
    #[test]
    fn the_shipped_theme_paints_the_draft_the_canvas_painted_before_the_theme() {
        let (pixels, _) = painted(&Resolved::shipped());

        assert!(draws(&pixels, [0xe5, 0xe5, 0xe5, 255]), "the draft's own ink");
        assert!(draws(&pixels, [0xc1, 0xc1, 0xc1, 255]), "the command word among them");
        assert!(draws(&pixels, [0x43, 0x43, 0x43, 255]), "the field under a selection");
    }

    /// And the same draft on a light window: the three colours it asks for are
    /// the three the table resolves, so a theme reaches even what the window
    /// paints for itself.
    #[test]
    fn a_light_theme_paints_the_draft_in_its_own_colours() {
        let light = light();
        let (pixels, _) = painted(&light);

        for role in [
            Role::ComposerDraftInk,
            Role::ComposerDraftCommandInk,
            Role::ComposerDraftSelectionWash,
        ] {
            let colour = light.get(role);
            assert!(
                draws(&pixels, [colour.r, colour.g, colour.b, 255]),
                "{} is not on the canvas: {colour:?}",
                role.name()
            );
        }
    }
}
