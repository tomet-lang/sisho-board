//! Rendering for the TUI Extension Launcher modal.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use sisho_board_core::model::Model;
use sisho_board_core::trust::TrustState;
use sisho_tui::{ModalFrame, Theme};

use super::cursor_style;

pub(super) fn draw_launcher_modal(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let inner = ModalFrame::new(" TUI Extensions ", 76, 72).render(f, area, theme);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Directory: ", Style::default().fg(theme.muted)),
            Span::styled(
                model.dir.display().to_string(),
                Style::default().fg(theme.secondary),
            ),
        ]),
        Line::from(""),
    ];

    let items = model.launcher_items();
    let mut selected_range: Option<(usize, usize)> = None;

    if items.is_empty() {
        lines.push(Line::from(Span::styled(
            "No launchable TUI extensions found.",
            Style::default().fg(theme.muted),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Install a sisho-<word> extension with capabilities.tui = true, or inspect Tools & Extensions.",
            Style::default().fg(theme.muted),
        )));
    } else {
        for (idx, (_, ext)) in items.iter().enumerate() {
            let start_line = lines.len();
            let is_selected = idx == model.launcher_selected;
            let cursor = if is_selected { "> " } else { "  " };

            let (trust_icon, trust_style, trust_badge) = match ext.trust {
                TrustState::Allowed => (
                    "✓ ",
                    Style::default().fg(theme.success),
                    Span::styled(" [allowed]", Style::default().fg(theme.success)),
                ),
                TrustState::Untrusted => (
                    "? ",
                    Style::default().fg(theme.warning),
                    Span::styled(" [untrusted]", Style::default().fg(theme.warning)),
                ),
                TrustState::Denied => (
                    "✗ ",
                    Style::default().fg(theme.error),
                    Span::styled(" [denied]", Style::default().fg(theme.error)),
                ),
            };

            let mut header_spans = vec![
                Span::styled(cursor, cursor_style(is_selected, theme)),
                Span::styled(trust_icon, trust_style),
                Span::styled(
                    &ext.word,
                    if is_selected {
                        Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                    } else {
                        Style::default().add_modifier(Modifier::BOLD)
                    },
                ),
            ];

            if let Some(info) = &ext.info {
                header_spans.push(Span::styled(
                    format!(" v{}", info.version),
                    Style::default().fg(theme.secondary),
                ));
            }

            header_spans.push(trust_badge);
            lines.push(Line::from(header_spans));

            if ext.is_querying {
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled("querying...", Style::default().fg(theme.muted)),
                ]));
            } else if let Some(info) = &ext.info
                && !info.about.is_empty()
            {
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(&info.about, Style::default().fg(theme.inactive_tab_fg)),
                ]));
            } else if ext.trust == TrustState::Untrusted {
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        "(allow via [a] or [Enter] to query metadata)",
                        Style::default().fg(theme.muted),
                    ),
                ]));
            }

            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    ext.path.display().to_string(),
                    Style::default().fg(theme.muted),
                ),
            ]));

            let end_line = lines.len();
            if is_selected {
                selected_range = Some((start_line, end_line));
            }
        }
    }

    let visible_height = inner.height as usize;
    let scroll_y = match selected_range {
        Some((start, end)) if end > visible_height => {
            (end - visible_height).min(start)
        }
        _ => 0,
    };

    let paragraph = Paragraph::new(lines)
        .scroll((scroll_y as u16, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, inner);
}
