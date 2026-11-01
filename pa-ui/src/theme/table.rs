//! The window's colour vocabulary.
//!
//! Every colour the window draws is a role: a name a component asks for, and a
//! value this table decides. A role resolves from the palette under it, so a
//! theme that sets a handful of base values moves the window, and a theme that
//! wants one place to differ names that role and nothing else.
//!
//! A colour value belongs in this file, or in a theme the reader wrote. A
//! component asks for a role.

/// One colour: eight bits a channel, plus the alpha a veil needs.
///
/// Resolution works in these, so the theme can be resolved and mixed in a test
/// with no window open. `to_slint` is the only step that knows Slint exists.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    /// A colour written the way a theme file writes one: `#rrggbb`, or
    /// `#rrggbbaa` when it carries its own alpha.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        let byte = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
        Some(match digits.len() {
            6 => Self { r: byte(0)?, g: byte(2)?, b: byte(4)?, a: 255 },
            8 => Self { r: byte(0)?, g: byte(2)?, b: byte(4)?, a: byte(6)? },
            _ => return None,
        })
    }

    /// A literal written in this file. A bad one is a mistake in this crate, not
    /// in a theme a reader wrote, so it panics where a theme file returns.
    pub(crate) fn hex(text: &str) -> Self {
        Self::parse(text).expect("the table writes well-formed colours")
    }

    /// The same colour at another alpha.
    pub(crate) fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// This colour laid over `behind`, which must be opaque: what a veil comes
    /// out as, and what contrast is measured against.
    pub(crate) fn over(self, behind: Rgba) -> Rgba {
        let weight = u32::from(self.a);
        let rest = 255 - weight;
        let blend = |over: u8, under: u8| {
            ((u32::from(over) * weight + u32::from(under) * rest + 127) / 255) as u8
        };
        Rgba { r: blend(self.r, behind.r), g: blend(self.g, behind.g), b: blend(self.b, behind.b), a: 255 }
    }

    /// The colour the renderer draws: same channels, alpha included.
    pub(crate) fn to_slint(self) -> slint::Color {
        slint::Color::from_argb_u8(self.a, self.r, self.g, self.b)
    }

    /// `share` of this colour and the rest of `other`, both kept opaque.
    pub(crate) fn mix(self, other: Rgba, share: f32) -> Rgba {
        let lerp = |a: u8, b: u8| {
            (f32::from(a) * share + f32::from(b) * (1.0 - share)).round() as u8
        };
        Rgba {
            r: lerp(self.r, other.r),
            g: lerp(self.g, other.g),
            b: lerp(self.b, other.b),
            a: lerp(self.a, other.a),
        }
    }
}

/// The base values a theme sets: the only colours everything else is made of.
///
/// Each ships as the value the window already draws with, so the default theme
/// is today's window rather than a second design. A base value joins this list
/// when a role needs it, which is why the list is short: every one of these
/// moves something the reader can see.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Palette {
    /// The window's own background, under everything: the sidebars, the
    /// caption, the column the transcript sits in.
    Background,
    /// Body ink: the words the reader came for.
    Ink,
    /// Ink that steps back: secondary labels, a resting glyph.
    QuietInk,
    /// A stop, a failure, a delete.
    Danger,
}

impl Palette {
    pub(crate) const ALL: [Palette; 4] =
        [Palette::Background, Palette::Ink, Palette::QuietInk, Palette::Danger];

    /// The name a theme file uses.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Palette::Background => "background",
            Palette::Ink => "ink",
            Palette::QuietInk => "quiet-ink",
            Palette::Danger => "danger",
        }
    }

    /// The value the window ships with: one of the colours it drew before the
    /// theme existed, unchanged.
    fn shipped(self) -> Rgba {
        Rgba::hex(match self {
            Palette::Background => "#111111",
            Palette::Ink => "#e5e5e5",
            Palette::QuietInk => "#b0b0b0",
            Palette::Danger => "#c42b1c",
        })
    }
}

/// Where a role's value comes from when a theme file does not name it.
pub(crate) enum Derivation {
    /// The palette entry itself.
    Palette(Palette),
    /// The palette entry at an alpha, drawn over what is behind it: a hover
    /// chip, which has to let the surface through.
    Veil(Palette, u8),
    /// `share` of the first palette entry and the rest of the second, worked out
    /// when the theme resolves. A mark made this way is one opaque colour, so it
    /// draws exactly as the value it comes to while still moving with the
    /// palette it is mixed from.
    Mix(Palette, Palette, f32),
    /// A value the palette has no word for, kept because the window drew it.
    Fixed(Rgba),
}

/// One colour the window draws, named for the part it paints.
///
/// The names are the theme global's property names and the keys a theme file
/// overrides, so this list is the window's colour glossary: the test in
/// `tests.rs` holds it and `ui/theme.slint` to the same set.
// Every variant is prefixed with the part of the window it paints, because that
// prefix is the role's name in a theme file. The caption is simply the only part
// the table has reached so far.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Role {
    /// The minimise, maximise and close marks at rest: ink stepped back a
    /// little, so a row of three does not shout from the corner.
    TitleBarGlyph,
    /// A caption button with the pointer on it: a faint veil of ink over the
    /// window behind it, the chip the native caption draws.
    TitleBarCaptionFillHover,
    /// The close button with the pointer on it. The only caption that warns.
    TitleBarCloseFillHover,
    /// The close mark on that fill, in white: the one mark the window draws at
    /// full strength, because a warning fill is where it has to be found.
    TitleBarCloseInkHover,
    /// The right column's fold, with the pointer on it.
    TitleBarFoldFillHover,
    /// The fold mark at rest: quiet ink stepped back further, because it is
    /// chrome nobody is using.
    TitleBarFoldGlyph,
    /// The fold mark once the pointer arrives and the column says what it does.
    TitleBarFoldGlyphHover,
}

