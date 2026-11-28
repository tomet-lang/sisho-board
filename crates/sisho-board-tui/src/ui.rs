//! Ratatui rendering for `sisho-board` using the shared `sisho-tui` shell.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
};
use sisho_board_core::model::{FocusedPane, Model, PreviewSource, StepSlot};
use sisho_board_core::tools::Verdict;
use sisho_board_core::trust::TrustState;
use sisho_board_run::{Outcome, Status};
use sisho_board_sdk::BoardStatus;
use sisho_tui::{FooterBar, HeaderBar, KeyHint, ModalFrame, ShellLayout, StatusLine, Theme};

/// Render the complete `sisho-board` interface inside the shared `sisho-tui` shell.
pub fn draw(f: &mut Frame, model: &Model) {
    let theme = Theme::sisho();
    let shell = ShellLayout::split(f.area());

    HeaderBar::new("sisho-board")
        .context("Project Status")
        .render(f, shell.header, &theme);

    draw_board(f, shell.body, model, &theme);
    if model.is_launcher_open {
        draw_launcher_modal(f, shell.body, model, &theme);
    }
    draw_footer(f, shell.footer, model, &theme);
}

fn draw_board(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    draw_left_column(f, columns[0], model, theme);
    draw_right_column(f, columns[1], model, theme);
}

fn draw_left_column(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    draw_tools_and_extensions(f, rows[0], model, theme);
    draw_broken_links(f, rows[1], model, theme);
}

fn cursor_style(is_selected: bool, theme: &Theme) -> Style {
    if is_selected {
        Style::default().fg(theme.primary)
    } else {
        Style::default()
    }
}

fn board_status_icon(status: BoardStatus, theme: &Theme) -> (&'static str, Style) {
    match status {
        BoardStatus::Pass => ("✓ ", Style::default().fg(theme.success)),
        BoardStatus::Warn => (
            "! ",
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ),
        BoardStatus::Fail => (
            "✗ ",
            Style::default()
                .fg(theme.error)
                .add_modifier(Modifier::BOLD),
        ),
        BoardStatus::Info => ("• ", Style::default().fg(theme.info)),
    }
}

fn draw_tools_and_extensions(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let is_focused = model.focused_pane == FocusedPane::Extensions;
    let mut items = Vec::new();

    // Tools
    items.push(ListItem::new(Line::from(vec![Span::styled(
        "Core Tools:",
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(theme.warning),
    )])));

    for tool in &model.tools {
        let (icon, icon_style) = match &tool.verdict {
            Verdict::Fine(_) => ("  ✓ ", Style::default().fg(theme.success)),
            Verdict::TooOld { .. } => ("  ✗ ", Style::default().fg(theme.error)),
            Verdict::Unreported { .. } => ("  - ", Style::default().fg(theme.muted)),
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
            .fg(theme.warning),
    )])));

    if model.extensions.is_empty() {
        items.push(ListItem::new(Line::from(vec![Span::styled(
            "  (none found on PATH)",
            Style::default().fg(theme.muted),
        )])));
    } else {
        for (idx, ext) in model.extensions.iter().enumerate() {
            let is_selected = is_focused && idx == model.selected_extension;
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
                    Span::styled(" [untrusted: a/Enter]", Style::default().fg(theme.warning)),
                ),
                TrustState::Denied => (
                    "✗ ",
                    Style::default().fg(theme.error),
                    Span::styled(" [denied: a/Enter]", Style::default().fg(theme.error)),
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

            if let Some(note) = &ext.note {
                header_spans.push(Span::raw(" "));
                header_spans.push(Span::styled(
                    format!("({note})"),
                    Style::default().fg(theme.warning),
                ));
            }

            items.push(ListItem::new(Line::from(header_spans)));

            if let Some(info) = &ext.info
                && !info.about.is_empty()
            {
                items.push(ListItem::new(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(&info.about, Style::default().fg(theme.inactive_tab_fg)),
                ])));
            }

            items.push(ListItem::new(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    ext.path.display().to_string(),
                    Style::default().fg(theme.muted),
                ),
            ])));
        }
    }

    let list = List::new(items).block(theme.pane_block(" Tools & Extensions ", is_focused));
    f.render_widget(list, area);
}

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn spinner(tick: usize) -> &'static str {
    SPINNER[tick % SPINNER.len()]
}

