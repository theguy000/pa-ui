//! Where the window keeps a colour of its own, or names a role the theme never
//! declared.
//!
//! The role table is the one owner of the window's colours only while nothing
//! else can write one down. Three things break that, and all are read as text: a
//! value written where a role belongs, a role name the theme global does not
//! carry, and a hex pasted into the Rust that draws for itself. The suite runs
//! them over `ui/` and `src/`, so a contributor finds out in the run they already
//! make rather than from a screenshot a month later.
//!
//! What counts as a value: a hex literal (`#fff`, `#1d1d1d`, `#00000080`), a
//! colour built from numbers (`rgb(…)` and the three functions beside it), and a
//! colour name — a value spelled out in letters. Four things are not values:
//! `transparent`, which draws nothing and is how a component says there is no
//! field here; a reference to another component's property, which is a name
//! rather than a colour; anything inside a comment, which is a word about the
//! window; and, in a component, anything inside a label.
//!
//! The parsing here is only as much as these sources need: `//` to the end of the
//! line, and double-quoted labels that open and close on one line. There is no
//! block comment in `ui/` or `src/`.

/// Whether a scan wants the labels a line carries.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Labels {
    /// Emptied to blanks: a component's label is a word about the window, and a
    /// `#` in one is not a colour it draws.
    Blank,
    /// Read as written: a Rust string is exactly where a pasted hex would sit.
    Keep,
}

/// A source that broke the contract, and where a contributor has to look.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Break {
    /// The source, as the build names it: `ui/app.slint`, `src/markup.rs`.
    pub(crate) file: String,
    /// The line the value or the name is on, counting from one, the way an
    /// editor does.
    pub(crate) line: usize,
    /// What is written there: the literal, or the `Theme.<name>` that resolves to
    /// nothing.
    pub(crate) written: String,
}

impl Break {
    /// A break at one place in one source.
    pub(crate) fn at(file: &str, line: usize, written: impl Into<String>) -> Self {
        Self { file: file.to_string(), line, written: written.into() }
    }

    /// Where to look and what is written there, in the shape a compiler prints a
    /// diagnostic in, so a reader can jump straight to it:
    /// `ui/app.slint:83: #111111`.
    pub(crate) fn describe(&self) -> String {
        format!("{}:{}: {}", self.file, self.line, self.written)
    }
}

/// Every colour value written in the source of `file`.
///
/// One entry per value, so a line that pastes two of them is two places to fix.
pub(crate) fn colour_values(file: &str, source: &str) -> Vec<Break> {
    let mut found = Vec::new();
    for (line, text) in code_lines(source, Labels::Blank) {
        let mut at = 0;
        while let Some(c) = text[at..].chars().next() {
            let width = c.len_utf8();
            if c == '#' {
                if let Some(hex) = hex_value(&text[at..]) {
                    found.push(Break::at(file, line, hex));
                    at += hex.len();
                    continue;
                }
            } else if is_ident(c) {
                let word: String = text[at..].chars().take_while(|c| is_ident(*c)).collect();
                let called = opens_call(&text[at + word.len()..]);
                if is_colour_name(&word) || (is_colour_function(&word) && called) {
                    found.push(Break::at(file, line, word.clone()));
                }
                at += word.len();
                continue;
            }
            at += width;
        }
    }
    found
}

/// Every `Theme.<name>` in the source of `file` that `declared` does not carry.
///
/// A name a component uses and the global does not declare is a rename that
/// half-landed: the component goes on asking for a colour nothing answers to.
pub(crate) fn undeclared_roles(file: &str, source: &str, declared: &[&str]) -> Vec<Break> {
    let mut found = Vec::new();
    for (line, text) in code_lines(source, Labels::Blank) {
        let mut from = 0;
        while let Some(offset) = text[from..].find("Theme.") {
            let start = from + offset + "Theme.".len();
            let name: String = text[start..].chars().take_while(|c| is_ident(*c)).collect();
            if !name.is_empty() && !declared.contains(&name.as_str()) {
                found.push(Break::at(file, line, format!("Theme.{name}")));
            }
            // Past this name, past the dot even when nothing followed it, so the
            // walk always moves on.
            from = start + name.len();
        }
    }
    found
}

/// Every hex colour written in the source of a Rust file of the window.
///
/// Two parts draw their own colours in Rust — the transcript's markup and the
/// composer's canvas — and both read the theme now, so a hex here is a colour the
/// theme cannot reach in exactly the way one in a component is. What a text scan
/// cannot see is a colour built from channels by hand, `Color::from_rgb_u8(17, 17,
/// 17)`; the hex is the shape this crate writes a value in, and `Rgba` in the
/// table is the only thing that reads one.
pub(crate) fn rust_values(file: &str, source: &str) -> Vec<Break> {
    let mut found = Vec::new();
    for (line, text) in code_lines(source, Labels::Keep) {
        let mut from = 0;
        // The `"` before the `#` is what makes it a string rather than a number
        // read off something else, and the `"` after it is what ends the label.
        while let Some(offset) = text[from..].find("\"#") {
            let start = from + offset + 1;
            if let Some(hex) = hex_value(&text[start..]) {
                if text[start + hex.len()..].starts_with('"') {
                    found.push(Break::at(file, line, hex));
                    from = start + hex.len();
                    continue;
                }
            }
            from = start;
        }
    }
    found
}

