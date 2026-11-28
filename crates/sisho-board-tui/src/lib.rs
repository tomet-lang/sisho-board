//! Ratatui terminal interface and interactive event loop for `sisho-board`.
//!
//! Two-tier design:
//! - **Board**: Status of tools, extensions (with explicit user trust via `sisho`),
//!   extension board panes (`--sisho-board-pane`), integrity check outcomes, and broken links.
//! - **Counter**: Action prompt bar to run check, fix, check-links, manage extension trust, or launch TUI extensions.

pub mod screen;
pub mod ui;

#[cfg(test)]
mod render_tests;

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use sisho_board_core::{
    EditorColumn, ExtensionItem, Model, PaneId, TrustState, extension, resolve_editor, trust,
};

pub use screen::Screen;

/// Run the interactive board TUI in `dir`.
pub fn run(dir: PathBuf) -> anyhow::Result<()> {
    let sisho_program = trust::default_sisho_program();
    let extensions = extension::discover(&sisho_program);
    run_with_extensions(dir, extensions)
}

/// Run the interactive board TUI in `dir` with pre-scanned `extensions`.
pub fn run_with_extensions(dir: PathBuf, extensions: Vec<ExtensionItem>) -> anyhow::Result<()> {
    let model = Model::new(dir, extensions);
    run_with_model(model)
}

/// Run the interactive board TUI with an initialized [`Model`].
pub fn run_with_model(mut model: Model) -> anyhow::Result<()> {
    let mut screen = Screen::enter()?;

    loop {
        model.poll_background();
        screen.terminal.draw(|f| ui::draw(f, &model))?;

        if event::poll(Duration::from_millis(80))? {
            let Event::Key(key) = event::read()? else {
                continue;
            };

            let is_ctrl_c = key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'));
            if is_ctrl_c {
                break;
            }

            if model.pending_trust.is_some() {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        model.apply_pending_trust(TrustState::Allowed);
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        model.apply_pending_trust(TrustState::Denied);
                    }
                    KeyCode::Char('u') | KeyCode::Char('U') => {
                        model.apply_pending_trust(TrustState::Untrusted);
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        model.pending_trust = None;
                        model.message = Some("Trust change cancelled.".to_string());
                    }
                    _ => {}
                }
                continue;
            }

            if model.confirm_fix {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        model.run_fixes();
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        model.confirm_fix = false;
                        model.message = Some("Fix cancelled.".to_string());
                    }
                    _ => {}
                }
                continue;
            }

            if model.is_launcher_open {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('x') => {
                        model.close_launcher();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        model.move_launcher_selection_down();
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        model.move_launcher_selection_up();
                    }
                    KeyCode::Char('a') => {
                        model.request_launcher_trust_prompt();
                    }
                    KeyCode::Enter => {
                        if let Some(ext) = model.selected_launcher_extension() {
                            if ext.is_tui_launchable() {
                                let word = ext.word.clone();
                                let path = ext.path.clone();
                                let envs = extension::extension_dir_envs(&word);
                                model.close_launcher();
                                screen.execute_handover(|| {
                                    let _ = sisho_board_run::Step::new(&word, &path)
                                        .current_dir(&model.dir)
                                        .envs(envs)
                                        .run_inherited();
                                })?;
                                model.refresh_tools();
                                model.query_allowed_extensions();
                                model.run_checks();
                                model.run_check_links();
                                model.run_health();
                            } else if ext.trust == TrustState::Untrusted {
                                model.request_launcher_trust_prompt();
                            }
                        }
                    }
                    _ => {}
                }
                continue;
            }

            if model.layout_editor.is_open {
                handle_layout_editor_key(&mut model, key);
                continue;
            }

            match key.code {
                KeyCode::Char('q') => {
                    break;
                }
                KeyCode::Tab => {
                    model.next_view();
                }
                KeyCode::BackTab => {
                    model.prev_view();
                }
                KeyCode::Char('[') => {
                    model.prev_view();
                }
                KeyCode::Char(']') => {
                    model.next_view();
                }
                KeyCode::Char(c @ '1'..='9') => {
                    let idx = (c as usize) - ('1' as usize);
                    model.select_view(idx);
                }
                KeyCode::Char('c') => {
                    model.run_checks();
                    model.run_health();
                    model.query_allowed_extensions();
                }
                KeyCode::Char('f') => {
                    if model.is_fixing || model.is_checking {
                        model.message = Some("Operations already in progress.".to_string());
                    } else {
                        model.pending_trust = None;
                        model.confirm_fix = true;
                    }
                }
                KeyCode::Char('l') => {
                    model.run_check_links();
                }
                KeyCode::Char('x') => {
                    model.open_launcher();
                }
                KeyCode::Char('v') => {
                    model.open_layout_editor();
                }
                _ if !model.pane_active => match key.code {
                    KeyCode::Esc => {
                        break;
                    }
                    KeyCode::Up => {
                        model.move_pane_up();
                    }
                    KeyCode::Down => {
                        model.move_pane_down();
                    }
                    KeyCode::Left => {
                        model.move_pane_left();
                    }
                    KeyCode::Right => {
                        model.move_pane_right();
                    }
                    KeyCode::Enter => {
                        model.enter_pane();
                    }
                    _ => {}
                },
                _ => match key.code {
                    KeyCode::Esc => {
                        model.exit_pane();
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        model.move_selection_up();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        model.move_selection_down();
                    }
                    KeyCode::PageUp => {
                        model.scroll_preview_up(10);
                    }
                    KeyCode::PageDown => {
                        model.scroll_preview_down(10);
                    }
                    KeyCode::Home => {
                        model.preview_scroll = 0;
                    }
                    KeyCode::Char('a') => {
                        if model.focused_pane == PaneId::Extensions {
                            model.request_trust_prompt();
                        }
                    }
                    KeyCode::Enter | KeyCode::Char('e') => match &model.focused_pane {
                        PaneId::Extensions if key.code == KeyCode::Enter => {
                            model.request_trust_prompt();
                        }
                        PaneId::BrokenLinks => {
                            if let Some(link) = model.selected_broken_link_item() {
                                let source_path = model.dir.join(&link.source);
                                let line = link.at.map(|p| p.line);
                                let column = link.at.map(|p| p.column);
                                let step =
                                    resolve_editor(&source_path, line, column, &model.extensions);
                                screen.execute_handover(|| {
                                    let _ = step.run_inherited();
                                })?;
                                model.run_check_links();
                            }
                        }
                        PaneId::Checks
                        | PaneId::Health
                        | PaneId::ExtensionPanes
                        | PaneId::Extension(_) => {
                            if model.active_view().contains(&PaneId::OutputPreview) {
                                model.focused_pane = PaneId::OutputPreview;
                            }
                        }
                        PaneId::Extensions | PaneId::OutputPreview => {}
                    },
                    _ => {}
                },
            }
        }
    }

    Ok(())
}

