//! The operating dashboard and board for the Tomet ecosystem (`sisho-board`).
//!
//! Two-tier design:
//! - **Board**: Status of tools, extensions (with explicit user trust via `~/.config/sisho/board.json`),
//!   extension board panes (`--sisho-board-pane`), integrity check outcomes, and broken links.
//! - **Counter**: Action prompt bar to run check, fix, check-links, toggle extension trust, or open desk.

pub mod editor;
pub mod extension;
pub mod health;
pub mod links;
pub mod model;
pub mod runner;
pub mod screen;
pub mod tools;
pub mod trust;
pub mod ui;

#[cfg(test)]
mod model_tests;
#[cfg(test)]
mod render_tests;

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};

pub use editor::resolve_editor;
pub use model::{ExtensionItem, FocusedPane, Model, PreviewSource};
pub use screen::Screen;
pub use trust::{TrustConfig, TrustState};

/// Run the interactive board TUI in `dir`.
pub fn run(dir: PathBuf) -> anyhow::Result<()> {
    let extensions = extension::scan();
    run_with_extensions(dir, extensions)
}

/// Run the interactive board TUI in `dir` with pre-scanned `extensions`.
pub fn run_with_extensions(dir: PathBuf, extensions: Vec<ExtensionItem>) -> anyhow::Result<()> {
    let mut screen = Screen::enter()?;
    let mut model = Model::new(dir, extensions);

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
                        model.confirm_fix = true;
                    }
                }
                KeyCode::Char('l') => {
                    model.run_check_links();
                }
                KeyCode::Char('a') => {
                    if model.focused_pane == FocusedPane::Extensions {
                        model.toggle_selected_extension_trust();
                    }
                }
                KeyCode::Enter | KeyCode::Char('e') => match model.focused_pane {
                    FocusedPane::Extensions if key.code == KeyCode::Enter => {
                        model.toggle_selected_extension_trust();
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
                KeyCode::Char('d') => {
                    let desk_ext = model
                        .extensions
                        .iter()
                        .find(|e| e.word == "desk" && e.is_active);
                    match desk_ext {
                        Some(desk) if desk.trust == TrustState::Allowed => {
                            let desk_path = desk.path.clone();
                            let target_arg = if model.focused_pane == FocusedPane::BrokenLinks {
                                if let Some(link) = model.selected_broken_link_item() {
                                    model.dir.join(&link.source)
                                } else {
                                    model.dir.clone()
                                }
                            } else {
                                model.dir.clone()
                            };
                            screen.execute_handover(|| {
                                let _ = runner::Step::new("desk", &desk_path)
                                    .arg(&target_arg)
                                    .run_inherited();
                            })?;
                            model.refresh_tools();
                            model.query_allowed_extensions();
                            model.run_checks();
                            model.run_check_links();
                            model.run_health();
                        }
                        Some(_) => {
                            model.message = Some(
                                "sisho-desk is not allowed yet (focus Tools & Extensions and press [a] to allow)."
                                    .to_string(),
                            );
                        }
                        None => {
                            model.message =
                                Some("sisho-desk extension is not installed on PATH".to_string());
                        }
                    }
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
