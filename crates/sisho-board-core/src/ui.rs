//! Ratatui rendering for `sisho-board` (Board + Counter).

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use sisho_board_sdk::BoardStatus;

use crate::model::{FocusedPane, Model, PreviewSource, StepSlot};
use crate::runner::Status;
use crate::tools::Verdict;
use crate::trust::TrustState;

/// Render the complete sisho-board interface (Board + Counter).
pub fn draw(f: &mut Frame, model: &Model) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(12), Constraint::Length(4)])
        .split(f.area());

    draw_board(f, chunks[0], model);
    draw_counter(f, chunks[1], model);
}

fn draw_board(f: &mut Frame, area: Rect, model: &Model) {
    let board_block = Block::default()
        .borders(Borders::ALL)
        .title(" Sisho Board (Project Status) ")
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    let inner = board_block.inner(area);
    f.render_widget(board_block, area);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(inner);

    draw_left_column(f, columns[0], model);
    draw_right_column(f, columns[1], model);
}

fn draw_left_column(f: &mut Frame, area: Rect, model: &Model) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    draw_tools_and_extensions(f, rows[0], model);
    draw_broken_links(f, rows[1], model);
}

fn draw_tools_and_extensions(f: &mut Frame, area: Rect, model: &Model) {
    let is_focused = model.focused_pane == FocusedPane::Extensions;
    let mut items = Vec::new();

    // Tools
    items.push(ListItem::new(Line::from(vec![Span::styled(
        "Core Tools:",
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::Yellow),
    )])));

    for tool in &model.tools {
        let (icon, icon_style) = match &tool.verdict {
            Verdict::Fine(_) => ("  ✓ ", Style::default().fg(Color::Green)),
            Verdict::TooOld { .. } => ("  ✗ ", Style::default().fg(Color::Red)),
            Verdict::Unreported { .. } => ("  - ", Style::default().fg(Color::DarkGray)),
        };

        let ver_str = tool.version.as_deref().unwrap_or("not found");
        let note = match &tool.verdict {
            Verdict::Fine(_) => " (fine)",
            Verdict::TooOld { .. } => " (older than needed)",
            Verdict::Unreported { .. } => " (missing)",
        };

        items.push(ListItem::new(Line::from(vec![
            Span::styled(icon, icon_style),
            Span::styled(tool.name, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format!(": {ver_str}{note}")),
        ])));
    }

    // Extensions
    items.push(ListItem::new(Line::from("")));
    items.push(ListItem::new(Line::from(vec![Span::styled(
        "Installed Extensions:",
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::Yellow),
    )])));

    if model.extensions.is_empty() {
        items.push(ListItem::new(Line::from(vec![Span::styled(
            "  (none found on PATH)",
            Style::default().fg(Color::DarkGray),
        )])));
    } else {
        for (idx, ext) in model.extensions.iter().enumerate() {
            let is_selected = is_focused && idx == model.selected_extension;
            let cursor = if is_selected { "> " } else { "  " };

            let (trust_icon, trust_style, trust_badge) = match ext.trust {
                TrustState::Allowed => (
                    "✓ ",
                    Style::default().fg(Color::Green),
                    Span::styled(" [allowed]", Style::default().fg(Color::Green)),
                ),
                TrustState::Untrusted => (
                    "? ",
                    Style::default().fg(Color::Yellow),
                    Span::styled(" [untrusted: a/Enter]", Style::default().fg(Color::Yellow)),
                ),
                TrustState::Denied => (
                    "✗ ",
                    Style::default().fg(Color::Red),
                    Span::styled(" [denied: a/Enter]", Style::default().fg(Color::Red)),
                ),
            };

            let mut header_spans = vec![
                Span::styled(
                    cursor,
                    if is_selected {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default()
                    },
                ),
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
                    Style::default().fg(Color::Cyan),
                ));
            }

            header_spans.push(trust_badge);

            if let Some(note) = &ext.note {
                header_spans.push(Span::raw(" "));
                header_spans.push(Span::styled(
                    format!("({note})"),
                    Style::default().fg(Color::Yellow),
                ));
            }

            items.push(ListItem::new(Line::from(header_spans)));

            if let Some(info) = &ext.info
                && !info.about.is_empty()
            {
                items.push(ListItem::new(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(&info.about, Style::default().fg(Color::Gray)),
                ])));
            }

            items.push(ListItem::new(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    ext.path.display().to_string(),
                    Style::default().fg(Color::DarkGray),
                ),
            ])));
        }
    }

    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(" Tools & Extensions ")
            .title_style(if is_focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            }),
    );
    f.render_widget(list, area);
}

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn spinner(tick: usize) -> &'static str {
    SPINNER[tick % SPINNER.len()]
}

