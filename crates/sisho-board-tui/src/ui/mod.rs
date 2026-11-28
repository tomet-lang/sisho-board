//! Ratatui rendering for `sisho-board` using the shared `sisho-tui` shell.

mod launcher;
mod layout_editor;
mod panes;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph, Wrap},
};
use sisho_board_core::config::{BoardView, PaneId, PaneSlot};
use sisho_board_core::layout_editor::{EditorColumn, EditorConfirm};
use sisho_board_core::model::Model;
use sisho_board_run::{Outcome, Status};
use sisho_tui::{FooterBar, HeaderBar, KeyHint, ShellLayout, StatusLine, TabItem, Theme};

const VIEW_KEYS: [&str; 9] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];
const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Render the complete `sisho-board` interface inside the shared `sisho-tui` shell.
pub fn draw(f: &mut Frame, model: &Model) {
    let theme = Theme::sisho();
    let shell = ShellLayout::split(f.area());

    let active_idx = model.active_view_index();
    let tabs = model.board_config.views.iter().enumerate().map(|(idx, view)| {
        let key = VIEW_KEYS.get(idx).copied().unwrap_or("·");
        TabItem::new(key, view.name.as_str(), idx == active_idx)
    });

    HeaderBar::new("sisho-board")
        .context("Project Status")
        .tabs(tabs)
        .render(f, shell.header, &theme);

    draw_board(f, shell.body, model, &theme);
    if model.is_launcher_open {
        launcher::draw_launcher_modal(f, shell.body, model, &theme);
    } else if model.layout_editor.is_open {
        layout_editor::draw_layout_editor_modal(f, shell.body, model, &theme);
    }
    draw_footer(f, shell.footer, model, &theme);
}

fn draw_board(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let view = model.active_view();
    match (view.left.is_empty(), view.right.is_empty()) {
        (true, true) => {
            draw_empty_view(f, area, view, theme);
        }
        (false, true) => {
            draw_pane_column(f, area, &view.left, model, theme);
        }
        (true, false) => {
            draw_pane_column(f, area, &view.right, model, theme);
        }
        (false, false) => {
            let left_ratio = view.left_ratio.clamp(20, 80);
            let right_ratio = 100u16.saturating_sub(left_ratio);
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(left_ratio),
                    Constraint::Percentage(right_ratio),
                ])
                .split(area);

            draw_pane_column(f, columns[0], &view.left, model, theme);
            draw_pane_column(f, columns[1], &view.right, model, theme);
        }
    }
}

fn draw_empty_view(f: &mut Frame, area: Rect, view: &BoardView, theme: &Theme) {
    let title = format!(" {} ", view.name);
    let msg = Paragraph::new(Line::from(Span::styled(
        "This view has no panes. Press [v] to customize layout.",
        Style::default().fg(theme.muted),
    )))
    .block(theme.pane_block(title, false))
    .wrap(Wrap { trim: true });
    f.render_widget(msg, area);
}

fn draw_pane_column(
    f: &mut Frame,
    area: Rect,
    slots: &[PaneSlot],
    model: &Model,
    theme: &Theme,
) {
    if slots.is_empty() || area.height == 0 || area.width == 0 {
        return;
    }
    let constraints: Vec<Constraint> = slots
        .iter()
        .map(|slot| Constraint::Fill(slot.weight.max(1)))
        .collect();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    for (slot, row_area) in slots.iter().zip(rows.iter()) {
        draw_pane_slot(f, *row_area, &slot.pane, model, theme);
    }
}

fn draw_pane_slot(f: &mut Frame, area: Rect, pane: &PaneId, model: &Model, theme: &Theme) {
    match pane {
        PaneId::Checks => panes::draw_checks(f, area, model, theme),
        PaneId::Health => panes::draw_health(f, area, model, theme),
        PaneId::Extensions => panes::draw_tools_and_extensions(f, area, model, theme),
        PaneId::BrokenLinks => panes::draw_broken_links(f, area, model, theme),
        PaneId::ExtensionPanes => panes::draw_extension_panes(f, area, model, theme),
        PaneId::OutputPreview => panes::draw_output_preview(f, area, model, theme),
        PaneId::Extension(word) => {
            panes::draw_standalone_extension_pane(f, area, word, model, theme);
        }
    }
}

pub(super) fn cursor_style(is_selected: bool, theme: &Theme) -> Style {
    if is_selected {
        Style::default().fg(theme.primary)
    } else {
        Style::default()
    }
}

pub(super) fn board_pane_block(
    title: impl Into<String>,
    is_focused: bool,
    pane_active: bool,
    theme: &Theme,
) -> Block<'static> {
    let mut title = title.into();
    if is_focused && pane_active {
        let trimmed = title.trim_end();
        title = format!("{trimmed} [ACTIVE] ");
        return theme
            .pane_block(title, true)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(theme.warning))
            .title_style(
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            );
    }
    theme.pane_block(title, is_focused)
}

