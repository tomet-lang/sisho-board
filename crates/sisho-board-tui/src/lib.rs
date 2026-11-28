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

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use sisho_board_core::{
    ExtensionItem, FocusedPane, Model, TrustState, extension, resolve_editor, trust,
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
                                let target_arg = model.launch_target_arg();
                                let envs = extension::extension_dir_envs(&word);
                                model.close_launcher();
                                screen.execute_handover(|| {
                                    let _ = sisho_board_run::Step::new(&word, &path)
                                        .envs(envs)
                                        .arg(&target_arg)
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

            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    break;
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
                KeyCode::Char('a') => {
                    if model.focused_pane == FocusedPane::Extensions {
                        model.request_trust_prompt();
                    }
                }
                KeyCode::Enter | KeyCode::Char('e') => match model.focused_pane {
                    FocusedPane::Extensions if key.code == KeyCode::Enter => {
                        model.request_trust_prompt();
                    }
                    FocusedPane::BrokenLinks => {
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
                    FocusedPane::Checks
                    | FocusedPane::Health
                    | FocusedPane::ExtensionPanes => {
                        model.focused_pane = FocusedPane::OutputPreview;
                    }
                    _ => {}
                },
                KeyCode::Char('x') => {
                    model.open_launcher();
                }
                KeyCode::Tab => {
                    model.next_pane();
                }
                KeyCode::BackTab => {
                    model.prev_pane();
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
                _ => {}
            }
        }
    }

    Ok(())
}