fn draw_broken_links(f: &mut Frame, area: Rect, model: &Model) {
    let is_focused = model.focused_pane == FocusedPane::BrokenLinks;
    let broken = model.broken_links();
    let mut items = Vec::new();

    if model.is_checking_links && model.links_report.is_none() {
        let spin = spinner(model.tick);
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("  {spin} "), Style::default().fg(Color::Cyan)),
            Span::styled(
                "Scanning links in background...",
                Style::default().fg(Color::Yellow),
            ),
        ])));
    } else {
        if let Some(report) = &model.links_report {
            items.push(ListItem::new(Line::from(vec![Span::styled(
                format!(
                    "Scanned {} files ({} links checked)",
                    report.files_scanned, report.links_checked
                ),
                Style::default().fg(Color::DarkGray),
            )])));
        }

        if broken.is_empty() {
            items.push(ListItem::new(Line::from(vec![Span::styled(
                "✓ No broken links found",
                Style::default().fg(Color::Green),
            )])));
        } else {
            let available_height = area.height.saturating_sub(3) as usize;
            let (start_idx, end_idx) = if available_height > 0 && broken.len() > available_height {
                let start = if model.selected_broken_link >= available_height {
                    model.selected_broken_link + 1 - available_height
                } else {
                    0
                };
                let end = (start + available_height).min(broken.len());
                (start, end)
            } else {
                (0, broken.len())
            };

            for (i, link) in broken[start_idx..end_idx].iter().enumerate() {
                let actual_idx = start_idx + i;
                let is_selected = is_focused && actual_idx == model.selected_broken_link;
                let cursor = if is_selected { "> " } else { "  " };
                let at_str = link.at.map(|p| format!(":{}", p.line)).unwrap_or_default();
                items.push(ListItem::new(Line::from(vec![
                    Span::styled(
                        cursor,
                        if is_selected {
                            Style::default().fg(Color::Cyan)
                        } else {
                            Style::default()
                        },
                    ),
                    Span::styled("✗ ", Style::default().fg(Color::Red)),
                    Span::styled(
                        format!("{}{at_str}", link.source.display()),
                        if is_selected {
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                        } else {
                            Style::default().fg(Color::White)
                        },
                    ),
                    Span::raw(format!(" -> {}", link.target)),
                ])));
            }
        }
    }

    let title = if model.is_checking_links {
        " Broken Links (scanning...) ".to_string()
    } else if !broken.is_empty() && is_focused {
        format!(
            " Broken Links ({}/{}) ",
            model.selected_broken_link + 1,
            broken.len()
        )
    } else {
        format!(" Broken Links ({}) ", broken.len())
    };

    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title)
            .title_style(if is_focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            }),
    );
    f.render_widget(list, area);
}

fn draw_right_column(f: &mut Frame, area: Rect, model: &Model) {
    let cmd_count = if model.check_slots.is_empty() {
        crate::tools::check_steps(&model.dir).len()
    } else {
        model.check_slots.len()
    };
    let checks_height = (cmd_count as u16) + 2;

    if model.has_extension_panes() {
        let ext_lines = count_extension_pane_lines(model) + 2;
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Max(checks_height),
                Constraint::Max(ext_lines as u16),
                Constraint::Min(4),
            ])
            .split(area);

        draw_checks(f, rows[0], model);
        draw_extension_panes(f, rows[1], model);
        draw_output_preview(f, rows[2], model);
    } else {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Max(checks_height), Constraint::Min(0)])
            .split(area);

        draw_checks(f, rows[0], model);
        draw_output_preview(f, rows[1], model);
    }
}

