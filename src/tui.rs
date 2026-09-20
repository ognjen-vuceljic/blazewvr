//! Static 3-region TUI skeleton: input pane, script pane, and a shared
//! output/error pane. This is layout-only for now (issue #3 of Wave 4) —
//! live reload, tabbed multi-input, and picker/history integration land
//! in later Wave 4 PRs. Rendering is split from the terminal setup so the
//! layout itself is testable headlessly via `ratatui::backend::TestBackend`.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
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

/// State to display: script source, currently-bound input's raw text (or
/// a placeholder if none is bound), and the last output/error text.
pub struct PlaygroundState<'a> {
    pub script: &'a str,
    pub input_label: &'a str,
    pub input_text: &'a str,
    pub output_text: &'a str,
}

/// Draws the static 3-pane playground into `frame`, bordered and titled.
pub fn render(frame: &mut Frame, state: &PlaygroundState) {
    let regions = layout(frame.area());

    frame.render_widget(
        Paragraph::new(state.input_text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(state.input_label),
        ),
        regions.input,
    );
    frame.render_widget(
        Paragraph::new(state.script).block(Block::default().borders(Borders::ALL).title("script")),
        regions.script,
    );
    frame.render_widget(
        Paragraph::new(state.output_text)
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
    fn render_draws_all_three_titled_panes() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = PlaygroundState {
            script: "output application/json --- {}",
            input_label: "payload",
            input_text: r#"{"a": 1}"#,
            output_text: r#"{"a": 1}"#,
        };

        terminal.draw(|frame| render(frame, &state)).unwrap();

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
        let state = PlaygroundState {
            script: "---",
            input_label: "no input bound",
            input_text: "",
            output_text: "",
        };

        terminal.draw(|frame| render(frame, &state)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("no input bound"));
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
