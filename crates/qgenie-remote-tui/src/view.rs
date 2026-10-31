use crate::app::{App, Confirm};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Row, Table, TableState},
};

pub(crate) fn draw(frame: &mut Frame<'_>, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(frame.area());
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    " QGENIE REMOTE ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("Local preview"),
            ]),
            Line::from(format!(
                " {} · {} {} · {} sessions",
                app.status.name,
                app.status.os,
                app.status.arch,
                app.sessions.len()
            )),
        ])
        .block(Block::default().borders(Borders::BOTTOM)),
        rows[0],
    );
    if let Some(screen) = &app.screen {
        let title = format!(
            " {} · {} · cursor {} ",
            screen.info.command.join(" "),
            if app.controller.is_some() {
                "CONTROLLING"
            } else {
                "OBSERVING"
            },
            screen.cursor.0
        );
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(rows[1]);
        frame.render_widget(block, rows[1]);
        draw_terminal(
            frame,
            screen.parser.screen(),
            inner,
            app.controller.is_some(),
        );
    } else if app.approvals {
        frame.render_widget(Paragraph::new("No pending approvals.\n\nTerminal shells do not issue structured approval requests.\nCodex and other structured agents are not available in this local preview.").block(Block::default().borders(Borders::ALL).title(" Owner approvals ")),rows[1]);
    } else {
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(27), Constraint::Percentage(73)])
            .split(rows[1]);
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    format!("● {}", app.status.name),
                    Style::default().fg(Color::Green),
                ),
                Line::from(format!("{} {}", app.status.os, app.status.arch)),
                Line::from(""),
                Line::from("Available: shell"),
                Line::from("Private local connection"),
                Line::from(""),
                Line::from("Allowed roots:"),
                Line::from(app.status.roots.join("\n")),
            ])
            .block(Block::default().borders(Borders::ALL).title(" Machines ")),
            panes[0],
        );
        let entries: Vec<_> = app
            .sessions
            .iter()
            .map(|session| {
                Row::new(vec![
                    session.command.first().cloned().unwrap_or_default(),
                    session.cwd.clone(),
                    format!("{:?}", session.state),
                ])
            })
            .collect();
        let table = Table::new(
            entries,
            [
                Constraint::Percentage(25),
                Constraint::Percentage(50),
                Constraint::Percentage(25),
            ],
        )
        .header(Row::new(["Command", "Directory", "State"]).style(Style::default().fg(Color::Cyan)))
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ")
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(if app.history {
                    " History · h live · ] next · [ first "
                } else {
                    " Live sessions · h history "
                }),
        );
        frame.render_stateful_widget(
            table,
            panes[1],
            &mut TableState::default().with_selected(Some(app.selected)),
        );
        if app.sessions.is_empty() {
            frame.render_widget(
                Paragraph::new(
                    "No sessions on this page. n creates a shell; h switches live/history.",
                ),
                Rect {
                    x: panes[1].x + 2,
                    y: panes[1].y + 3,
                    width: panes[1].width.saturating_sub(4),
                    height: 1,
                },
            );
        }
    }
    let warning = if app.screen.is_some() {
        "Local replay is private. Enterprise recording is not available. Ctrl-] opens commands."
    } else {
        "n new · enter observe · x terminate · p approvals · q quit"
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(app.message.as_str()),
            Line::styled(warning, Style::default().fg(Color::Yellow)),
        ])
        .block(Block::default().borders(Borders::TOP)),
        rows[2],
    );
    if let Some(cwd) = &app.new_cwd {
        popup(
            frame,
            " New terminal session ",
            &format!(
                "Working directory (within allowed roots):\n{cwd}\n\nEnter starts your shell · Esc cancels"
            ),
        );
    }
    if let Some(confirm) = &app.confirm {
        let text = match confirm {
            Confirm::Terminate(_) => {
                "Terminate this session and its shell?\n\nPress y to confirm. Any other key cancels."
            }
            Confirm::Takeover(_) => {
                "Another client controls this session. Take over?\n\nPress y to confirm. Any other key cancels."
            }
        };
        popup(frame, " Confirm ", text);
    }
}
fn popup(frame: &mut Frame<'_>, title: &str, text: &str) {
    let area = frame.area();
    let width = area.width.saturating_sub(4).min(80);
    let height = area.height.saturating_sub(2).min(8);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title(title)),
        popup,
    );
}
fn color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

pub(crate) fn draw_terminal(
    frame: &mut Frame<'_>,
    screen: &vt100::Screen,
    inner: Rect,
    controlling: bool,
) {
    let (height, width) = screen.size();
    let mut lines = Vec::new();
    for row in 0..height.min(inner.height) {
        let mut spans = Vec::new();
        for col in 0..width.min(inner.width) {
            if let Some(cell) = screen.cell(row, col) {
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut style = Style::default()
                    .fg(color(cell.fgcolor()))
                    .bg(color(cell.bgcolor()));
                if cell.bold() {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if cell.italic() {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                if cell.underline() {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                if cell.inverse() {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                let contents = cell.contents();
                spans.push(Span::styled(
                    if contents.is_empty() { " " } else { contents },
                    style,
                ));
            }
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), inner);
    if controlling && !screen.hide_cursor() {
        let (row, col) = screen.cursor_position();
        if row < inner.height && col < inner.width {
            frame.set_cursor_position((inner.x + col, inner.y + row));
        }
    }
}