fn count_extension_pane_lines(model: &Model) -> usize {
    let panes = model.active_extension_panes();
    let mut lines = 0;
    for (_, pane) in &panes {
        if panes.len() > 1 {
            lines += 1;
        }
        if pane.gauge.is_some() {
            lines += 1;
        }
        lines += pane.items.len();
    }
    lines.max(1)
}

fn format_gauge_bar(percent: u8, width: usize) -> String {
    let clamped = percent.min(100) as usize;
    let filled = (clamped * width + 50) / 100;
    let empty = width.saturating_sub(filled);
    format!("[{}{}] {:>3}%", "█".repeat(filled), "░".repeat(empty), clamped)
}

fn draw_extension_panes(f: &mut Frame, area: Rect, model: &Model) {
    let is_focused = model.focused_pane == FocusedPane::ExtensionPanes;
    let panes = model.active_extension_panes();
    let mut items = Vec::new();
    let mut flat_idx = 0usize;

    for (word, pane) in &panes {
        if panes.len() > 1 {
            items.push(ListItem::new(Line::from(vec![Span::styled(
                format!("{} (sisho-{word}):", pane.title),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )])));
        }

        if let Some(gauge) = &pane.gauge {
            let bar = format_gauge_bar(gauge.percent, 10);
            let bar_color = if gauge.percent >= 80 {
                Color::Green
            } else if gauge.percent >= 50 {
                Color::Yellow
            } else {
                Color::Red
            };
            items.push(ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    bar,
                    Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(&gauge.label, Style::default().fg(Color::White)),
            ])));
        }

        for item in &pane.items {
            let is_selected = is_focused && flat_idx == model.selected_ext_item;
            let cursor = if is_selected { "> " } else { "  " };
            let (status_icon, status_style) = match item.status {
                BoardStatus::Pass => ("✓ ", Style::default().fg(Color::Green)),
                BoardStatus::Warn => (
                    "! ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                BoardStatus::Fail => (
                    "✗ ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                BoardStatus::Info => ("• ", Style::default().fg(Color::Cyan)),
            };

            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    cursor,
                    if is_selected {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default()
                    },
                ),
                Span::styled(status_icon, status_style),
                Span::styled(
                    &item.label,
                    if is_selected {
                        Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                    } else {
                        Style::default()
                    },
                ),
                Span::raw(": "),
                Span::styled(&item.value, Style::default().fg(Color::White)),
            ])));

            flat_idx += 1;
        }
    }

    let title = if let [(word, pane)] = panes.as_slice() {
        format!(" {} (sisho-{word}) ", pane.title)
    } else {
        " Extension Panes ".to_string()
    };

    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title)
            .title_style(if is_focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            }),
    );
    f.render_widget(list, area);
}

