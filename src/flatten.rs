//! Flattens a multi-line DataWeave script into a single line so it can be
//! fed to `dw repl`, which evaluates one document per input line.
//!
//! A naive `//`/`/* */` strip-and-join breaks the moment a comment-like
//! sequence appears inside a string literal (e.g. `"http://example.com"`),
//! which is extremely common in real DataWeave scripts. This tracks
//! string-literal state (respecting `\"` escapes) so comments are only
//! stripped outside of strings.

// ponytail: not yet wired into run/watch commands (lands with the REPL
// supervisor and watch-mode PRs); allowed dead here so this PR can ship
// the tokenizer standalone with full test coverage.
#![allow(dead_code)]

#[derive(Clone, Copy, PartialEq)]
enum State {
    Code,
    LineComment,
    BlockComment,
    StringLit,
}

/// Flattens `src` into a single-line document, stripping `//` and `/* */`
/// comments that fall outside string literals, and collapsing all
/// remaining newlines to single spaces.
pub fn flatten(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut state = State::Code;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();

        match state {
            State::Code => {
                if c == '"' {
                    state = State::StringLit;
                    out.push(c);
                } else if c == '/' && next == Some('/') {
                    state = State::LineComment;
                    i += 1;
                } else if c == '/' && next == Some('*') {
                    state = State::BlockComment;
                    i += 1;
                } else if c == '\n' || c == '\r' {
                    out.push(' ');
                } else {
                    out.push(c);
                }
            }
            State::StringLit => {
                if c == '\\' && next.is_some() {
                    out.push(c);
                    if let Some(n) = next {
                        out.push(n);
                    }
                    i += 1;
                } else if c == '"' {
                    state = State::Code;
                    out.push(c);
                } else if c == '\n' || c == '\r' {
                    out.push(' ');
                } else {
                    out.push(c);
                }
            }
            State::LineComment => {
                if c == '\n' {
                    state = State::Code;
                    out.push(' ');
                }
            }
            State::BlockComment => {
                if c == '*' && next == Some('/') {
                    state = State::Code;
                    i += 1;
                } else if c == '\n' || c == '\r' {
                    out.push(' ');
                }
            }
        }
        i += 1;
    }

    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_line_comments() {
        let src = "1 + 1 // add\n+ 2";
        assert_eq!(flatten(src), "1 + 1 + 2");
    }

    #[test]
    fn strips_block_comments() {
        let src = "1 /* block\ncomment */ + 2";
        assert_eq!(flatten(src), "1 + 2");
    }

    #[test]
    fn does_not_strip_comment_markers_inside_string_literals() {
        let src = r#"payload.url default "http://example.com""#;
        assert_eq!(flatten(src), r#"payload.url default "http://example.com""#);
    }

    #[test]
    fn does_not_treat_block_marker_inside_string_as_comment() {
        let src = r#""a/*b" ++ "c*/d""#;
        assert_eq!(flatten(src), r#""a/*b" ++ "c*/d""#);
    }

    #[test]
    fn handles_escaped_quotes_inside_strings() {
        let src = r#""she said \"hi // not a comment\"" ++ "tail""#;
        assert_eq!(
            flatten(src),
            r#""she said \"hi // not a comment\"" ++ "tail""#
        );
    }

    #[test]
    fn collapses_multiple_newlines_and_whitespace() {
        let src = "%dw 2.0\noutput application/json\n\n\n---\npayload";
        assert_eq!(flatten(src), "%dw 2.0 output application/json --- payload");
    }

    #[test]
    fn real_world_multiline_script_with_leading_comment() {
        let src = "%dw 2.0\noutput application/json\n// fetch adults only\n---\npayload.items filter ((i) -> i.age > 17) // trailing\n";
        assert_eq!(
            flatten(src),
            r#"%dw 2.0 output application/json --- payload.items filter ((i) -> i.age > 17)"#
        );
    }

    #[test]
    fn line_comment_at_end_of_input_without_trailing_newline() {
        let src = "payload // trailing comment, no newline after";
        assert_eq!(flatten(src), "payload");
    }

    #[test]
    fn unterminated_block_comment_consumes_rest_of_input() {
        let src = "payload /* never closed";
        assert_eq!(flatten(src), "payload");
    }
}