pub(super) fn spinner(tick: usize) -> &'static str {
    SPINNER[tick % SPINNER.len()]
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

    if model.layout_editor.is_open {
        let (hints, status) = if model.layout_editor.rename_buffer.is_some() {
            (
                vec![
                    KeyHint::new("Type", "Name"),
                    KeyHint::new("Enter", "Save"),
                    KeyHint::new("Esc", "Cancel"),
                ],
                StatusLine::info("Renaming active board view..."),
            )
        } else if let Some(confirm) = model.layout_editor.confirm_action {
            let msg = match confirm {
                EditorConfirm::DeleteView => {
                    format!("Delete view '{}'?", model.active_view().name)
                }
                EditorConfirm::ResetDefault => "Reset layout to default?".to_string(),
            };
            (
                vec![
                    KeyHint::new("y", "Confirm"),
                    KeyHint::new("n/Esc", "Cancel"),
                ],
                StatusLine::info(msg),
            )
        } else if model.layout_editor.grabbed {
            (
                vec![
                    KeyHint::new("←↑↓→", "Move Pane"),
                    KeyHint::new("Space/Enter", "Drop"),
                    KeyHint::new("+/-", "Size"),
                    KeyHint::new("Esc", "Release"),
                ],
                StatusLine::info("Moving pane: use arrow keys to position, Space/Enter to drop."),
            )
        } else if model.layout_editor.column == EditorColumn::Available {
            (
                vec![
                    KeyHint::new("←↑↓→", "Cursor"),
                    KeyHint::new("Space/Enter", "Add & Grab"),
                    KeyHint::new("u", "Undo"),
                    KeyHint::new("Esc/v", "Close"),
                ],
                StatusLine::info(format!(
                    "Layout Editor: {} (Available)",
                    model.active_view().name
                )),
            )
        } else {
            let col_name = match model.layout_editor.column {
                EditorColumn::Left => "Left",
                EditorColumn::Right => "Right",
                EditorColumn::Available => unreachable!(),
            };
            (
                vec![
                    KeyHint::new("←↑↓→", "Cursor"),
                    KeyHint::new("Space/Enter", "Grab"),
                    KeyHint::new("+/-", "Size"),
                    KeyHint::new("Del", "Hide"),
                    KeyHint::new("</>", "Split"),
                    KeyHint::new("n/R/D", "View"),
                    KeyHint::new("u", "Undo"),
                    KeyHint::new("Esc/v", "Close"),
                ],
                StatusLine::info(format!(
                    "Layout Editor: {} ({col_name})",
                    model.active_view().name
                )),
            )
        };
        FooterBar::new()
            .hints(&hints)
            .status(Some(status))
            .render(f, area, theme);
        return;
    }

    let hints = if model.pane_active {
        let mut active_hints = vec![KeyHint::new("Esc", "Panes")];
        match &model.focused_pane {
            PaneId::Extensions => {
                if !model.extensions.is_empty() {
                    active_hints.push(KeyHint::new("a/Enter", "Change Trust"));
                }
                active_hints.push(KeyHint::new("j/k", "Move"));
            }
            PaneId::BrokenLinks => {
                if !model.broken_links().is_empty() {
                    active_hints.push(KeyHint::new("Enter/e", "Edit"));
                }
                active_hints.push(KeyHint::new("j/k", "Move"));
            }
            PaneId::Checks
            | PaneId::Health
            | PaneId::ExtensionPanes
            | PaneId::Extension(_) => {
                active_hints.push(KeyHint::new("j/k", "Move"));
            }
            PaneId::OutputPreview => {
                active_hints.push(KeyHint::new("j/k", "Scroll"));
            }
        }
        active_hints.push(KeyHint::new("c", "Check"));
        active_hints.push(KeyHint::new("f", "Fix"));
        active_hints.push(KeyHint::new("l", "Links"));
        active_hints.push(KeyHint::new("q", "Quit"));
        active_hints
    } else {
        vec![
            KeyHint::new("←↑↓→", "Select Pane"),
            KeyHint::new("Enter", "Explore"),
            if model.board_config.views.len() > 1 {
                KeyHint::new("Tab/1-9", "View")
            } else {
                KeyHint::new("Tab", "View")
            },
            KeyHint::new("x", "Launch"),
            KeyHint::new("v", "Layout"),
            KeyHint::new("c", "Check"),
            KeyHint::new("f", "Fix"),
            KeyHint::new("l", "Links"),
            KeyHint::new("q", "Quit"),
        ]
    };

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

pub(super) fn footer_summary(model: &Model) -> String {
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