fn draw_checks(f: &mut Frame, area: Rect, model: &Model) {
    let is_focused = model.focused_pane == FocusedPane::Checks;
    let mut items = Vec::new();

    if model.check_slots.is_empty() {
        if model.is_checking {
            let spin = spinner(model.tick);
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {spin} "), Style::default().fg(Color::Cyan)),
                Span::styled(
                    "Running checks in background...",
                    Style::default().fg(Color::Yellow),
                ),
            ])));
        } else if model.is_fixing {
            let spin = spinner(model.tick);
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {spin} "), Style::default().fg(Color::Cyan)),
                Span::styled(
                    "Applying fixes in background...",
                    Style::default().fg(Color::Yellow),
                ),
            ])));
        } else {
            items.push(ListItem::new(Line::from("No checks run yet. Press [c].")));
        }
    } else {
        for (i, slot) in model.check_slots.iter().enumerate() {
            let is_selected = is_focused && i == model.selected_step;
            let cursor = if is_selected { "> " } else { "  " };

            let line = match slot {
                StepSlot::Done(outcome) => {
                    let (status_icon, status_style) = match outcome.status {
                        Status::Passed => ("✓ ", Style::default().fg(Color::Green)),
                        Status::Failed { .. } => (
                            "✗ ",
                            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                        ),
                        Status::NotFound => ("- ", Style::default().fg(Color::Yellow)),
                        Status::CouldNotStart { .. } => ("! ", Style::default().fg(Color::Red)),
                        Status::Signalled { .. } => ("! ", Style::default().fg(Color::Magenta)),
                    };

                    let duration_str = format!("({:.2}s)", outcome.duration.as_secs_f64());
                    Line::from(vec![
                        Span::styled(
                            cursor,
                            if is_selected {
                                Style::default().fg(Color::Cyan)
                            } else {
                                Style::default()
                            },
                        ),
                        Span::styled(status_icon, status_style),
                        Span::styled(
                            &outcome.name,
                            if is_selected {
                                Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                            } else {
                                Style::default()
                            },
                        ),
                        Span::raw(" "),
                        Span::styled(duration_str, Style::default().fg(Color::DarkGray)),
                    ])
                }
                StepSlot::Running { name } => {
                    let spin = spinner(model.tick.wrapping_add(i));
                    Line::from(vec![
                        Span::styled(
                            cursor,
                            if is_selected {
                                Style::default().fg(Color::Cyan)
                            } else {
                                Style::default()
                            },
                        ),
                        Span::styled(format!("{spin} "), Style::default().fg(Color::Cyan)),
                        Span::styled(
                            name,
                            if is_selected {
                                Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                            } else {
                                Style::default()
                            },
                        ),
                        Span::raw(" "),
                        Span::styled("(running...)", Style::default().fg(Color::Yellow)),
                    ])
                }
            };

            items.push(ListItem::new(line));
        }
    }

    let title = if model.is_checking {
        " Integrity & Checks (running...) "
    } else if model.is_fixing {
        " Integrity & Checks (fixing...) "
    } else {
        " Integrity & Checks "
    };

    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title)
            .title_style(if is_focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            }),
    );
    f.render_widget(list, area);
}

