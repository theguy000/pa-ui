//! Reading a theme file: the text of one file in, a patch out, or the refusal
//! to log.
//!
//! A theme file is sparse by design — a handful of base values and whatever
//! roles the author wanted to name — so the parser walks the document by hand
//! and refuses anything it does not recognise, saying which file, which name it
//! was given and which name it thinks was meant.

use serde_json::{Map, Value};

use super::{Overlay, Palette, Resolved, Rgba, Role};

/// The schema this window reads. A file that names another one is refused
/// rather than half-read: a later version may mean something else by a name
/// this one already uses.
const VERSION: i64 = 1;

/// Why a theme could not be used.
///
/// The file is part of the message because one line in the log is all the
/// reader ever learns of it, and the window stays as it was.
#[derive(Debug)]
pub(crate) struct Refusal {
    file: String,
    detail: String,
}

impl Refusal {
    fn new(file: &str, detail: impl Into<String>) -> Self {
        Self { file: file.to_string(), detail: detail.into() }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "theme `{}`: {}", self.file, self.detail)
    }
}

/// The theme a file's text resolves to, or the reason it could not be read.
///
/// Pure: the same text always gives the same map, so the whole layer is tested
/// without a window or a file on disk.
pub(crate) fn from_text(file: &str, text: &str) -> Result<Resolved, Refusal> {
    Ok(Resolved::with_overlay(&patch(file, text)?))
}

/// The theme a file resolves to. The one place a path is opened, so the window's
/// selection and a theme author's check read a file the same way.
pub(crate) fn load(path: &str) -> Result<Resolved, Refusal> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| Refusal::new(path, format!("cannot be read ({error})")))?;
    from_text(path, &text)
}

/// The window's theme: the one compiled in, with the file the reader selected
/// laid over it. A selection that cannot be read leaves the shipped theme in
/// place and comes back as the line to log.
pub(crate) fn select(choice: Option<&str>) -> (Resolved, Option<Refusal>) {
    let Some(choice) = choice.filter(|choice| !choice.is_empty()) else {
        return (Resolved::shipped(), None);
    };
    match load(choice) {
        Ok(theme) => (theme, None),
        Err(refusal) => (Resolved::shipped(), Some(refusal)),
    }
}

fn patch(file: &str, text: &str) -> Result<Overlay, Refusal> {
    // A file saved on Windows usually opens with a byte-order mark. It belongs
    // to the file, not to the document, and refusing a theme over it would be a
    // puzzle with no answer in the message.
    let text = text.trim_start_matches('\u{feff}');
    let document: Value =
        serde_json::from_str(text).map_err(|error| Refusal::new(file, error.to_string()))?;
    let Value::Object(fields) = document else {
        return Err(Refusal::new(file, "a theme is one JSON object"));
    };

    for key in fields.keys() {
        if !matches!(key.as_str(), "name" | "version" | "palette" | "roles") {
            return Err(unknown(
                file,
                "key",
                key,
                ["name", "version", "palette", "roles"].into_iter(),
            ));
        }
    }
    match fields.get("name") {
        // The name is the theme's own: it is what a bundled theme will be
        // selected by, and what tells one file from another when a reader has
        // several of them.
        Some(Value::String(name)) if !name.trim().is_empty() => {}
        Some(_) => return Err(Refusal::new(file, "`name` is not a name")),
        None => return Err(Refusal::new(file, "has no `name`: a theme names itself")),
    }
    match fields.get("version") {
        Some(Value::Number(version)) if version.as_i64() == Some(VERSION) => {}
        Some(other) => {
            return Err(Refusal::new(
                file,
                format!("is version {other}; this window reads version {VERSION}"),
            ))
        }
        None => {
            return Err(Refusal::new(
                file,
                format!("has no `version`; this window reads version {VERSION}"),
            ))
        }
    }

    let mut patch = Overlay::default();
    if let Some(block) = fields.get("palette") {
        for (used, value) in entries(file, "palette", block)? {
            let Some(key) = Palette::ALL.iter().find(|key| key.name() == used) else {
                return Err(unknown(
                    file,
                    "palette value",
                    used,
                    Palette::ALL.iter().map(|key| key.name()),
                ));
            };
            patch.set_palette(*key, colour(file, "palette", used, value)?);
        }
    }
    if let Some(block) = fields.get("roles") {
        for (used, value) in entries(file, "roles", block)? {
            let Some(role) = Role::ALL.iter().find(|role| role.name() == used) else {
                return Err(unknown(file, "role", used, Role::ALL.iter().map(|role| role.name())));
            };
            patch.set_role(*role, colour(file, "roles", used, value)?);
        }
    }
    Ok(patch)
}

/// One block of the file: names to colours, or a refusal that says which block
/// was the wrong shape.
fn entries<'a>(
    file: &str,
    block: &str,
    value: &'a Value,
) -> Result<&'a Map<String, Value>, Refusal> {
    match value {
        Value::Object(entries) => Ok(entries),
        _ => Err(Refusal::new(file, format!("`{block}` is not an object of names to colours"))),
    }
}

fn colour(file: &str, block: &str, used: &str, value: &Value) -> Result<Rgba, Refusal> {
    let Some(text) = value.as_str() else {
        return Err(Refusal::new(file, format!("`{block}.{used}` is not a colour string")));
    };
    Rgba::parse(text).ok_or_else(|| {
        Refusal::new(file, format!("`{block}.{used}` is `{text}`; a colour is #rrggbb or #rrggbbaa"))
    })
}

/// A name this window does not have, with the one it probably meant.
fn unknown<'a>(
    file: &str,
    what: &str,
    used: &str,
    legal: impl Iterator<Item = &'a str>,
) -> Refusal {
    match nearest(used, legal) {
        Some(meant) => Refusal::new(file, format!("unknown {what} `{used}`; nearest is `{meant}`")),
        None => Refusal::new(file, format!("unknown {what} `{used}`")),
    }
}

/// The legal name a misspelt one was probably meant to be: fewest edits, with a
/// tie going to the earlier name in the table.
fn nearest<'a>(used: &str, legal: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<(usize, &'a str)> = None;
    for name in legal {
        let distance = edits(used, name);
        let closer = match best {
            Some((closest, _)) => distance < closest,
            None => true,
        };
        if closer {
            best = Some((distance, name));
        }
    }
    best.map(|(_, name)| name)
}

/// How many single-character edits turn one name into the other.
fn edits(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut above: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (row, from) in a.iter().enumerate() {
        current[0] = row + 1;
        for (column, to) in b.iter().enumerate() {
            let substitute = above[column] + usize::from(from != to);
            current[column + 1] =
                (above[column + 1] + 1).min(current[column] + 1).min(substitute);
        }
        std::mem::swap(&mut above, &mut current);
    }
    above[b.len()]
}
