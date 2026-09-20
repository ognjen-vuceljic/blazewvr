//! Generic structural colorizer applied to both `dw` output and error
//! text, regardless of the script's declared output MIME (JSON/XML/CSV/...).
//! Colors by token shape rather than implementing a per-format highlighter.

use crate::span::{SpanStyle, StyledSpan, render_ansi};

/// Tokenizes `text` into styled spans: quoted strings green, numbers
/// cyan, structural punctuation (`{}[]:,`) dimmed, everything else
/// plain. This is the shape a TUI consumes directly; `colorize` renders
/// the same tokenization to an ANSI string for terminal output.
pub fn tokenize(text: &str) -> Vec<StyledSpan> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut plain_start = 0;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            flush_plain(&mut spans, &chars, plain_start, start);
            spans.push(StyledSpan::new(
                chars[start..i].iter().collect::<String>(),
                SpanStyle::Green,
            ));
            plain_start = i;
        } else if c.is_ascii_digit()
            || (c == '-' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            flush_plain(&mut spans, &chars, plain_start, start);
            spans.push(StyledSpan::new(
                chars[start..i].iter().collect::<String>(),
                SpanStyle::Cyan,
            ));
            plain_start = i;
        } else if matches!(c, '{' | '}' | '[' | ']' | ':' | ',') {
            flush_plain(&mut spans, &chars, plain_start, i);
            spans.push(StyledSpan::new(c.to_string(), SpanStyle::Dimmed));
            i += 1;
            plain_start = i;
        } else {
            i += 1;
        }
    }
    flush_plain(&mut spans, &chars, plain_start, chars.len());
    spans
}

/// Pushes a `Plain` span for `chars[start..end]`, if non-empty.
fn flush_plain(spans: &mut Vec<StyledSpan>, chars: &[char], start: usize, end: usize) {
    if start < end {
        spans.push(StyledSpan::new(
            chars[start..end].iter().collect::<String>(),
            SpanStyle::Plain,
        ));
    }
}

/// Colorizes `text`: quoted strings green, numbers cyan, structural
/// punctuation (`{}[]:,`) dimmed, everything else left as-is.
pub fn colorize(text: &str) -> String {
    render_ansi(&tokenize(text))
}

/// Strips the ANSI escape codes `colorize` emits, for round-trip testing.
pub(crate) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use colored::Colorize;

    fn force_color() {
        colored::control::set_override(true);
    }

    #[test]
    fn strips_to_original_text() {
        let text = r#"{"age": 21, "items": [1, 2.5, -3]}"#;
        assert_eq!(strip_ansi(&colorize(text)), text);
    }

    #[test]
    fn colorizes_strings_green() {
        force_color();
        let out = colorize(r#""hello""#);
        assert_eq!(out, "\"hello\"".green().to_string());
    }

    #[test]
    fn colorizes_numbers_cyan() {
        force_color();
        let out = colorize("42");
        assert_eq!(out, "42".cyan().to_string());
    }

    #[test]
    fn colorizes_negative_and_decimal_numbers() {
        force_color();
        assert_eq!(colorize("-3"), "-3".cyan().to_string());
        assert_eq!(colorize("2.5"), "2.5".cyan().to_string());
    }

    #[test]
    fn colorizes_punctuation_dimmed() {
        force_color();
        assert_eq!(colorize("{"), "{".dimmed().to_string());
        assert_eq!(colorize(","), ",".dimmed().to_string());
    }

    #[test]
    fn leaves_bare_words_uncolored() {
        force_color();
        assert_eq!(colorize("payload"), "payload");
    }

    #[test]
    fn handles_escaped_quotes_inside_strings() {
        let text = r#""she said \"hi\"""#;
        assert_eq!(strip_ansi(&colorize(text)), text);
    }

    #[test]
    fn hyphen_not_followed_by_digit_is_not_a_number() {
        force_color();
        assert_eq!(colorize("a-b"), "a-b");
    }

    #[test]
    fn unterminated_string_literal_consumes_rest_of_input() {
        let text = r#""never closed"#;
        assert_eq!(strip_ansi(&colorize(text)), text);
    }

    #[test]
    fn tokenize_produces_the_expected_span_sequence() {
        // Locks in the structured shape a TUI will consume directly,
        // not just the ANSI-rendered string.
        assert_eq!(
            tokenize(r#"{"age": 21}"#),
            vec![
                StyledSpan::new("{", SpanStyle::Dimmed),
                StyledSpan::new("\"age\"", SpanStyle::Green),
                StyledSpan::new(":", SpanStyle::Dimmed),
                StyledSpan::new(" ", SpanStyle::Plain),
                StyledSpan::new("21", SpanStyle::Cyan),
                StyledSpan::new("}", SpanStyle::Dimmed),
            ]
        );
    }

    #[test]
    fn tokenize_of_plain_text_is_a_single_plain_span() {
        assert_eq!(
            tokenize("payload"),
            vec![StyledSpan::new("payload", SpanStyle::Plain)]
        );
    }

    #[test]
    fn colorize_matches_render_ansi_of_tokenize() {
        let text = r#"{"age": 21, "items": [1, 2.5, -3]}"#;
        assert_eq!(colorize(text), render_ansi(&tokenize(text)));
    }
}