fn draw_output_preview(f: &mut Frame, area: Rect, model: &Model) {
    let is_focused = model.focused_pane == FocusedPane::OutputPreview;
    let mut total_lines = 0;

    let content = match model.preview_source {
        PreviewSource::ExtensionItem => {
            if let Some(ext_ref) = model.selected_extension_pane_item() {
                let mut lines = Vec::new();
                lines.push(Line::from(vec![
                    Span::styled("Extension: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(format!("sisho-{} — {}", ext_ref.word, ext_ref.pane.title)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("Item: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{}: {}", ext_ref.item.label, ext_ref.item.value),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]));

                if let Some(detail) = &ext_ref.item.detail {
                    lines.push(Line::from(""));
                    for l in detail.lines() {
                        lines.push(Line::from(l));
                    }
                } else if let Some(summary) = &ext_ref.pane.summary {
                    lines.push(Line::from(""));
                    for l in summary.lines() {
                        lines.push(Line::from(l));
                    }
                } else {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "(no additional detail for this item)",
                        Style::default().fg(Color::DarkGray),
                    )));
                }
                total_lines = lines.len();
                lines
            } else {
                vec![Line::from("No extension item selected.")]
            }
        }
        PreviewSource::CheckStep => {
            if let Some(outcome) = model.selected_outcome() {
                let mut lines = Vec::new();
                lines.push(Line::from(vec![
                    Span::styled("Command: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(&outcome.command_line),
                ]));

                if !outcome.stdout.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "--- stdout ---",
                        Style::default().fg(Color::DarkGray),
                    )));
                    for l in outcome.stdout.lines() {
                        lines.push(Line::from(l));
                    }
                }
                if !outcome.stderr.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "--- stderr ---",
                        Style::default().fg(Color::Red),
                    )));
                    for l in outcome.stderr.lines() {
                        lines.push(Line::from(Span::styled(l, Style::default().fg(Color::Red))));
                    }
                }

                if outcome.stdout.is_empty() && outcome.stderr.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "(no output produced)",
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                total_lines = lines.len();
                lines
            } else {
                vec![Line::from("No step selected or step is still running.")]
            }
        }
    };

    let visible_height = area.height.saturating_sub(2) as usize;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_y = model.preview_scroll.min(max_scroll);

    let title = if total_lines > visible_height {
        format!(
            " Step Output Preview (lines {}-{}/{}) ",
            scroll_y + 1,
            (scroll_y + visible_height).min(total_lines),
            total_lines
        )
    } else {
        " Step Output Preview ".to_string()
    };

    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };

    let paragraph = Paragraph::new(content)
        .scroll((scroll_y as u16, 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(title)
                .title_style(if is_focused {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                }),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}

fn draw_counter(f: &mut Frame, area: Rect, model: &Model) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Counter (Actions) ")
        .title_style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        );

    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = Vec::new();

    if model.confirm_fix {
        lines.push(Line::from(vec![
            Span::styled(
                " Apply fixes across the project? (writes files)  ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "[y] Yes, apply fixes",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(
                "[n/Esc] Cancel",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ]));
    } else {
        let focus_desc = match model.focused_pane {
            FocusedPane::Checks => "Checks",
            FocusedPane::Extensions => "Extensions",
            FocusedPane::BrokenLinks => "Links",
            FocusedPane::ExtensionPanes => "ExtPane",
            FocusedPane::OutputPreview => "Preview",
        };

        let mut action_spans = vec![
            Span::styled("[Tab]", Style::default().fg(Color::Cyan)),
            Span::raw(format!(" Pane:{focus_desc}   ")),
        ];

        match model.focused_pane {
            FocusedPane::Extensions => {
                if !model.extensions.is_empty() {
                    action_spans.push(Span::styled(
                        "[a/Enter]",
                        Style::default().fg(Color::Yellow),
                    ));
                    action_spans.push(Span::raw(" Toggle Trust   "));
                }
                action_spans.push(Span::styled("[↑/k ↓/j]", Style::default().fg(Color::Cyan)));
                action_spans.push(Span::raw(" Move   "));
            }
            FocusedPane::BrokenLinks => {
                if !model.broken_links().is_empty() {
                    action_spans.push(Span::styled(
                        "[Enter/e]",
                        Style::default().fg(Color::Yellow),
                    ));
                    action_spans.push(Span::raw(" Edit   "));
                }
                action_spans.push(Span::styled("[d]", Style::default().fg(Color::Cyan)));
                action_spans.push(Span::raw(" Desk   "));
                action_spans.push(Span::styled("[↑/k ↓/j]", Style::default().fg(Color::Cyan)));
                action_spans.push(Span::raw(" Move   "));
            }
            FocusedPane::Checks | FocusedPane::ExtensionPanes => {
                action_spans.push(Span::styled("[↑/k ↓/j]", Style::default().fg(Color::Cyan)));
                action_spans.push(Span::raw(" Move   "));
                action_spans.push(Span::styled("[d]", Style::default().fg(Color::Cyan)));
                action_spans.push(Span::raw(" Desk   "));
            }
            FocusedPane::OutputPreview => {
                action_spans.push(Span::styled("[↑/k ↓/j]", Style::default().fg(Color::Cyan)));
                action_spans.push(Span::raw(" Scroll   "));
                action_spans.push(Span::styled("[d]", Style::default().fg(Color::Cyan)));
                action_spans.push(Span::raw(" Desk   "));
            }
        }

        action_spans.push(Span::styled("[c]", Style::default().fg(Color::Cyan)));
        action_spans.push(Span::raw(" Check   "));
        action_spans.push(Span::styled("[f]", Style::default().fg(Color::Cyan)));
        action_spans.push(Span::raw(" Fix   "));
        action_spans.push(Span::styled("[l]", Style::default().fg(Color::Cyan)));
        action_spans.push(Span::raw(" Links   "));
        action_spans.push(Span::styled("[q/Esc]", Style::default().fg(Color::Cyan)));
        action_spans.push(Span::raw(" Quit"));

        lines.push(Line::from(action_spans));

        if let Some(msg) = &model.message {
            let is_busy = model.is_checking || model.is_checking_links || model.is_fixing;
            let status_prefix = if is_busy {
                format!("status: {} ", spinner(model.tick))
            } else {
                "status: ".to_string()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    status_prefix,
                    if is_busy {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
                Span::styled(msg, Style::default().fg(Color::White)),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}
