//! DataWeave-aware syntax highlighting, distinct from the generic
//! structural colorizer (colorize.rs) which handles JSON/XML/CSV *output*.
//! This is for text that's actually DWL source — most importantly the
//! source snippet `dw` echoes inside its own error messages (e.g.
//! `5| payload.items filter (...)`), which the generic colorizer would
//! otherwise treat as plain punctuation rather than as code.

use colored::Colorize;

const KEYWORDS: &[&str] = &[
    "output", "input", "var", "fun", "if", "else", "do", "match", "case", "type", "ns", "import",
    "as", "is", "unless", "using", "null", "true", "false", "and", "or", "not", "dw",
];

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Highlights DWL source: keywords magenta, string literals green,
/// numbers cyan, comments dimmed, structural punctuation dimmed,
/// identifiers/operators left as-is.
///
/// Strips any ANSI codes already present in `source` first. `dw`'s own
/// error output is not plain text — e.g. `\x1b[31m[ERROR] ...\x1b[0m` —
/// and scanning that byte-for-byte treats bytes *inside* dw's escape
/// sequences (the `[`, digits, and `m` that make up `\x1b[31m`) as
/// ordinary punctuation/numbers/identifiers to wrap in *new* color
/// codes, splicing them into the middle of dw's sequence and corrupting
/// it into something the terminal can't render correctly — the reported
/// "errors have no real color, it's all flat" symptom. Stripping first
/// means this always highlights plain text, never someone else's codes.
pub fn highlight(source: &str) -> String {
    let source = &crate::colorize::strip_ansi(source);
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
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
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            out.push_str(&s.dimmed().to_string());
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            let start = i;
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            let s: String = chars[start..i].iter().collect();
            out.push_str(&s.dimmed().to_string());
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            out.push_str(&s.cyan().to_string());
        } else if is_ident_start(c) {
            let start = i;
            while i < chars.len() && is_ident_continue(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if KEYWORDS.contains(&word.as_str()) {
                out.push_str(&word.magenta().to_string());
            } else {
                out.push_str(&word);
            }
        } else if matches!(c, '{' | '}' | '[' | ']' | '(' | ')' | ',' | ':' | '.') {
            out.push_str(&c.to_string().dimmed().to_string());
            i += 1;
        } else {
            out.push(c);
            i += 1;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colorize::strip_ansi;

    #[test]
    fn strips_to_original_text() {
        let src = r#"%dw 2.0
output application/json
// comment
---
if (payload.age > 17) "adult" else "minor""#;
        assert_eq!(strip_ansi(&highlight(src)), src);
    }

    #[test]
    fn highlights_keywords_magenta() {
        colored::control::set_override(true);
        assert_eq!(highlight("output"), "output".magenta().to_string());
        assert_eq!(highlight("var"), "var".magenta().to_string());
    }

    #[test]
    fn does_not_highlight_identifier_containing_keyword_as_substring() {
        colored::control::set_override(true);
        // "output" is a keyword but "outputs" is a different identifier
        assert_eq!(highlight("outputs"), "outputs");
    }

    #[test]
    fn highlights_strings_green() {
        colored::control::set_override(true);
        assert_eq!(highlight(r#""hello""#), "\"hello\"".green().to_string());
    }

    #[test]
    fn highlights_numbers_cyan() {
        colored::control::set_override(true);
        assert_eq!(highlight("42"), "42".cyan().to_string());
        assert_eq!(highlight("2.5"), "2.5".cyan().to_string());
    }

    #[test]
    fn highlights_line_comments_dimmed() {
        colored::control::set_override(true);
        assert_eq!(highlight("// hi"), "// hi".dimmed().to_string());
    }

    #[test]
    fn line_comment_stops_at_newline() {
        let src = "// hi\nvar x = 1";
        let stripped = strip_ansi(&highlight(src));
        assert_eq!(stripped, src);
    }

    #[test]
    fn highlights_block_comments_dimmed() {
        colored::control::set_override(true);
        assert_eq!(highlight("/* hi */"), "/* hi */".dimmed().to_string());
    }

    #[test]
    fn unterminated_block_comment_consumes_rest_of_input() {
        let src = "var x /* never closed";
        assert_eq!(strip_ansi(&highlight(src)), src);
    }

    #[test]
    fn highlights_punctuation_dimmed() {
        colored::control::set_override(true);
        assert_eq!(highlight("{"), "{".dimmed().to_string());
        assert_eq!(highlight("."), ".".dimmed().to_string());
    }

    #[test]
    fn leaves_plain_identifiers_and_operators_uncolored() {
        colored::control::set_override(true);
        assert_eq!(highlight("payload"), "payload");
        assert_eq!(highlight("->"), "->");
    }

    #[test]
    fn real_error_source_snippet_round_trips() {
        let src = "5| payload.items filter ((i) -> i.age > 17)";
        assert_eq!(strip_ansi(&highlight(src)), src);
    }

    #[test]
    fn strips_pre_existing_ansi_before_highlighting() {
        // Verbatim from a real `dw run` failure this session: dw's own
        // error output already contains ANSI codes. Scanning that
        // byte-for-byte (without stripping first) treats the bytes
        // *inside* dw's own escape sequences (the `[`, digits, and `m`
        // that make up `\x1b[31m`) as punctuation/numbers to wrap in new
        // color codes of their own, splicing them into the middle of
        // dw's sequence and corrupting it — which is why colored error
        // output was reported as looking flat/uncolored. Round-tripping
        // through strip_ansi must reproduce dw's plain-text message
        // exactly, with none of that corruption's extra/garbled bytes.
        let raw_from_dw = "\x1b[31m[ERROR] Error while executing the script:\x1b[0m";
        let plain = "[ERROR] Error while executing the script:";
        assert_eq!(strip_ansi(&highlight(raw_from_dw)), plain);
    }
}