fn draw_broken_links(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let is_focused = model.focused_pane == FocusedPane::BrokenLinks;
    let broken = model.broken_links();
    let mut items = Vec::new();

    if model.is_checking_links && model.links_report.is_none() {
        let spin = spinner(model.tick);
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("  {spin} "), Style::default().fg(theme.primary)),
            Span::styled(
                "Scanning links in background...",
                Style::default().fg(theme.warning),
            ),
        ])));
    } else {
        if let Some(report) = &model.links_report {
            items.push(ListItem::new(Line::from(vec![Span::styled(
                format!(
                    "Scanned {} files ({} links checked)",
                    report.files_scanned, report.links_checked
                ),
                Style::default().fg(theme.muted),
            )])));
        }

        if broken.is_empty() {
            items.push(ListItem::new(Line::from(vec![Span::styled(
                "✓ No broken links found",
                Style::default().fg(theme.success),
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
                    Span::styled(cursor, cursor_style(is_selected, theme)),
                    Span::styled("✗ ", Style::default().fg(theme.error)),
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

    let list = List::new(items).block(theme.pane_block(title, is_focused));
    f.render_widget(list, area);
}

fn draw_right_column(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let cmd_count = if model.check_slots.is_empty() {
        sisho_board_core::tools::check_steps(&model.dir).len()
    } else {
        model.check_slots.len()
    };
    let checks_height = (cmd_count as u16) + 2;
    // 1 gauge line + 6 pillar rows + 2 border lines = 9 lines (or 3 lines while scanning).
    let health_height: u16 = if model.health_report.is_some() { 9 } else { 3 };

    if model.has_extension_panes() {
        let ext_lines = count_extension_pane_lines(model) + 2;
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Max(checks_height),
                Constraint::Max(health_height),
                Constraint::Max(ext_lines as u16),
                Constraint::Min(4),
            ])
            .split(area);

        draw_checks(f, rows[0], model, theme);
        draw_health(f, rows[1], model, theme);
        draw_extension_panes(f, rows[2], model, theme);
        draw_output_preview(f, rows[3], model, theme);
    } else {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Max(checks_height),
                Constraint::Max(health_height),
                Constraint::Min(4),
            ])
            .split(area);

        draw_checks(f, rows[0], model, theme);
        draw_health(f, rows[1], model, theme);
        draw_output_preview(f, rows[2], model, theme);
    }
}

fn draw_health(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let is_focused = model.focused_pane == FocusedPane::Health;
    let mut items = Vec::new();

    if let Some(report) = &model.health_report {
        let bar = format_gauge_bar(report.overall_percent, 10);
        let bar_color = match sisho_board_health::status_for_percent(report.overall_percent) {
            BoardStatus::Pass => theme.success,
            BoardStatus::Warn => theme.warning,
            BoardStatus::Fail => theme.error,
            BoardStatus::Info => theme.info,
        };
        items.push(ListItem::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                bar,
                Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled("Overall Readiness", Style::default().fg(Color::White)),
        ])));

        for (idx, pillar) in report.pillars().into_iter().enumerate() {
            let is_selected = is_focused && idx == model.selected_health_pillar;
            let cursor = if is_selected { "> " } else { "  " };
            let (status_icon, status_style) = board_status_icon(pillar.status, theme);

            items.push(ListItem::new(Line::from(vec![
                Span::styled(cursor, cursor_style(is_selected, theme)),
                Span::styled(status_icon, status_style),
                Span::styled(
                    &pillar.name,
                    if is_selected {
                        Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                    } else {
                        Style::default()
                    },
                ),
                Span::raw(": "),
                Span::styled(&pillar.value_summary, Style::default().fg(Color::White)),
            ])));
        }
    } else {
        let spin = spinner(model.tick);
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("  {spin} "), Style::default().fg(theme.primary)),
            Span::styled(
                "Measuring vault readiness & health...",
                Style::default().fg(theme.warning),
            ),
        ])));
    }

    let title = if model.is_checking_health && model.health_report.is_none() {
        " Vault Readiness & Health (scanning...) "
    } else {
        " Vault Readiness & Health "
    };

    let list = List::new(items).block(theme.pane_block(title, is_focused));
    f.render_widget(list, area);
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
    format!(
        "[{}{}] {:>3}%",
        "█".repeat(filled),
        "░".repeat(empty),
        clamped
    )
}

