//! Bare web-address autolinking for transcript prose.
//!
//! The TUI's inline tokenizer ported its bare-URL rules from marked
//! (`pa-tui/src/autolink.rs`). This window has no Markdown tokenizer, so the
//! same rules run as a small pass before the transcript markup escapers:
//! `http(s)://` and `www.` become `[label](href)`, while code spans and links
//! the author already wrote are left alone.

use std::sync::LazyLock;

use fancy_regex::Regex;

/// `rules.inline.url` (GFM): bare protocol links and `www.` links.
static BARE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^((?:[hH][tT][tT][pP][sS]?|[fF][tT][pP])://|www\.)(?:[a-zA-Z0-9\-]+\.?)+[^\s<]*|^[A-Za-z0-9._+-]+(@)[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![-_])",
    )
    .expect("static autolink regex")
});

/// `rules.inline._backpedal`: trailing punctuation is peeled unless it is a
/// balanced parenthesis group.
static BACKPEDAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:[^?!.,:;*_'"~()&]+|\([^)]*\)|&(?![a-zA-Z0-9]+;$)|[?!.,:;*_'"~)]+(?!$))+"#)
        .expect("static autolink regex")
});

struct Token {
    raw_chars: usize,
    text: String,
    href: String,
}

fn bare_token(src: &str) -> Option<Token> {
    let caps = BARE.captures(src).ok().flatten()?;
    let scheme = caps.get(1)?.as_str().to_string();
    let matched = caps.get(0)?.as_str();
    let mut start = 0;
    let mut end = matched.len();
    while let Ok(Some(m)) = BACKPEDAL.find(&matched[start..end]) {
        if m.start() == 0 && m.end() == end - start {
            break;
        }
        start += m.start();
        end = start + m.end() - m.start();
    }
    let text = &matched[start..end];
    let href = if scheme == "www." {
        format!("http://{text}")
    } else {
        text.to_string()
    };
    Some(Token {
        raw_chars: text.chars().count(),
        text: text.to_string(),
        href,
    })
}

fn starts_with(bytes: &[char], i: usize, prefix: &str, ignore_case: bool) -> bool {
    prefix.chars().enumerate().all(|(k, pc)| {
        bytes.get(i + k).is_some_and(|&c| {
            if ignore_case {
                c.eq_ignore_ascii_case(&pc)
            } else {
                c == pc
            }
        })
    })
}

fn web_candidate(bytes: &[char], i: usize) -> bool {
    starts_with(bytes, i, "https://", true)
        || starts_with(bytes, i, "http://", true)
        || starts_with(bytes, i, "www.", false)
}

fn push_label(text: &str, out: &mut String) {
    for ch in text.chars() {
        if matches!(ch, '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
}

/// Turn bare web addresses into Markdown links without touching inline code,
/// existing link labels, or existing link destinations.
pub(crate) fn autolink_markup(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut starts: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
    starts.push(text.len());
    let mut out = String::with_capacity(text.len() + 32);
    let mut i = 0;
    let mut in_code = false;
    let mut in_label = false;
    let mut in_dest = false;
    let mut after_label = false;
    let mut escaped = false;

    while i < chars.len() {
        let ch = chars[i];
        if escaped {
            out.push(ch);
            escaped = false;
            i += 1;
            continue;
        }
        if ch == '\\' {
            out.push(ch);
            escaped = true;
            i += 1;
            continue;
        }
        if ch == '`' && !in_label && !in_dest {
            let mut ticks = 1;
            while i + ticks < chars.len() && chars[i + ticks] == '`' {
                ticks += 1;
            }
            for _ in 0..ticks {
                out.push('`');
            }
            in_code = !in_code;
            i += ticks;
            continue;
        }
        if in_code {
            out.push(ch);
            i += 1;
            continue;
        }
        if in_label {
            out.push(ch);
            if ch == ']' {
                in_label = false;
                after_label = true;
            }
            i += 1;
            continue;
        }
        if after_label {
            after_label = false;
            if ch == '(' {
                in_dest = true;
                out.push(ch);
                i += 1;
                continue;
            }
        }
        if in_dest {
            out.push(ch);
            if ch == ')' {
                in_dest = false;
            }
            i += 1;
            continue;
        }
        if ch == '[' {
            in_label = true;
            out.push(ch);
            i += 1;
            continue;
        }
        if web_candidate(&chars, i) {
            if let Some(token) = bare_token(&text[starts[i]..]) {
                out.push('[');
                push_label(&token.text, &mut out);
                out.push_str("](");
                out.push_str(&token.href);
                out.push(')');
                i += token.raw_chars;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::autolink_markup;

    #[test]
    fn bare_https_url_becomes_a_markdown_link() {
        assert_eq!(
            autolink_markup("see https://example.com/a?b=1."),
            "see [https://example.com/a?b=1](https://example.com/a?b=1)."
        );
    }

    #[test]
    fn www_gains_an_http_href() {
        assert_eq!(
            autolink_markup("go to www.example.com"),
            "go to [www.example.com](http://www.example.com)"
        );
    }

    #[test]
    fn code_spans_and_existing_links_are_untouched() {
        assert_eq!(
            autolink_markup("`https://example.com` and [docs](https://example.com)"),
            "`https://example.com` and [docs](https://example.com)"
        );
    }

    #[test]
    fn punctuation_stays_outside_the_link() {
        assert_eq!(
            autolink_markup("(https://example.com),"),
            "([https://example.com](https://example.com)),"
        );
    }
}
