//! Generic structural colorizer applied to both `dw` output and error
//! text, regardless of the script's declared output MIME (JSON/XML/CSV/...).
//! Colors by token shape rather than implementing a per-format highlighter.

// ponytail: not yet wired into a command (lands with watch mode, #3);
// allowed dead here so this PR can ship the colorizer standalone with
// full test coverage.
#![allow(dead_code)]

use colored::Colorize;

/// Colorizes `text`: quoted strings green, numbers cyan, structural
/// punctuation (`{}[]:,`) dimmed, everything else left as-is.
pub fn colorize(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
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
            let s: String = chars[start..i].iter().collect();
            out.push_str(&s.green().to_string());
        } else if c.is_ascii_digit()
            || (c == '-' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            out.push_str(&s.cyan().to_string());
        } else if matches!(c, '{' | '}' | '[' | ']' | ':' | ',') {
            out.push_str(&c.to_string().dimmed().to_string());
            i += 1;
        } else {
            out.push(c);
            i += 1;
        }
    }

    out
}

/// Strips the ANSI escape codes `colorize` emits, for round-trip testing.
fn strip_ansi(s: &str) -> String {
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
}
