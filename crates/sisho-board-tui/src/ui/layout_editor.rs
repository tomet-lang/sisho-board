//! Rendering for the Board Layout Editor modal.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
};
use sisho_board_core::config::PaneSlot;
use sisho_board_core::layout_editor::{EditorColumn, EditorConfirm};
use sisho_board_core::model::Model;
use sisho_tui::{ModalFrame, Theme};

use super::cursor_style;

pub(super) fn draw_layout_editor_modal(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let inner = ModalFrame::new(" Board Layout Editor ", 90, 80).render(f, area, theme);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(5)])
        .split(inner);

    draw_layout_editor_header(f, sections[0], model, theme);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(sections[1]);

    let view = model.active_view();
    draw_editor_slot_column(
        f,
        cols[0],
        " Left Column ",
        &view.left,
        model.layout_editor.column == EditorColumn::Left,
        model.layout_editor.selected,
        model.layout_editor.grabbed,
        theme,
    );
    draw_editor_slot_column(
        f,
        cols[1],
        " Right Column ",
        &view.right,
        model.layout_editor.column == EditorColumn::Right,
        model.layout_editor.selected,
        model.layout_editor.grabbed,
        theme,
    );
    draw_editor_available_column(f, cols[2], model, theme);
}

fn draw_layout_editor_header(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let view = model.active_view();
    let idx = model.active_view_index() + 1;
    let total = model.board_config.views.len();
    let left_ratio = view.left_ratio;
    let right_ratio = 100u16.saturating_sub(left_ratio);

    let bar_width = 16usize;
    let left_cells = ((left_ratio as usize) * bar_width + 50) / 100;
    let right_cells = bar_width.saturating_sub(left_cells);
    let split_bar = format!(
        "[{}{}]",
        "█".repeat(left_cells),
        "░".repeat(right_cells)
    );

    let line1 = Line::from(vec![
        Span::styled(
            format!("View {idx}/{total}: "),
            Style::default().fg(theme.muted),
        ),
        Span::styled(
            &view.name,
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  [[/]]", Style::default().fg(theme.muted)),
    ]);

    let line2 = Line::from(vec![
        Span::styled("Split: ", Style::default().fg(theme.muted)),
        Span::styled(split_bar, Style::default().fg(theme.secondary)),
        Span::raw(format!(" Left {left_ratio}% | Right {right_ratio}%")),
        Span::styled("  [</>]  ", Style::default().fg(theme.muted)),
        Span::styled(
            format!("Undo: {}", model.layout_editor.undo_stack.len()),
            Style::default().fg(theme.muted),
        ),
    ]);

    let line3 = if let Some(buf) = &model.layout_editor.rename_buffer {
        Line::from(vec![
            Span::styled(
                "Rename view: ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{buf}_"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::UNDERLINED),
            ),
        ])
    } else if let Some(confirm) = model.layout_editor.confirm_action {
        let prompt = match confirm {
            EditorConfirm::DeleteView => {
                format!("Delete view '{}'? [y] Yes · [n/Esc] Cancel", view.name)
            }
            EditorConfirm::ResetDefault => {
                "Reset layout to default? [y] Yes · [n/Esc] Cancel".to_string()
            }
        };
        Line::from(Span::styled(
            prompt,
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ))
    } else if model.layout_editor.grabbed {
        Line::from(Span::styled(
            "Moving pane: use arrow keys to position, [Space/Enter] to drop, [Esc] to release.",
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ))
    } else {
        Line::from(Span::styled(
            "Use arrow keys to navigate, [Space/Enter] to grab & move panes.",
            Style::default().fg(theme.muted),
        ))
    };

    let paragraph = Paragraph::new(vec![line1, line2, line3]);
    f.render_widget(paragraph, area);
}

#[allow(clippy::too_many_arguments)]
fn draw_editor_slot_column(
    f: &mut Frame,
    area: Rect,
    title: &str,
    slots: &[PaneSlot],
    is_active: bool,
    selected: usize,
    grabbed: bool,
    theme: &Theme,
) {
    let total_weight: u32 = slots.iter().map(|s| s.weight as u32).sum::<u32>().max(1);
    let items: Vec<ListItem> = if slots.is_empty() {
        vec![ListItem::new(Span::styled(
            "(empty)",
            Style::default().fg(theme.muted),
        ))]
    } else {
        slots
            .iter()
            .enumerate()
            .map(|(idx, slot)| {
                let is_selected = is_active && idx == selected;
                let is_grabbed = is_selected && grabbed;
                let (cursor, cursor_col_style) = if is_grabbed {
                    (
                        "» ",
                        Style::default()
                            .fg(theme.warning)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    (
                        if is_selected { "> " } else { "  " },
                        cursor_style(is_selected, theme),
                    )
                };
                let pct = ((slot.weight as u32) * 100 + total_weight / 2) / total_weight;
                let label_style = if is_grabbed {
                    Style::default()
                        .fg(theme.warning)
                        .add_modifier(Modifier::BOLD)
                } else if is_selected {
                    Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    Style::default()
                };
                let mut spans = vec![Span::styled(cursor, cursor_col_style)];
                if is_grabbed {
                    spans.push(Span::styled(
                        "[MOVING] ",
                        Style::default()
                            .fg(theme.warning)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
                spans.push(Span::styled(slot.pane.label(), label_style));
                spans.push(Span::styled(
                    format!(" w:{} ({pct}%)", slot.weight),
                    Style::default().fg(theme.secondary),
                ));
                ListItem::new(Line::from(spans))
            })
            .collect()
    };

    let list = List::new(items).block(theme.pane_block(title, is_active));
    f.render_widget(list, area);
}

fn draw_editor_available_column(f: &mut Frame, area: Rect, model: &Model, theme: &Theme) {
    let is_active = model.layout_editor.column == EditorColumn::Available;
    let available = model.available_panes();
    let items: Vec<ListItem> = if available.is_empty() {
        vec![ListItem::new(Span::styled(
            "(all panes placed)",
            Style::default().fg(theme.muted),
        ))]
    } else {
        available
            .iter()
            .enumerate()
            .map(|(idx, pane)| {
                let is_selected = is_active && idx == model.layout_editor.selected;
                let cursor = if is_selected { "> " } else { "  " };
                let badge = if pane.is_builtin() {
                    " [builtin]"
                } else {
                    " [ext]"
                };
                let label_style = if is_selected {
                    Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(vec![
                    Span::styled(cursor, cursor_style(is_selected, theme)),
                    Span::styled(pane.label(), label_style),
                    Span::styled(badge, Style::default().fg(theme.muted)),
                ]))
            })
            .collect()
    };

    let list = List::new(items).block(theme.pane_block(" Available ", is_active));
    f.render_widget(list, area);
}
