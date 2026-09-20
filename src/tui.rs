//! The 3-region TUI: input pane, script pane, and a shared output/error
//! pane. Layout and rendering are pure (testable via
//! `ratatui::backend::TestBackend`); the live-reload loop that drives
//! this lives in `main.rs`, alongside the terminal it owns.

use crate::span::{SpanStyle, StyledSpan};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

/// The three regions of the playground screen: input (top-left), script
/// (bottom-left), and shared output/error (right, full height).
pub struct Regions {
    pub input: Rect,
    pub script: Rect,
    pub output: Rect,
}

/// Splits `area` into the three playground regions: a left column (input
/// over script) and a right column (output), 50/50 by width.
pub fn layout(area: Rect) -> Regions {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(columns[0]);

    Regions {
        input: left[0],
        script: left[1],
        output: columns[1],
    }
}

/// Content to display: highlighted script source, one tab per bound
/// input (name + highlighted raw text — always at least one entry, a
/// placeholder if none is bound), which one is active, and the last
/// highlighted output/error.
pub struct PlaygroundState {
    pub script_spans: Vec<StyledSpan>,
    pub inputs: Vec<(String, Vec<StyledSpan>)>,
    pub active_input: usize,
    pub output_spans: Vec<StyledSpan>,
}

fn ratatui_style(style: SpanStyle) -> Style {
    match style {
        SpanStyle::Plain => Style::default(),
        SpanStyle::Green => Style::default().fg(Color::Green),
        SpanStyle::Cyan => Style::default().fg(Color::Cyan),
        SpanStyle::Dimmed => Style::default().add_modifier(Modifier::DIM),
        SpanStyle::Magenta => Style::default().fg(Color::Magenta),
    }
}

/// Converts `colorize`/`dwl_highlight`'s tokenized spans into a ratatui
/// `Text`, splitting each span on embedded newlines so multi-line source
/// (a whole script, a multi-line error) becomes one `Line` per source
/// line while keeping each token's own style.
pub fn spans_to_text(spans: &[StyledSpan]) -> Text<'static> {
    let mut lines: Vec<Line> = vec![Line::default()];
    for span in spans {
        for (i, part) in span.text.split('\n').enumerate() {
            if i > 0 {
                lines.push(Line::default());
            }
            if !part.is_empty() {
                lines
                    .last_mut()
                    .expect("just pushed if i > 0, seeded with one line otherwise")
                    .spans
                    .push(Span::styled(part.to_string(), ratatui_style(span.style)));
            }
        }
    }
    Text::from(lines)
}

