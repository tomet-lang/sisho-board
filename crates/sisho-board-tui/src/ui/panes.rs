//! Rendering for built-in and extension board panes.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
};
use sisho_board_core::config::PaneId;
use sisho_board_core::model::{FocusedPane, Model, PreviewSource, StepSlot};
use sisho_board_core::tools::Verdict;
use sisho_board_core::trust::TrustState;
use sisho_board_run::Status;
use sisho_board_sdk::{BoardGauge, BoardItem, BoardPane, BoardStatus};
use sisho_tui::Theme;

use super::{board_pane_block, cursor_style, spinner};

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

pub(super) fn draw_tools_and_extensions(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
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

    let list = List::new(items).block(board_pane_block(
        " Tools & Extensions ",
        is_focused,
        model.pane_active,
        theme,
    ));
    f.render_widget(list, area);
}

pub(super) fn draw_broken_links(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
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

    let list = List::new(items).block(board_pane_block(title, is_focused, model.pane_active, theme));
    f.render_widget(list, area);
}

pub(super) fn draw_health(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
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

    let list = List::new(items).block(board_pane_block(title, is_focused, model.pane_active, theme));
    f.render_widget(list, area);
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

fn push_board_gauge<'a>(items: &mut Vec<ListItem<'a>>, gauge: &'a BoardGauge, theme: &Theme) {
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

fn push_board_item<'a>(
    items: &mut Vec<ListItem<'a>>,
    item: &'a BoardItem,
    is_selected: bool,
    theme: &Theme,
) {
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
}

pub(super) fn draw_extension_panes(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let is_focused = model.focused_pane == FocusedPane::ExtensionPanes;
    let panes = model.active_extension_panes();
    let mut items = Vec::new();
    let mut flat_idx = 0usize;

    if panes.is_empty() {
        items.push(ListItem::new(Span::styled(
            "No active extension panes.",
            Style::default().fg(theme.muted),
        )));
    } else {
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
                push_board_gauge(&mut items, gauge, theme);
            }

            for item in &pane.items {
                let is_selected = is_focused && flat_idx == model.selected_ext_item;
                push_board_item(&mut items, item, is_selected, theme);
                flat_idx += 1;
            }
        }
    }

    let title = if let [(word, pane)] = panes.as_slice() {
        format!(" {} (sisho-{word}) ", pane.title)
    } else {
        " Extension Panes ".to_string()
    };

    let list = List::new(items).block(board_pane_block(title, is_focused, model.pane_active, theme));
    f.render_widget(list, area);
}