impl Role {
    pub(crate) const ALL: [Role; 7] = [
        Role::TitleBarGlyph,
        Role::TitleBarCaptionFillHover,
        Role::TitleBarCloseFillHover,
        Role::TitleBarCloseInkHover,
        Role::TitleBarFoldFillHover,
        Role::TitleBarFoldGlyph,
        Role::TitleBarFoldGlyphHover,
    ];

    /// The name a theme file and the theme global both use.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Role::TitleBarGlyph => "title-bar-glyph",
            Role::TitleBarCaptionFillHover => "title-bar-caption-fill-hover",
            Role::TitleBarCloseFillHover => "title-bar-close-fill-hover",
            Role::TitleBarCloseInkHover => "title-bar-close-ink-hover",
            Role::TitleBarFoldFillHover => "title-bar-fold-fill-hover",
            Role::TitleBarFoldGlyph => "title-bar-fold-glyph",
            Role::TitleBarFoldGlyphHover => "title-bar-fold-glyph-hover",
        }
    }

    /// The alphas and shares below are the ones that land on the pixel the
    /// window drew before there was a theme: the marks at `#d7d7d7` and
    /// `#6f6f6f`, the chips at `#222222` and `#2b2b2b` over the background.
    fn derivation(self) -> Derivation {
        match self {
            Role::TitleBarGlyph => Derivation::Mix(Palette::Ink, Palette::Background, 0.933),
            Role::TitleBarCaptionFillHover => Derivation::Veil(Palette::Ink, 0x14),
            Role::TitleBarCloseFillHover => Derivation::Palette(Palette::Danger),
            Role::TitleBarCloseInkHover => Derivation::Fixed(Rgba::hex("#ffffff")),
            Role::TitleBarFoldFillHover => Derivation::Veil(Palette::Ink, 0x1f),
            Role::TitleBarFoldGlyph => Derivation::Mix(Palette::QuietInk, Palette::Background, 0.592),
            Role::TitleBarFoldGlyphHover => Derivation::Palette(Palette::Ink),
        }
    }
}

/// What one theme file changes: the base values it sets, and the roles it names
/// outright. Sparse in both directions — what a file leaves out keeps the value
/// the window shipped with.
#[derive(Default)]
pub(crate) struct Overlay {
    palette: Vec<(Palette, Rgba)>,
    roles: Vec<(Role, Rgba)>,
}

impl Overlay {
    /// A base value, from the file's `palette` block.
    pub(crate) fn set_palette(&mut self, key: Palette, value: Rgba) {
        self.palette.push((key, value));
    }

    /// One role named outright, from the file's `roles` block. It beats
    /// whatever the palette would have derived for the same role.
    pub(crate) fn set_role(&mut self, role: Role, value: Rgba) {
        self.roles.push((role, value));
    }
}

/// Every role resolved: a complete map, with nothing left to look up.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Resolved {
    palette: [Rgba; Palette::ALL.len()],
    roles: [Rgba; Role::ALL.len()],
}

impl Resolved {
    /// The theme compiled into the window. There is no file behind it, so there
    /// is nothing to fail: a selection that cannot be read leaves exactly this.
    pub(crate) fn shipped() -> Self {
        Self::with_overlay(&Overlay::default())
    }

    /// The shipped theme with one file's values on top.
    ///
    /// The whole patch arrives at once and is applied in one pass — palette,
    /// then every derivation, then the roles the file named — so a file can
    /// never leave half of itself behind.
    pub(crate) fn with_overlay(patch: &Overlay) -> Self {
        let blank = Rgba { r: 0, g: 0, b: 0, a: 255 };
        let mut palette = [blank; Palette::ALL.len()];
        for key in Palette::ALL {
            palette[key as usize] = key.shipped();
        }
        for &(key, value) in &patch.palette {
            palette[key as usize] = value;
        }

        let mut roles = [blank; Role::ALL.len()];
        for role in Role::ALL {
            roles[role as usize] = derive(&palette, role);
        }
        // An override always wins over the derivation it would otherwise have.
        for &(role, value) in &patch.roles {
            roles[role as usize] = value;
        }
        Self { palette, roles }
    }

    /// One role's colour.
    pub(crate) fn get(&self, role: Role) -> Rgba {
        self.roles[role as usize]
    }

    /// The window's own background: what everything is drawn on, and what a part
    /// that draws no field of its own — the caption — is read against. Nothing
    /// sits behind the window, so a background carrying an alpha is read as if
    /// it were solid.
    pub(crate) fn background(&self) -> Rgba {
        self.palette[Palette::Background as usize].over(Rgba { r: 0, g: 0, b: 0, a: 255 })
    }
}

fn derive(palette: &[Rgba; Palette::ALL.len()], role: Role) -> Rgba {
    match role.derivation() {
        Derivation::Palette(key) => palette[key as usize],
        Derivation::Veil(key, alpha) => palette[key as usize].with_alpha(alpha),
        Derivation::Mix(from, to, share) => palette[from as usize].mix(palette[to as usize], share),
        Derivation::Fixed(value) => value,
    }
}