/// The roles the theme global declares, in the order the file declares them.
///
/// The global is the seam code generation cannot close: a role is written in the
/// table and again in `ui/theme.slint`, so both sides are read as text and held
/// to the same set. A role with no setter is not this file's business — the bridge
/// matches every role, so one cannot exist without one.
pub(crate) fn declared_roles(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter_map(|line| {
            // Read as `<visibility> property <color> <name>` and nothing else, so
            // a comment after the declaration, a default value or a different
            // qualifier neither hides a role nor invents one.
            let declaration = line.split("//").next().unwrap_or_default();
            let mut words = declaration.split_whitespace();
            if !matches!(words.next()?, "in" | "out" | "in-out") {
                return None;
            }
            if words.next()? != "property" || words.next()? != "<color>" {
                return None;
            }
            let name = words.next()?;
            let end = name.find([';', ':']).unwrap_or(name.len());
            Some(&name[..end])
        })
        .collect()
}

/// Each line of `source`, numbered from one, as a scan reads it.
fn code_lines(
    source: &str,
    labels: Labels,
) -> impl Iterator<Item = (usize, String)> + '_ {
    source.lines().enumerate().map(move |(index, line)| (index + 1, code(line, labels)))
}

/// One line with its comment gone, and its labels handled as `labels` asks.
///
/// Labels are kept or blanked rather than cut away because a label is not the end
/// of the line: `text: "x"; color: #fff;` writes a value after one.
fn code(line: &str, labels: Labels) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_label = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if in_label {
            // An escape would otherwise end the label early, and its second
            // character must not be read as code either.
            if c == '\\' {
                out.push(' ');
                if let Some(escaped) = chars.next() {
                    out.push(if labels == Labels::Keep { escaped } else { ' ' });
                }
                continue;
            }
            if c == '"' {
                in_label = false;
            }
            out.push(if labels == Labels::Keep { c } else { ' ' });
            continue;
        }
        match c {
            '"' => {
                in_label = true;
                out.push('"');
            }
            '/' if chars.clone().next() == Some('/') => break,
            _ => out.push(c),
        }
    }
    out
}

/// A hex colour written at the start of `text`, if one is: the literal as it was
/// written, so a report shows the contributor their own spelling.
///
/// Three, six and eight digits are the lengths Slint reads as a colour, so they
/// are the lengths a source can hold.
fn hex_value(text: &str) -> Option<&str> {
    let digits = text[1..].chars().take_while(|c| c.is_ascii_hexdigit()).count();
    if !matches!(digits, 3 | 6 | 8) {
        return None;
    }
    let rest = &text[1 + digits..];
    rest.chars().next().is_none_or(|c| !is_ident(c)).then(|| &text[..1 + digits])
}

/// Whether `c` can be part of a Slint or Rust name.
///
/// A hyphen is a name character here, so a role called `gold-ink` is one word and
/// not a colour name with something after it.
fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// Whether a word stands for a colour: the names Slint reads as values, which are
/// the CSS ones.
fn is_colour_name(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    COLOUR_NAMES.split(' ').any(|name| name == lower)
}

/// Whether a word opens a colour built from numbers. Exactly the four Slint reads
/// as values: a call it cannot parse is a build error rather than a colour.
fn is_colour_function(word: &str) -> bool {
    matches!(word.to_ascii_lowercase().as_str(), "rgb" | "rgba" | "hsv" | "hsva")
}

/// Whether what follows a word is the bracket that makes it a call.
fn opens_call(rest: &str) -> bool {
    rest.trim_start().starts_with('(')
}

/// The colour names Slint accepts, which are the CSS ones.
///
/// `transparent` is not among them on purpose: it is an alpha of zero rather than
/// a colour anyone chose, and the window uses it to mean "no field here".
const COLOUR_NAMES: &str = "aliceblue antiquewhite aqua aquamarine azure beige bisque \
blanchedalmond blue blueviolet brown burlywood cadetblue chartreuse chocolate coral \
cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgreen \
darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon \
darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink \
deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro \
ghostwhite gold goldenrod gray green greenyellow grey honeydew hotpink indianred indigo \
ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan \
lightgoldenrodyellow lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen \
lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime limegreen linen \
magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen \
mediumslateblue mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream \
mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid \
palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum \
powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown \
seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen \
steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow yellowgreen";
