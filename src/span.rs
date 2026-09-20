//! Shared token+style representation so the CLI's ANSI-string rendering
//! and (landing in a later Wave 4 PR) a `ratatui` TUI's structured
//! widgets can consume the *same* highlighting logic, instead of either
//! duplicating it or having the TUI re-parse ANSI escape codes back out
//! of an already-rendered string (exactly the class of bug fixed in
//! dwl_highlight's ANSI-corruption issue).

use colored::Colorize;

/// A highlighting intent, independent of how it's ultimately rendered —
/// ANSI escape codes for a terminal, or a `ratatui::style::Style` for a
/// TUI (that mapping lands with the TUI itself).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanStyle {
    Plain,
    Green,
    Cyan,
    Dimmed,
    Magenta,
}

/// One highlighted run of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledSpan {
    pub text: String,
    pub style: SpanStyle,
}

impl StyledSpan {
    pub fn new(text: impl Into<String>, style: SpanStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

/// Renders `spans` as a single ANSI-coded string for terminal output.
pub fn render_ansi(spans: &[StyledSpan]) -> String {
    let mut out = String::new();
    for span in spans {
        match span.style {
            SpanStyle::Plain => out.push_str(&span.text),
            SpanStyle::Green => out.push_str(&span.text.green().to_string()),
            SpanStyle::Cyan => out.push_str(&span.text.cyan().to_string()),
            SpanStyle::Dimmed => out.push_str(&span.text.dimmed().to_string()),
            SpanStyle::Magenta => out.push_str(&span.text.magenta().to_string()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_ansi_concatenates_plain_text_unchanged() {
        colored::control::set_override(true);
        let spans = vec![
            StyledSpan::new("a", SpanStyle::Plain),
            StyledSpan::new("b", SpanStyle::Plain),
        ];
        assert_eq!(render_ansi(&spans), "ab");
    }

    #[test]
    fn render_ansi_wraps_each_style() {
        colored::control::set_override(true);
        assert_eq!(
            render_ansi(&[StyledSpan::new("x", SpanStyle::Green)]),
            "x".green().to_string()
        );
        assert_eq!(
            render_ansi(&[StyledSpan::new("x", SpanStyle::Cyan)]),
            "x".cyan().to_string()
        );
        assert_eq!(
            render_ansi(&[StyledSpan::new("x", SpanStyle::Dimmed)]),
            "x".dimmed().to_string()
        );
        assert_eq!(
            render_ansi(&[StyledSpan::new("x", SpanStyle::Magenta)]),
            "x".magenta().to_string()
        );
    }

    #[test]
    fn render_ansi_of_empty_spans_is_empty() {
        assert_eq!(render_ansi(&[]), "");
    }
}