fn draw_extension_panes(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let is_focused = model.focused_pane == FocusedPane::ExtensionPanes;
    let panes = model.active_extension_panes();
    let mut items = Vec::new();
    let mut flat_idx = 0usize;

    for (word, pane) in &panes {
        if panes.len() > 1 {
            items.push(ListItem::new(Line::from(vec![Span::styled(
                format!("{} (sisho-{word}):", pane.title),
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            )])));
        }

        if let Some(gauge) = &pane.gauge {
            let bar = format_gauge_bar(gauge.percent, 10);
            let bar_color = if gauge.percent >= 80 {
                theme.success
            } else if gauge.percent >= 50 {
                theme.warning
            } else {
                theme.error
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
            let (status_icon, status_style) = board_status_icon(item.status, theme);

            items.push(ListItem::new(Line::from(vec![
                Span::styled(cursor, cursor_style(is_selected, theme)),
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

    let list = List::new(items).block(theme.pane_block(title, is_focused));
    f.render_widget(list, area);
}

fn draw_checks(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let is_focused = model.focused_pane == FocusedPane::Checks;
    let mut items = Vec::new();

    if model.check_slots.is_empty() {
        if model.is_checking {
            let spin = spinner(model.tick);
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {spin} "), Style::default().fg(theme.primary)),
                Span::styled(
                    "Running checks in background...",
                    Style::default().fg(theme.warning),
                ),
            ])));
        } else if model.is_fixing {
            let spin = spinner(model.tick);
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {spin} "), Style::default().fg(theme.primary)),
                Span::styled(
                    "Applying fixes in background...",
                    Style::default().fg(theme.warning),
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
                        Status::Passed => ("✓ ", Style::default().fg(theme.success)),
                        Status::Failed { .. } => (
                            "✗ ",
                            Style::default()
                                .fg(theme.error)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Status::NotFound => ("- ", Style::default().fg(theme.warning)),
                        Status::CouldNotStart { .. } => ("! ", Style::default().fg(theme.error)),
                        Status::Signalled { .. } => ("! ", Style::default().fg(Color::Magenta)),
                    };

                    let duration_str = format!("({:.2}s)", outcome.duration.as_secs_f64());
                    Line::from(vec![
                        Span::styled(cursor, cursor_style(is_selected, theme)),
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
                        Span::styled(duration_str, Style::default().fg(theme.muted)),
                    ])
                }
                StepSlot::Running { name } => {
                    let spin = spinner(model.tick.wrapping_add(i));
                    Line::from(vec![
                        Span::styled(cursor, cursor_style(is_selected, theme)),
                        Span::styled(format!("{spin} "), Style::default().fg(theme.primary)),
                        Span::styled(
                            name,
                            if is_selected {
                                Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                            } else {
                                Style::default()
                            },
                        ),
                        Span::raw(" "),
                        Span::styled("(running...)", Style::default().fg(theme.warning)),
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

    let list = List::new(items).block(theme.pane_block(title, is_focused));
    f.render_widget(list, area);
}

fn draw_output_preview(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let is_focused = model.focused_pane == FocusedPane::OutputPreview;
    let mut total_lines = 0;

    let content = match model.preview_source {
        PreviewSource::HealthPillar => {
            if let Some(pillar) = model.selected_health_pillar_metric() {
                let mut lines = Vec::new();
                lines.push(Line::from(vec![
                    Span::styled("Pillar: ", Style::default().fg(theme.muted)),
                    Span::styled(
                        format!("{} — {}", pillar.name, pillar.value_summary),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]));
                lines.push(Line::from(""));
                for l in pillar.detail.lines() {
                    lines.push(Line::from(l));
                }
                total_lines = lines.len();
                lines
            } else {
                vec![Line::from("Scanning vault readiness and health...")]
            }
        }
        PreviewSource::ExtensionItem => {
            if let Some(ext_ref) = model.selected_extension_pane_item() {
                let mut lines = Vec::new();
                lines.push(Line::from(vec![
                    Span::styled("Extension: ", Style::default().fg(theme.muted)),
                    Span::raw(format!("sisho-{} — {}", ext_ref.word, ext_ref.pane.title)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("Item: ", Style::default().fg(theme.muted)),
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
                        Style::default().fg(theme.muted),
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
                    Span::styled("Command: ", Style::default().fg(theme.muted)),
                    Span::raw(&outcome.command_line),
                ]));

                if !outcome.stdout.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "--- stdout ---",
                        Style::default().fg(theme.muted),
                    )));
                    for l in outcome.stdout.lines() {
                        lines.push(Line::from(l));
                    }
                }
                if !outcome.stderr.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "--- stderr ---",
                        Style::default().fg(theme.error),
                    )));
                    for l in outcome.stderr.lines() {
                        lines.push(Line::from(Span::styled(
                            l,
                            Style::default().fg(theme.error),
                        )));
                    }
                }

                if outcome.stdout.is_empty() && outcome.stderr.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "(no output produced)",
                        Style::default().fg(theme.muted),
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

    let paragraph = Paragraph::new(content)
        .scroll((scroll_y as u16, 0))
        .block(theme.pane_block(title, is_focused))
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}

fn draw_launcher_modal(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let inner = ModalFrame::new(" TUI Extensions ", 76, 72).render(f, area, theme);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let target_arg = model.launch_target_arg();
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Target: ", Style::default().fg(theme.muted)),
            Span::styled(
                target_arg.display().to_string(),
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

fn draw_footer(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    if let Some(prompt) = &model.pending_trust {
        let hints = [
            KeyHint::new("y", "Allow (execute)"),
            KeyHint::new("d", "Deny (block)"),
            KeyHint::new("u", "Untrusted"),
            KeyHint::new("n/Esc", "Cancel"),
        ];
        let status = StatusLine::info(format!(
            "Trust for 'sisho-{}' ({}):",
            prompt.word,
            prompt.canonical_path.display()
        ));
        FooterBar::new()
            .hints(&hints)
            .status(Some(status))
            .render(f, area, theme);
        return;
    }

    if model.confirm_fix {
        let hints = [
            KeyHint::new("y", "Yes, apply fixes"),
            KeyHint::new("n/Esc", "Cancel"),
        ];
        let status = StatusLine::info("Apply fixes across the project? (writes files)");
        FooterBar::new()
            .hints(&hints)
            .status(Some(status))
            .render(f, area, theme);
        return;
    }

    if model.is_launcher_open {
        let launcher_count = model.launcher_items().len();
        let hints = if launcher_count > 0 {
            vec![
                KeyHint::new("j/k", "Move"),
                KeyHint::new("Enter", "Launch"),
                KeyHint::new("a", "Change Trust"),
                KeyHint::new("Esc/x", "Close"),
            ]
        } else {
            vec![KeyHint::new("Esc/x", "Close")]
        };
        let status = StatusLine::info(format!(
            "TUI Extension Launcher ({launcher_count} available)"
        ));
        FooterBar::new()
            .hints(&hints)
            .status(Some(status))
            .summary(footer_summary(model))
            .render(f, area, theme);
        return;
    }

    let focus_desc = match model.focused_pane {
        FocusedPane::Checks => "Pane:Checks",
        FocusedPane::Health => "Pane:Health",
        FocusedPane::Extensions => "Pane:Extensions",
        FocusedPane::BrokenLinks => "Pane:Links",
        FocusedPane::ExtensionPanes => "Pane:ExtPane",
        FocusedPane::OutputPreview => "Pane:Preview",
    };

    let mut hints = vec![KeyHint::new("Tab", focus_desc)];

    match model.focused_pane {
        FocusedPane::Extensions => {
            if !model.extensions.is_empty() {
                hints.push(KeyHint::new("a/Enter", "Change Trust"));
            }
            hints.push(KeyHint::new("j/k", "Move"));
        }
        FocusedPane::BrokenLinks => {
            if !model.broken_links().is_empty() {
                hints.push(KeyHint::new("Enter/e", "Edit"));
            }
            hints.push(KeyHint::new("j/k", "Move"));
        }
        FocusedPane::Checks | FocusedPane::Health | FocusedPane::ExtensionPanes => {
            hints.push(KeyHint::new("j/k", "Move"));
        }
        FocusedPane::OutputPreview => {
            hints.push(KeyHint::new("j/k", "Scroll"));
        }
    }

    hints.push(KeyHint::new("x", "Launch"));
    hints.push(KeyHint::new("c", "Check"));
    hints.push(KeyHint::new("f", "Fix"));
    hints.push(KeyHint::new("l", "Links"));
    hints.push(KeyHint::new("q", "Quit"));

    let is_busy = model.is_checking || model.is_checking_links || model.is_fixing;
    let status = model.message.as_deref().map(|msg| {
        if is_busy {
            StatusLine::info(format!("{} {msg}", spinner(model.tick)))
        } else {
            classify_status_message(msg, &model.check_outcomes)
        }
    });

    FooterBar::new()
        .hints(&hints)
        .status(status)
        .summary(footer_summary(model))
        .render(f, area, theme);
}

fn classify_status_message<'a>(msg: &'a str, check_outcomes: &[Outcome]) -> StatusLine<'a> {
    if msg.starts_with("Failed")
        || msg.contains("error")
        || msg.contains("not allowed")
        || msg.contains("not installed")
    {
        StatusLine::error(msg)
    } else if msg == "Checks completed." {
        if check_outcomes
            .iter()
            .any(|outcome| !matches!(outcome.status, Status::Passed))
        {
            StatusLine::error(msg)
        } else {
            StatusLine::success(msg)
        }
    } else if msg.starts_with("Fixes applied") || msg.starts_with("Allowed ") {
        StatusLine::success(msg)
    } else {
        StatusLine::info(msg)
    }
}

fn footer_summary(model: &Model) -> String {
    if model.is_checking || model.is_checking_links || model.is_fixing {
        return format!("{} Running background tasks...", spinner(model.tick));
    }

    let readiness = model
        .health_report
        .as_ref()
        .map(|report| format!("Readiness {}%", report.overall_percent))
        .unwrap_or_else(|| "Readiness scanning...".to_string());
    let broken = model.broken_links().len();
    format!(
        "{readiness} · {} extensions · {broken} broken links",
        model.extensions.len()
    )
}