pub(super) fn draw_standalone_extension_pane(
    f: &mut Frame,
    area: Rect,
    word: &str,
    model: &Model,
    theme: &Theme,
) {
    let is_focused = matches!(&model.focused_pane, PaneId::Extension(w) if w == word);
    let ext = model.extensions.iter().find(|e| e.word == word);

    let (title, items) = match ext {
        None => (
            format!(" Ext: {word} "),
            vec![ListItem::new(Span::styled(
                format!("Extension 'sisho-{word}' not found on PATH."),
                Style::default().fg(theme.muted),
            ))],
        ),
        Some(ext) if !ext.is_active => {
            let note = ext
                .note
                .as_deref()
                .unwrap_or("Extension is inactive or shadowed.");
            (
                format!(" Ext: {word} "),
                vec![ListItem::new(Span::styled(
                    note,
                    Style::default().fg(theme.muted),
                ))],
            )
        }
        Some(ext) if ext.trust == TrustState::Untrusted => (
            format!(" Ext: {word} [untrusted] "),
            vec![ListItem::new(Span::styled(
                format!(
                    "sisho-{word} is untrusted. Focus Tools & Extensions and press [a/Enter] to allow."
                ),
                Style::default().fg(theme.warning),
            ))],
        ),
        Some(ext) if ext.trust == TrustState::Denied => (
            format!(" Ext: {word} [denied] "),
            vec![ListItem::new(Span::styled(
                format!(
                    "sisho-{word} is denied. Focus Tools & Extensions and press [a/Enter] to change trust."
                ),
                Style::default().fg(theme.error),
            ))],
        ),
        Some(ext) if ext.is_querying && ext.board_pane.is_none() => (
            format!(" Ext: {word} (querying...) "),
            vec![ListItem::new(Span::styled(
                format!("Querying sisho-{word} --sisho-board-pane..."),
                Style::default().fg(theme.muted),
            ))],
        ),
        Some(ext) => {
            if let Some(pane) = &ext.board_pane {
                let mut rows = Vec::new();
                if let Some(gauge) = &pane.gauge {
                    push_board_gauge(&mut rows, gauge, theme);
                }
                if let Some(summary) = &pane.summary {
                    for line in summary.lines() {
                        rows.push(ListItem::new(Span::styled(
                            format!("  {line}"),
                            Style::default().fg(theme.muted),
                        )));
                    }
                }
                let selected_idx = model.selected_standalone_ext_item_index(word);
                for (idx, item) in pane.items.iter().enumerate() {
                    let is_selected = is_focused && idx == selected_idx;
                    push_board_item(&mut rows, item, is_selected, theme);
                }
                if rows.is_empty() {
                    rows.push(ListItem::new(Span::styled(
                        "(no items reported)",
                        Style::default().fg(theme.muted),
                    )));
                }
                (format!(" {} (sisho-{word}) ", pane.title), rows)
            } else {
                (
                    format!(" Ext: {word} "),
                    vec![ListItem::new(Span::styled(
                        format!("sisho-{word} does not expose a board pane."),
                        Style::default().fg(theme.muted),
                    ))],
                )
            }
        }
    };

    let list = List::new(items).block(board_pane_block(title, is_focused, model.pane_active, theme));
    f.render_widget(list, area);
}

pub(super) fn draw_checks(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
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

    let list = List::new(items).block(board_pane_block(title, is_focused, model.pane_active, theme));
    f.render_widget(list, area);
}

pub(super) fn draw_output_preview(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let is_focused = model.focused_pane == FocusedPane::OutputPreview;
    let mut total_lines = 0;

    let content = match &model.preview_source {
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
                let lines = build_extension_preview_lines(
                    ext_ref.word,
                    ext_ref.pane,
                    Some(ext_ref.item),
                    theme,
                );
                total_lines = lines.len();
                lines
            } else {
                vec![Line::from("No extension item selected.")]
            }
        }
        PreviewSource::StandaloneExtensionItem(word) => {
            if let Some(pane) = model.standalone_extension_pane(word) {
                let item = model.selected_standalone_ext_item(word);
                let lines = build_extension_preview_lines(word, pane, item, theme);
                total_lines = lines.len();
                lines
            } else {
                vec![Line::from(format!("No board pane data for sisho-{word}."))]
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
        .block(board_pane_block(title, is_focused, model.pane_active, theme))
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}

fn build_extension_preview_lines<'a>(
    word: &str,
    pane: &'a BoardPane,
    item: Option<&'a BoardItem>,
    theme: &Theme,
) -> Vec<Line<'a>> {
    let mut lines = vec![Line::from(vec![
        Span::styled("Extension: ", Style::default().fg(theme.muted)),
        Span::raw(format!("sisho-{word} — {}", pane.title)),
    ])];

    if let Some(item) = item {
        lines.push(Line::from(vec![
            Span::styled("Item: ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{}: {}", item.label, item.value),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));

        if let Some(detail) = &item.detail {
            lines.push(Line::from(""));
            for l in detail.lines() {
                lines.push(Line::from(l));
            }
        } else if let Some(summary) = &pane.summary {
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
    } else if let Some(summary) = &pane.summary {
        lines.push(Line::from(""));
        for l in summary.lines() {
            lines.push(Line::from(l));
        }
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "(no additional detail)",
            Style::default().fg(theme.muted),
        )));
    }

    lines
}