fn handle_layout_editor_key(model: &mut Model, key: KeyEvent) {
    if model.layout_editor.confirm_action.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => model.confirm_editor_action(),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                model.cancel_editor_action();
            }
            _ => {}
        }
        return;
    }

    if model.layout_editor.rename_buffer.is_some() {
        match key.code {
            KeyCode::Enter => model.commit_rename_view(),
            KeyCode::Esc => model.cancel_rename_view(),
            KeyCode::Backspace => {
                if let Some(buf) = &mut model.layout_editor.rename_buffer {
                    buf.pop();
                }
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if let Some(buf) = &mut model.layout_editor.rename_buffer {
                    buf.push(c);
                }
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Esc => {
            if model.layout_editor.grabbed {
                model.cancel_grab();
            } else {
                model.close_layout_editor();
            }
        }
        KeyCode::Char('v') => model.close_layout_editor(),
        KeyCode::Up | KeyCode::Char('k') => model.move_cursor_up(),
        KeyCode::Down | KeyCode::Char('j') => model.move_cursor_down(),
        KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => model.move_column_left(),
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => model.move_column_right(),
        KeyCode::Char(' ') | KeyCode::Enter => model.toggle_grab(),
        KeyCode::Delete | KeyCode::Backspace => {
            if !model.layout_editor.grabbed && model.layout_editor.column != EditorColumn::Available
            {
                model.remove_selected_pane();
            }
        }
        KeyCode::Char('+') | KeyCode::Char('=') => model.adjust_selected_pane_weight(5),
        KeyCode::Char('-') | KeyCode::Char('_') => model.adjust_selected_pane_weight(-5),
        KeyCode::Char('<') => model.adjust_left_ratio(-5),
        KeyCode::Char('>') => model.adjust_left_ratio(5),
        KeyCode::Char('[') => model.prev_view(),
        KeyCode::Char(']') => model.next_view(),
        KeyCode::Char('n') => model.add_view(),
        KeyCode::Char('R') => model.start_rename_view(),
        KeyCode::Char('D') => model.request_delete_view(),
        KeyCode::Char('r') => model.request_reset_default(),
        KeyCode::Char('u') => model.undo_layout_edit(),
        _ => {}
    }
}