/// Builds the input pane's title as a tab bar: each bound input's name,
/// with the active one bracketed (e.g. `[payload] headers`). With only
/// one input bound this just brackets the one name — still correct,
/// just not visually a "tab bar" until a second input is bound.
fn input_tabs_title(inputs: &[(String, Vec<StyledSpan>)], active: usize) -> String {
    inputs
        .iter()
        .enumerate()
        .map(|(i, (name, _))| {
            if i == active {
                format!("[{name}]")
            } else {
                name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Draws the 3-pane playground into `frame`, bordered and titled, with
/// each pane's content highlighted per its spans. The input pane shows
/// only the active tab's content; `Tab`/arrows cycle it (see
/// `main.rs::run_playground_ticks`).
pub fn render(frame: &mut Frame, state: &PlaygroundState) {
    let regions = layout(frame.area());

    let active = state.active_input.min(state.inputs.len().saturating_sub(1));
    let active_spans = state
        .inputs
        .get(active)
        .map(|(_, spans)| spans.as_slice())
        .unwrap_or(&[]);

    frame.render_widget(
        Paragraph::new(spans_to_text(active_spans)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(input_tabs_title(&state.inputs, active)),
        ),
        regions.input,
    );
    frame.render_widget(
        Paragraph::new(spans_to_text(&state.script_spans))
            .block(Block::default().borders(Borders::ALL).title("script")),
        regions.script,
    );
    frame.render_widget(
        Paragraph::new(spans_to_text(&state.output_spans))
            .block(Block::default().borders(Borders::ALL).title("output")),
        regions.output,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn layout_splits_left_column_into_input_over_script_and_right_column_into_output() {
        let area = Rect::new(0, 0, 100, 40);
        let regions = layout(area);

        assert_eq!(regions.input.x, 0);
        assert_eq!(regions.script.x, 0);
        assert_eq!(regions.output.x, regions.input.width);

        assert_eq!(regions.input.y, 0);
        assert_eq!(regions.script.y, regions.input.y + regions.input.height);

        assert_eq!(regions.input.height + regions.script.height, area.height);
        assert_eq!(regions.input.width, regions.script.width);
        assert_eq!(regions.input.width + regions.output.width, area.width);
    }

    #[test]
    fn spans_to_text_splits_on_embedded_newlines() {
        let spans = vec![StyledSpan::new("a\nb\nc", SpanStyle::Plain)];
        let text = spans_to_text(&spans);
        assert_eq!(text.lines.len(), 3);
        assert_eq!(text.lines[0].spans[0].content, "a");
        assert_eq!(text.lines[1].spans[0].content, "b");
        assert_eq!(text.lines[2].spans[0].content, "c");
    }

    #[test]
    fn spans_to_text_keeps_each_tokens_own_style_within_a_line() {
        let spans = vec![
            StyledSpan::new("var", SpanStyle::Magenta),
            StyledSpan::new(" x", SpanStyle::Plain),
        ];
        let text = spans_to_text(&spans);
        assert_eq!(text.lines.len(), 1);
        assert_eq!(
            text.lines[0].spans[0].style,
            ratatui_style(SpanStyle::Magenta)
        );
        assert_eq!(
            text.lines[0].spans[1].style,
            ratatui_style(SpanStyle::Plain)
        );
    }

    #[test]
    fn spans_to_text_of_empty_spans_is_a_single_empty_line() {
        let text = spans_to_text(&[]);
        assert_eq!(text.lines.len(), 1);
        assert!(text.lines[0].spans.is_empty());
    }

    fn fake_state() -> PlaygroundState {
        PlaygroundState {
            script_spans: vec![StyledSpan::new(
                "output application/json --- {}",
                SpanStyle::Plain,
            )],
            inputs: vec![(
                "payload".to_string(),
                vec![StyledSpan::new(r#"{"a": 1}"#, SpanStyle::Plain)],
            )],
            active_input: 0,
            output_spans: vec![StyledSpan::new(r#"{"a": 1}"#, SpanStyle::Plain)],
        }
    }

    #[test]
    fn render_draws_all_three_titled_panes() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| render(frame, &fake_state())).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("payload"));
        assert!(content.contains("script"));
        assert!(content.contains("output"));
        assert!(content.contains("{\"a\": 1}"));
    }

    #[test]
    fn render_shows_a_placeholder_label_when_no_input_is_bound() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = fake_state();
        state.inputs = vec![("no input bound".to_string(), vec![])];

        terminal.draw(|frame| render(frame, &state)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("no input bound"));
    }

    #[test]
    fn input_tabs_title_brackets_only_the_active_name() {
        let inputs = vec![
            ("payload".to_string(), vec![]),
            ("headers".to_string(), vec![]),
        ];
        assert_eq!(input_tabs_title(&inputs, 0), "[payload] headers");
        assert_eq!(input_tabs_title(&inputs, 1), "payload [headers]");
    }

    #[test]
    fn render_shows_only_the_active_tabs_content() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = fake_state();
        state.inputs = vec![
            (
                "payload".to_string(),
                vec![StyledSpan::new("PAY", SpanStyle::Plain)],
            ),
            (
                "headers".to_string(),
                vec![StyledSpan::new("HDR", SpanStyle::Plain)],
            ),
        ];
        state.active_input = 1;

        terminal.draw(|frame| render(frame, &state)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("[headers]"));
        assert!(content.contains("HDR"));
        assert!(!content.contains("PAY"));
    }

    fn buffer_to_string(buffer: &ratatui::buffer::Buffer) -> String {
        let area = buffer.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }
}
