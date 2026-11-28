//! State model and background worker coordination for `sisho-board`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use sisho_board_health::health::{self, HealthReport, PillarMetric};
use sisho_board_health::links::{self, BrokenLink, Report};
use sisho_board_run::{Outcome, run_parallel_streaming};
use sisho_board_sdk::{BoardItem, BoardPane};
use sisho_sdk::ExtensionInfo;

use sisho_board_config::{
    BoardConfig, BoardView, DEFAULT_PANE_WEIGHT, MAX_LEFT_RATIO, MAX_PANE_WEIGHT,
    MIN_EDITOR_PANE_WEIGHT, MIN_LEFT_RATIO, PaneId, PaneSlot, resolve_config_path,
};
use crate::extension::{query_board_pane, query_info};
use crate::layout_editor::{EditorColumn, EditorConfirm, LayoutEditorState};
use crate::tools::{Tool, Verdict, check_steps, fix_steps};
use crate::trust::{TrustState, default_sisho_program, set_extension_trust};

/// An extension discovered on `PATH` and presented on the Board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionItem {
    /// Extension word (e.g. `"desk"` for `sisho-desk`).
    pub word: String,
    /// Full path to the executable on `PATH`.
    pub path: PathBuf,
    /// Canonicalized path to the executable (used as trust key by `sisho`).
    pub canonical_path: PathBuf,
    /// True if runnable as `sisho <word>` (not shadowed by a builtin).
    pub is_active: bool,
    /// Optional note such as `"shadowed by builtin"`.
    pub note: Option<String>,
    /// User trust state reported by `sisho` (`Untrusted`, `Allowed`, `Denied`).
    pub trust: TrustState,
    /// Metadata from `--sisho-extension-info` (populated only when `Allowed`).
    pub info: Option<ExtensionInfo>,
    /// Board pane from `--sisho-board-pane` (populated only when `Allowed`).
    pub board_pane: Option<BoardPane>,
    /// True while a background thread is querying this allowed extension.
    pub is_querying: bool,
}

impl ExtensionItem {
    /// Create a new `ExtensionItem` with [`TrustState::Untrusted`] and no queried data.
    pub fn new(
        word: impl Into<String>,
        path: impl Into<PathBuf>,
        is_active: bool,
        note: Option<String>,
    ) -> Self {
        let path = path.into();
        let canonical_path = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        Self {
            word: word.into(),
            path,
            canonical_path,
            is_active,
            note,
            trust: TrustState::Untrusted,
            info: None,
            board_pane: None,
            is_querying: false,
        }
    }

    /// True if this extension is active, allowed, and declares TUI capability.
    pub fn is_tui_launchable(&self) -> bool {
        self.is_active
            && self.word != "board"
            && self.trust == TrustState::Allowed
            && self.info.as_ref().is_some_and(|i| i.capabilities.tui)
    }
}

/// Pending confirmation prompt for changing an extension's trust state via `sisho`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTrustPrompt {
    /// Extension word (e.g. `"desk"`).
    pub word: String,
    /// Canonicalized binary path passed to `sisho --allow|--deny|--untrust`.
    pub canonical_path: PathBuf,
    /// Current trust state before the user confirms a change.
    pub current_trust: TrustState,
}

/// Status of a core ecosystem tool (`tomet`, `twrit`, `catasto`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolStatus {
    pub name: &'static str,
    pub version: Option<String>,
    pub verdict: Verdict,
}

/// A slot representing a check or fix step in progress or completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepSlot {
    Running { name: String },
    Done(Outcome),
}

impl StepSlot {
    pub fn name(&self) -> &str {
        match self {
            Self::Running { name } => name,
            Self::Done(outcome) => &outcome.name,
        }
    }

    pub fn outcome(&self) -> Option<&Outcome> {
        match self {
            Self::Done(outcome) => Some(outcome),
            Self::Running { .. } => None,
        }
    }
}

/// Alias for [`PaneId`] representing the currently focused pane on the Board.
pub type FocusedPane = PaneId;

/// Which pane provides the content currently shown in the Output Preview pane.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PreviewSource {
    #[default]
    CheckStep,
    HealthPillar,
    ExtensionItem,
    StandaloneExtensionItem(String),
}

/// Messages sent from background worker threads to the model.
pub enum WorkerMessage {
    CheckStepDone {
        index: usize,
        outcome: Outcome,
    },
    ChecksDone(Vec<Outcome>),
    LinksDone(Result<Report, String>),
    HealthDone(Box<HealthReport>),
    FixStepDone {
        index: usize,
        outcome: Outcome,
    },
    FixesDone(Vec<Outcome>),
    ExtensionQueried {
        word: String,
        info: Option<ExtensionInfo>,
        board_pane: Option<BoardPane>,
    },
}

/// Reference to a single item inside an allowed extension's [`BoardPane`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionPaneItemRef<'a> {
    pub word: &'a str,
    pub pane: &'a BoardPane,
    pub item: &'a BoardItem,
}

/// The state of the board and counter.
pub struct Model {
    pub dir: PathBuf,
    pub sisho_program: PathBuf,
    pub board_config_path: Option<PathBuf>,
    pub board_config: BoardConfig,
    pub tools: Vec<ToolStatus>,
    pub extensions: Vec<ExtensionItem>,
    pub check_slots: Vec<StepSlot>,
    pub check_outcomes: Vec<Outcome>,
    pub links_report: Option<Report>,
    pub health_report: Option<HealthReport>,
    pub selected_extension: usize,
    pub selected_step: usize,
    pub selected_health_pillar: usize,
    pub selected_broken_link: usize,
    pub selected_ext_item: usize,
    pub selected_standalone_ext_items: BTreeMap<String, usize>,
    pub preview_scroll: usize,
    pub focused_pane: PaneId,
    pub pane_active: bool,
    pub preview_source: PreviewSource,
    pub confirm_fix: bool,
    pub pending_trust: Option<PendingTrustPrompt>,
    pub is_launcher_open: bool,
    pub launcher_selected: usize,
    pub layout_editor: LayoutEditorState,
    pub message: Option<String>,
    pub is_checking: bool,
    pub is_checking_links: bool,
    pub is_checking_health: bool,
    pub is_fixing: bool,
    pub tick: usize,
    pub sender: Sender<WorkerMessage>,
    pub receiver: Receiver<WorkerMessage>,
}

impl Model {
    /// Create a new `Model` using the default UI board config path
    /// (`$SISHO_CONFIG_DIR/board.json` or `$XDG_CONFIG_HOME/sisho/board/board.json`)
    /// and default `sisho` executable (`SISHO_BIN` or `"sisho"`).
    pub fn new(dir: PathBuf, extensions: Vec<ExtensionItem>) -> Self {
        Self::with_config_path(dir, extensions, resolve_config_path())
    }

    /// Create a new `Model` with an explicit or overridden `board.json` path.
    pub fn with_config_path(
        dir: PathBuf,
        extensions: Vec<ExtensionItem>,
        board_config_path: Option<PathBuf>,
    ) -> Self {
        Self::with_options(dir, extensions, board_config_path, default_sisho_program())
    }

    /// Create a new `Model` with explicit `board.json` path and `sisho` executable.
    pub fn with_options(
        dir: PathBuf,
        extensions: Vec<ExtensionItem>,
        board_config_path: Option<PathBuf>,
        sisho_program: PathBuf,
    ) -> Self {
        let board_config = board_config_path
            .as_deref()
            .map(BoardConfig::load_from)
            .unwrap_or_default();

        let (sender, receiver) = mpsc::channel();
        let steps = check_steps(&dir);
        let check_slots = steps
            .iter()
            .map(|s| StepSlot::Running {
                name: s.name().to_string(),
            })
            .collect();

        let mut model = Self {
            dir,
            sisho_program,
            board_config_path,
            board_config,
            tools: Vec::new(),
            extensions,
            check_slots,
            check_outcomes: Vec::new(),
            links_report: None,
            health_report: None,
            selected_extension: 0,
            selected_step: 0,
            selected_health_pillar: 0,
            selected_broken_link: 0,
            selected_ext_item: 0,
            selected_standalone_ext_items: BTreeMap::new(),
            preview_scroll: 0,
            focused_pane: PaneId::default(),
            pane_active: false,
            preview_source: PreviewSource::default(),
            confirm_fix: false,
            pending_trust: None,
            is_launcher_open: false,
            launcher_selected: 0,
            layout_editor: LayoutEditorState::default(),
            message: Some("Running checks...".to_string()),
            is_checking: false,
            is_checking_links: false,
            is_checking_health: false,
            is_fixing: false,
            tick: 0,
            sender,
            receiver,
        };
        model.ensure_focus_visible();
        model.refresh_tools();
        model.query_allowed_extensions();
        model.run_check_links();
        model.run_health();
        model.run_checks();
        model
    }

    /// Refresh tools information and version status synchronously.
    pub fn refresh_tools(&mut self) {
        let mut statuses = Vec::new();
        for tool in Tool::ALL {
            let step = tool.version_step();
            let outcome = step.run();
            let reported = outcome
                .is_pass()
                .then(|| outcome.stdout.lines().next())
                .flatten();
            let verdict = tool.judge(reported);
            let version = reported.map(|s| s.to_string());
            statuses.push(ToolStatus {
                name: tool.name(),
                version,
                verdict,
            });
        }
        self.tools = statuses;
    }

    /// Query `--sisho-extension-info` and `--sisho-board-pane` in background threads
    /// ONLY for active extensions whose [`TrustState`] is [`TrustState::Allowed`].
    ///
    /// Extensions that are [`TrustState::Untrusted`] or [`TrustState::Denied`] are
    /// strictly never executed.
    pub fn query_allowed_extensions(&mut self) {
        for index in 0..self.extensions.len() {
            if self.extensions[index].is_active
                && self.extensions[index].trust == TrustState::Allowed
            {
                self.spawn_extension_query(index);
            }
        }
    }

    fn spawn_extension_query(&mut self, index: usize) {
        let Some(ext) = self.extensions.get_mut(index) else {
            return;
        };
        if !ext.is_active || ext.trust != TrustState::Allowed {
            return;
        }
        ext.is_querying = true;
        let word = ext.word.clone();
        let path = ext.path.clone();
        let dir = self.dir.clone();
        let sender = self.sender.clone();

        std::thread::spawn(move || {
            let info = query_info(&word, &path);
            let board_pane = query_board_pane(&word, &path, &dir);
            let _ = sender.send(WorkerMessage::ExtensionQueried {
                word,
                info,
                board_pane,
            });
        });
    }

    fn request_trust_prompt_at(&mut self, index: usize) {
        let Some(ext) = self.extensions.get(index) else {
            return;
        };
        self.confirm_fix = false;
        self.pending_trust = Some(PendingTrustPrompt {
            word: ext.word.clone(),
            canonical_path: ext.canonical_path.clone(),
            current_trust: ext.trust,
        });
    }

    /// Open the trust confirmation prompt for the currently selected extension
    /// in `Tools & Extensions`, without executing or changing trust yet.
    pub fn request_trust_prompt(&mut self) {
        self.request_trust_prompt_at(self.selected_extension);
    }

    /// Open the trust confirmation prompt for the currently selected extension
    /// in the TUI extension launcher, without executing or changing trust yet.
    pub fn request_launcher_trust_prompt(&mut self) {
        let Some((index, _)) = self.launcher_items().get(self.launcher_selected).copied() else {
            return;
        };
        self.request_trust_prompt_at(index);
    }

    /// Apply a confirmed trust decision (`Allowed`, `Denied`, or `Untrusted`)
    /// by delegating to `sisho --allow|--deny|--untrust <canonical_path>`.
    pub fn apply_pending_trust(&mut self, new_state: TrustState) {
        let Some(prompt) = self.pending_trust.take() else {
            return;
        };

        if let Err(err) = set_extension_trust(&self.sisho_program, &prompt.canonical_path, new_state)
        {
            self.message = Some(format!(
                "Failed to update trust for sisho-{}: {err}",
                prompt.word
            ));
            return;
        }

        let mut query_indices = Vec::new();
        for (idx, ext) in self.extensions.iter_mut().enumerate() {
            if ext.canonical_path == prompt.canonical_path {
                ext.trust = new_state;
                if new_state == TrustState::Allowed {
                    if ext.is_active {
                        query_indices.push(idx);
                    }
                } else {
                    ext.info = None;
                    ext.board_pane = None;
                    ext.is_querying = false;
                }
            }
        }

        self.clamp_launcher_selection();

        match new_state {
            TrustState::Allowed => {
                let any_queried = !query_indices.is_empty();
                for idx in query_indices {
                    self.spawn_extension_query(idx);
                }
                if any_queried {
                    self.message = Some(format!(
                        "Allowed sisho-{} via sisho. Querying extension...",
                        prompt.word
                    ));
                } else {
                    self.message = Some(format!("Allowed sisho-{} via sisho.", prompt.word));
                }
            }
            TrustState::Denied | TrustState::Untrusted => {
                let total_items = self.extension_pane_items().len();
                if self.selected_ext_item >= total_items {
                    self.selected_ext_item = total_items.saturating_sub(1);
                }
                self.selected_standalone_ext_items.remove(&prompt.word);
                if (!self.has_extension_panes() && self.preview_source == PreviewSource::ExtensionItem)
                    || matches!(&self.preview_source, PreviewSource::StandaloneExtensionItem(w) if w == &prompt.word)
                {
                    self.preview_source = PreviewSource::CheckStep;
                }
                let action = match new_state {
                    TrustState::Denied => "Denied",
                    TrustState::Untrusted => "Reset to untrusted:",
                    TrustState::Allowed => unreachable!(),
                };
                self.message = Some(format!("{action} sisho-{} via sisho.", prompt.word));
            }
        }
    }

    /// Run the check steps suite across the project in a background thread.
    pub fn run_checks(&mut self) {
        if self.is_checking || self.is_fixing {
            return;
        }
        self.is_checking = true;
        let steps = check_steps(&self.dir);
        self.check_slots = steps
            .iter()
            .map(|s| StepSlot::Running {
                name: s.name().to_string(),
            })
            .collect();
        self.check_outcomes.clear();
        self.preview_scroll = 0;
        self.message = Some("Running checks...".to_string());

        let dir = self.dir.clone();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let steps = check_steps(&dir);
            let sender_clone = sender.clone();
            let outcomes = run_parallel_streaming(steps, move |index, outcome| {
                let _ = sender_clone.send(WorkerMessage::CheckStepDone { index, outcome });
            });
            let _ = sender.send(WorkerMessage::ChecksDone(outcomes));
        });
    }

    /// Run the fix steps suite across the project in a background thread.
    pub fn run_fixes(&mut self) {
        if self.is_fixing || self.is_checking {
            return;
        }
        self.is_fixing = true;
        self.confirm_fix = false;
        let steps = fix_steps(&self.dir);
        self.check_slots = steps
            .iter()
            .map(|s| StepSlot::Running {
                name: s.name().to_string(),
            })
            .collect();
        self.check_outcomes.clear();
        self.preview_scroll = 0;
        self.message = Some("Applying fixes...".to_string());

        let dir = self.dir.clone();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let steps = fix_steps(&dir);
            let mut outcomes = Vec::new();
            for (index, step) in steps.into_iter().enumerate() {
                let outcome = step.run();
                let _ = sender.send(WorkerMessage::FixStepDone {
                    index,
                    outcome: outcome.clone(),
                });
                outcomes.push(outcome);
            }
            let _ = sender.send(WorkerMessage::FixesDone(outcomes));
        });
    }

    /// Run check-links verification on the project directory in a background thread.
    pub fn run_check_links(&mut self) {
        if self.is_checking_links {
            return;
        }
        self.is_checking_links = true;
        let dir = self.dir.clone();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let res = links::check(&dir);
            let _ = sender.send(WorkerMessage::LinksDone(res));
        });
    }

    /// Run the built-in Readiness & Health analyzer in a background thread.
    pub fn run_health(&mut self) {
        if self.is_checking_health {
            return;
        }
        self.is_checking_health = true;
        let dir = self.dir.clone();
        let links_report = self.links_report.clone();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let report = health::analyze(&dir, links_report.as_ref());
            let _ = sender.send(WorkerMessage::HealthDone(Box::new(report)));
        });
    }

    /// Poll any messages from background worker threads and update model state.
    pub fn poll_background(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        while let Ok(msg) = self.receiver.try_recv() {
            match msg {
                WorkerMessage::CheckStepDone { index, outcome } => {
                    if index < self.check_slots.len() {
                        self.check_slots[index] = StepSlot::Done(outcome);
                    }
                }
                WorkerMessage::ChecksDone(outcomes) => {
                    self.check_outcomes = outcomes.clone();
                    self.check_slots = outcomes.into_iter().map(StepSlot::Done).collect();
                    self.is_checking = false;
                    if self.selected_step >= self.check_slots.len() {
                        self.selected_step = 0;
                    }
                    self.message = Some("Checks completed.".to_string());
                }
                WorkerMessage::FixStepDone { index, outcome } => {
                    if index < self.check_slots.len() {
                        self.check_slots[index] = StepSlot::Done(outcome);
                    }
                }
                WorkerMessage::FixesDone(outcomes) => {
                    self.check_outcomes = outcomes.clone();
                    self.check_slots = outcomes.into_iter().map(StepSlot::Done).collect();
                    self.is_fixing = false;
                    if self.selected_step >= self.check_slots.len() {
                        self.selected_step = 0;
                    }
                    self.run_health();
                    self.message = Some("Fixes applied. Run check to verify.".to_string());
                }
                WorkerMessage::LinksDone(res) => {
                    self.is_checking_links = false;
                    match res {
                        Ok(report) => {
                            self.links_report = Some(report);
                            if self.selected_broken_link >= self.broken_links().len() {
                                self.selected_broken_link = 0;
                            }
                            if let Some(health) = &mut self.health_report {
                                health.update_link_health(self.links_report.as_ref());
                            }
                        }
                        Err(err) => {
                            self.message = Some(format!("check-links error: {err}"));
                        }
                    }
                }
                WorkerMessage::HealthDone(mut report) => {
                    self.is_checking_health = false;
                    if self.links_report.is_some() {
                        report.update_link_health(self.links_report.as_ref());
                    }
                    self.health_report = Some(*report);
                }
                WorkerMessage::ExtensionQueried {
                    word,
                    info,
                    board_pane,
                } => {
                    if let Some(ext) = self.extensions.iter_mut().find(|e| e.word == word) {
                        ext.is_querying = false;
                        if ext.trust == TrustState::Allowed {
                            ext.info = info;
                            ext.board_pane = board_pane;
                            let count = ext
                                .board_pane
                                .as_ref()
                                .map(|p| p.items.len())
                                .unwrap_or(0);
                            if let Some(sel) = self.selected_standalone_ext_items.get_mut(&word) {
                                if count == 0 {
                                    *sel = 0;
                                } else if *sel >= count {
                                    *sel = count - 1;
                                }
                            }
                        }
                    }
                    self.clamp_launcher_selection();
                }
            }
        }
    }

    pub fn broken_links(&self) -> &[BrokenLink] {
        self.links_report
            .as_ref()
            .map(|r| r.broken.as_slice())
            .unwrap_or(&[])
    }

    /// True if at least one allowed (and non-hidden) extension currently has a [`BoardPane`].
    pub fn has_extension_panes(&self) -> bool {
        self.extensions.iter().any(|e| {
            e.trust == TrustState::Allowed
                && !self.board_config.is_pane_hidden(&e.word)
                && e.board_pane.is_some()
        })
    }

    /// Return all allowed (and non-hidden) extensions that have emitted a [`BoardPane`].
    pub fn active_extension_panes(&self) -> Vec<(&str, &BoardPane)> {
        self.extensions
            .iter()
            .filter(|e| {
                e.trust == TrustState::Allowed && !self.board_config.is_pane_hidden(&e.word)
            })
            .filter_map(|e| e.board_pane.as_ref().map(|pane| (e.word.as_str(), pane)))
            .collect()
    }

    /// Flat list of all selectable items across active extension [`BoardPane`]s.
    pub fn extension_pane_items(&self) -> Vec<ExtensionPaneItemRef<'_>> {
        let mut list = Vec::new();
        for (word, pane) in self.active_extension_panes() {
            for item in &pane.items {
                list.push(ExtensionPaneItemRef { word, pane, item });
            }
        }
        list
    }

    /// Currently selected item in [`PaneId::ExtensionPanes`].
    pub fn selected_extension_pane_item(&self) -> Option<ExtensionPaneItemRef<'_>> {
        let items = self.extension_pane_items();
        items.get(self.selected_ext_item).cloned()
    }

    /// Return the [`BoardPane`] for an allowed standalone extension pane `word`, if available.
    pub fn standalone_extension_pane(&self, word: &str) -> Option<&BoardPane> {
        self.extensions
            .iter()
            .find(|e| e.word == word && e.trust == TrustState::Allowed)
            .and_then(|e| e.board_pane.as_ref())
    }

    /// Currently selected item index inside a standalone [`PaneId::Extension`] pane.
    pub fn selected_standalone_ext_item_index(&self, word: &str) -> usize {
        self.selected_standalone_ext_items
            .get(word)
            .copied()
            .unwrap_or(0)
    }

    /// Currently selected [`BoardItem`] inside a standalone [`PaneId::Extension`] pane.
    pub fn selected_standalone_ext_item(&self, word: &str) -> Option<&BoardItem> {
        let pane = self.standalone_extension_pane(word)?;
        let idx = self.selected_standalone_ext_item_index(word);
        pane.items.get(idx)
    }

    /// Currently selected pillar in [`PaneId::Health`].
    pub fn selected_health_pillar_metric(&self) -> Option<&PillarMetric> {
        self.health_report
            .as_ref()
            .and_then(|r| r.pillars().get(self.selected_health_pillar).copied())
    }

    /// Return the preview text for the currently active [`PreviewSource`].
    pub fn selected_preview_text(&self) -> Option<String> {
        match &self.preview_source {
            PreviewSource::HealthPillar => {
                if let Some(pillar) = self.selected_health_pillar_metric() {
                    Some(pillar.detail.clone())
                } else {
                    Some("Scanning vault readiness and health...".to_string())
                }
            }
            PreviewSource::ExtensionItem => self
                .selected_extension_pane_item()
                .and_then(|ext_ref| ext_ref.item.detail.clone().or_else(|| ext_ref.pane.summary.clone())),
            PreviewSource::StandaloneExtensionItem(word) => {
                let pane = self.standalone_extension_pane(word)?;
                self.selected_standalone_ext_item(word)
                    .and_then(|item| item.detail.clone())
                    .or_else(|| pane.summary.clone())
            }
            PreviewSource::CheckStep => self.selected_outcome().map(|o| {
                let mut text = o.stdout.clone();
                if !o.stderr.is_empty() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&o.stderr);
                }
                text
            }),
        }
    }

    /// Snap `focused_pane` to the first visible pane in the active view if the
    /// currently focused pane is no longer present, and reset `pane_active`.
    pub fn ensure_focus_visible(&mut self) {
        self.pane_active = false;
        let visible = self.active_view().visible_panes();
        if let Some(&first) = visible.first()
            && !visible.contains(&&self.focused_pane)
        {
            self.focused_pane = first.clone();
            self.sync_preview_source();
        }
    }

    /// Activate the currently focused pane so navigation keys move within its items.
    pub fn enter_pane(&mut self) {
        if self.active_view().contains(&self.focused_pane) {
            self.pane_active = true;
        }
    }

    /// Exit active pane item mode back to 2D pane selection.
    pub fn exit_pane(&mut self) {
        self.pane_active = false;
    }

    fn focused_pane_position(&self) -> Option<(bool, usize)> {
        let view = self.active_view();
        if let Some(idx) = view.left.iter().position(|s| s.pane == self.focused_pane) {
            return Some((true, idx));
        }
        if let Some(idx) = view.right.iter().position(|s| s.pane == self.focused_pane) {
            return Some((false, idx));
        }
        None
    }

    /// Move pane focus one slot up within the current column.
    pub fn move_pane_up(&mut self) {
        let Some((is_left, idx)) = self.focused_pane_position() else {
            return;
        };
        if idx == 0 {
            return;
        }
        let view = self.active_view();
        let target = if is_left {
            view.left[idx - 1].pane.clone()
        } else {
            view.right[idx - 1].pane.clone()
        };
        self.focused_pane = target;
        self.sync_preview_source();
    }

    /// Move pane focus one slot down within the current column.
    pub fn move_pane_down(&mut self) {
        let Some((is_left, idx)) = self.focused_pane_position() else {
            return;
        };
        let view = self.active_view();
        let col = if is_left { &view.left } else { &view.right };
        if idx + 1 >= col.len() {
            return;
        }
        self.focused_pane = col[idx + 1].pane.clone();
        self.sync_preview_source();
    }

    /// Move pane focus from the right column to a proportional slot in the left column.
    pub fn move_pane_left(&mut self) {
        let Some((false, idx)) = self.focused_pane_position() else {
            return;
        };
        let view = self.active_view();
        if view.left.is_empty() || view.right.is_empty() {
            return;
        }
        let target_idx = (idx * view.left.len() / view.right.len()).min(view.left.len() - 1);
        self.focused_pane = view.left[target_idx].pane.clone();
        self.sync_preview_source();
    }

    /// Move pane focus from the left column to a proportional slot in the right column.
    pub fn move_pane_right(&mut self) {
        let Some((true, idx)) = self.focused_pane_position() else {
            return;
        };
        let view = self.active_view();
        if view.left.is_empty() || view.right.is_empty() {
            return;
        }
        let target_idx = (idx * view.right.len() / view.left.len()).min(view.right.len() - 1);
        self.focused_pane = view.right[target_idx].pane.clone();
        self.sync_preview_source();
    }

    /// Cycle focus forward across only the panes visible in `active_view()`.
    pub fn focus_next_pane(&mut self) {
        let order: Vec<PaneId> = self
            .active_view()
            .visible_panes()
            .into_iter()
            .cloned()
            .collect();
        if order.is_empty() {
            return;
        }
        let pos = order
            .iter()
            .position(|p| p == &self.focused_pane)
            .unwrap_or(0);
        self.focused_pane = order[(pos + 1) % order.len()].clone();
        self.sync_preview_source();
    }

    /// Cycle focus backward across only the panes visible in `active_view()`.
    pub fn focus_prev_pane(&mut self) {
        let order: Vec<PaneId> = self
            .active_view()
            .visible_panes()
            .into_iter()
            .cloned()
            .collect();
        if order.is_empty() {
            return;
        }
        let pos = order
            .iter()
            .position(|p| p == &self.focused_pane)
            .unwrap_or(0);
        self.focused_pane = order[(pos + order.len() - 1) % order.len()].clone();
        self.sync_preview_source();
    }

    /// Alias for [`Model::focus_next_pane`].
    #[inline]
    pub fn next_pane(&mut self) {
        self.focus_next_pane();
    }

    /// Alias for [`Model::focus_prev_pane`].
    #[inline]
    pub fn prev_pane(&mut self) {
        self.focus_prev_pane();
    }

    fn sync_preview_source(&mut self) {
        match &self.focused_pane {
            PaneId::Checks => {
                self.preview_source = PreviewSource::CheckStep;
                self.preview_scroll = 0;
            }
            PaneId::Health => {
                self.preview_source = PreviewSource::HealthPillar;
                self.preview_scroll = 0;
            }
            PaneId::ExtensionPanes => {
                self.preview_source = PreviewSource::ExtensionItem;
                self.preview_scroll = 0;
            }
            PaneId::Extension(word) => {
                self.preview_source = PreviewSource::StandaloneExtensionItem(word.clone());
                self.preview_scroll = 0;
            }
            PaneId::Extensions | PaneId::BrokenLinks | PaneId::OutputPreview => {}
        }
    }

    pub fn move_selection_up(&mut self) {
        match self.focused_pane.clone() {
            PaneId::Checks => self.move_check_selection_up(),
            PaneId::Health => self.move_health_selection_up(),
            PaneId::Extensions => self.move_extension_selection_up(),
            PaneId::BrokenLinks => self.move_link_selection_up(),
            PaneId::ExtensionPanes => self.move_ext_item_selection_up(),
            PaneId::OutputPreview => self.scroll_preview_up(1),
            PaneId::Extension(word) => self.move_standalone_ext_item_selection_up(&word),
        }
    }

    pub fn move_selection_down(&mut self) {
        match self.focused_pane.clone() {
            PaneId::Checks => self.move_check_selection_down(),
            PaneId::Health => self.move_health_selection_down(),
            PaneId::Extensions => self.move_extension_selection_down(),
            PaneId::BrokenLinks => self.move_link_selection_down(),
            PaneId::ExtensionPanes => self.move_ext_item_selection_down(),
            PaneId::OutputPreview => self.scroll_preview_down(1),
            PaneId::Extension(word) => self.move_standalone_ext_item_selection_down(&word),
        }
    }

    pub fn move_health_selection_up(&mut self) {
        if self.selected_health_pillar > 0 {
            self.selected_health_pillar -= 1;
            self.preview_scroll = 0;
            self.preview_source = PreviewSource::HealthPillar;
        }
    }

    pub fn move_health_selection_down(&mut self) {
        if self.selected_health_pillar + 1 < 6 {
            self.selected_health_pillar += 1;
            self.preview_scroll = 0;
            self.preview_source = PreviewSource::HealthPillar;
        }
    }

    pub fn move_extension_selection_up(&mut self) {
        if self.selected_extension > 0 {
            self.selected_extension -= 1;
        }
    }

    pub fn move_extension_selection_down(&mut self) {
        if !self.extensions.is_empty() && self.selected_extension + 1 < self.extensions.len() {
            self.selected_extension += 1;
        }
    }

    pub fn move_check_selection_up(&mut self) {
        if self.selected_step > 0 {
            self.selected_step -= 1;
            self.preview_scroll = 0;
            self.preview_source = PreviewSource::CheckStep;
        }
    }

    pub fn move_check_selection_down(&mut self) {
        if !self.check_slots.is_empty() && self.selected_step + 1 < self.check_slots.len() {
            self.selected_step += 1;
            self.preview_scroll = 0;
            self.preview_source = PreviewSource::CheckStep;
        }
    }

    pub fn move_ext_item_selection_up(&mut self) {
        if self.selected_ext_item > 0 {
            self.selected_ext_item -= 1;
            self.preview_scroll = 0;
            self.preview_source = PreviewSource::ExtensionItem;
        }
    }

    pub fn move_ext_item_selection_down(&mut self) {
        let count = self.extension_pane_items().len();
        if count > 0 && self.selected_ext_item + 1 < count {
            self.selected_ext_item += 1;
            self.preview_scroll = 0;
            self.preview_source = PreviewSource::ExtensionItem;
        }
    }

    pub fn move_standalone_ext_item_selection_up(&mut self, word: &str) {
        let current = self.selected_standalone_ext_item_index(word);
        if current > 0 {
            self.selected_standalone_ext_items
                .insert(word.to_string(), current - 1);
            self.preview_scroll = 0;
            self.preview_source = PreviewSource::StandaloneExtensionItem(word.to_string());
        }
    }

    pub fn move_standalone_ext_item_selection_down(&mut self, word: &str) {
        let count = self
            .standalone_extension_pane(word)
            .map(|p| p.items.len())
            .unwrap_or(0);
        let current = self.selected_standalone_ext_item_index(word);
        if count > 0 && current + 1 < count {
            self.selected_standalone_ext_items
                .insert(word.to_string(), current + 1);
            self.preview_scroll = 0;
            self.preview_source = PreviewSource::StandaloneExtensionItem(word.to_string());
        }
    }

    pub fn move_link_selection_up(&mut self) {
        if self.selected_broken_link > 0 {
            self.selected_broken_link -= 1;
        }
    }

    pub fn move_link_selection_down(&mut self) {
        let count = self.broken_links().len();
        if count > 0 && self.selected_broken_link + 1 < count {
            self.selected_broken_link += 1;
        }
    }

    pub fn scroll_preview_up(&mut self, lines: usize) {
        self.preview_scroll = self.preview_scroll.saturating_sub(lines);
    }

    pub fn scroll_preview_down(&mut self, lines: usize) {
        self.preview_scroll = self.preview_scroll.saturating_add(lines);
    }

    pub fn selected_outcome(&self) -> Option<&Outcome> {
        self.check_slots
            .get(self.selected_step)
            .and_then(StepSlot::outcome)
    }

    pub fn selected_broken_link_item(&self) -> Option<&BrokenLink> {
        self.broken_links().get(self.selected_broken_link)
    }

    /// Active extensions eligible to appear in the global TUI extension launcher.
    ///
    /// Excludes `board`, inactive/shadowed extensions, and [`TrustState::Denied`] extensions.
    /// [`TrustState::Allowed`] extensions are included only once queried and known to declare
    /// `capabilities.tui = true`. [`TrustState::Untrusted`] active extensions are included
    /// (without ever executing them) so users can inspect or allow them in place.
    pub fn launcher_items(&self) -> Vec<(usize, &ExtensionItem)> {
        self.extensions
            .iter()
            .enumerate()
            .filter(|(_, ext)| {
                ext.is_active
                    && ext.word != "board"
                    && match ext.trust {
                        TrustState::Allowed => {
                            ext.info.as_ref().is_some_and(|i| i.capabilities.tui)
                        }
                        TrustState::Untrusted => true,
                        TrustState::Denied => false,
                    }
            })
            .collect()
    }

    fn clamp_launcher_selection(&mut self) {
        let count = self.launcher_items().len();
        if count == 0 {
            self.launcher_selected = 0;
        } else if self.launcher_selected >= count {
            self.launcher_selected = count - 1;
        }
    }

    /// Open the global TUI extension launcher modal, clearing transient counter prompts.
    pub fn open_launcher(&mut self) {
        self.confirm_fix = false;
        self.pending_trust = None;
        self.close_layout_editor();
        self.is_launcher_open = true;
        self.clamp_launcher_selection();
    }

    /// Close the global TUI extension launcher modal.
    pub fn close_launcher(&mut self) {
        self.is_launcher_open = false;
    }

    /// Move the launcher selection up by one item.
    pub fn move_launcher_selection_up(&mut self) {
        if self.launcher_selected > 0 {
            self.launcher_selected -= 1;
        }
    }

    /// Move the launcher selection down by one item.
    pub fn move_launcher_selection_down(&mut self) {
        let count = self.launcher_items().len();
        if count > 0 && self.launcher_selected + 1 < count {
            self.launcher_selected += 1;
        }
    }

    /// Return the currently selected extension in the TUI extension launcher.
    pub fn selected_launcher_extension(&self) -> Option<&ExtensionItem> {
        self.launcher_items()
            .get(self.launcher_selected)
            .map(|&(_, ext)| ext)
    }

    // -------------------------------------------------------------------------
    // Board Views & Persistence
    // -------------------------------------------------------------------------

    fn save_board_config(&mut self) {
        if let Some(path) = &self.board_config_path
            && let Err(err) = self.board_config.save_to(path)
        {
            self.message = Some(format!("Failed to save board config: {err}"));
        }
    }

    /// Index of the currently active view in `board_config.views`.
    pub fn active_view_index(&self) -> usize {
        self.board_config
            .active_view
            .min(self.board_config.views.len().saturating_sub(1))
    }

    /// Reference to the currently active [`BoardView`].
    pub fn active_view(&self) -> &BoardView {
        self.board_config.active_view()
    }

    /// Switch to the view at `idx` (no-op if out of bounds), ensuring focus stays
    /// on a visible pane and persisting the updated `active_view`.
    pub fn select_view(&mut self, idx: usize) {
        if idx >= self.board_config.views.len() {
            return;
        }
        self.board_config.active_view = idx;
        self.pane_active = false;
        self.layout_editor.grabbed = false;
        self.clamp_editor_selection();
        self.ensure_focus_visible();
        self.save_board_config();
    }

    /// Switch to the next view in `board_config.views` (wrapping around).
    pub fn next_view(&mut self) {
        let len = self.board_config.views.len();
        if len <= 1 {
            return;
        }
        let next = (self.active_view_index() + 1) % len;
        self.select_view(next);
    }

    /// Switch to the previous view in `board_config.views` (wrapping around).
    pub fn prev_view(&mut self) {
        let len = self.board_config.views.len();
        if len <= 1 {
            return;
        }
        let prev = (self.active_view_index() + len - 1) % len;
        self.select_view(prev);
    }

    // -------------------------------------------------------------------------
    // Layout Editor
    // -------------------------------------------------------------------------

    /// Open the Layout Editor modal, clearing transient prompts from other modals.
    pub fn open_layout_editor(&mut self) {
        self.confirm_fix = false;
        self.pending_trust = None;
        self.is_launcher_open = false;
        self.pane_active = false;
        self.layout_editor.is_open = true;
        self.layout_editor.grabbed = false;
        self.layout_editor.rename_buffer = None;
        self.layout_editor.confirm_action = None;
        self.clamp_editor_selection();
    }

    /// Close the Layout Editor modal and discard transient rename/confirmation state.
    pub fn close_layout_editor(&mut self) {
        self.layout_editor.is_open = false;
        self.layout_editor.grabbed = false;
        self.layout_editor.rename_buffer = None;
        self.layout_editor.confirm_action = None;
    }

    /// All panes (built-in and allowed extension panes with `board_pane`) not currently
    /// placed in `active_view()`.
    pub fn available_panes(&self) -> Vec<PaneId> {
        let view = self.active_view();
        let mut panes: Vec<PaneId> = PaneId::BUILTIN_ALL
            .into_iter()
            .filter(|pane| !view.contains(pane))
            .collect();
        for ext in &self.extensions {
            if ext.trust == TrustState::Allowed && ext.board_pane.is_some() {
                let pane = PaneId::Extension(ext.word.clone());
                if !view.contains(&pane) && !panes.contains(&pane) {
                    panes.push(pane);
                }
            }
        }
        panes
    }

    fn editor_column_len(&self, col: EditorColumn) -> usize {
        match col {
            EditorColumn::Left => self.active_view().left.len(),
            EditorColumn::Right => self.active_view().right.len(),
            EditorColumn::Available => self.available_panes().len(),
        }
    }

    fn clamp_editor_selection(&mut self) {
        let len = self.editor_column_len(self.layout_editor.column);
        if len == 0 {
            self.layout_editor.selected = 0;
        } else if self.layout_editor.selected >= len {
            self.layout_editor.selected = len - 1;
        }
    }

    fn push_layout_undo(&mut self) {
        self.layout_editor.push_undo(self.board_config.clone());
    }

    fn finish_layout_mutation(&mut self) {
        self.clamp_editor_selection();
        self.ensure_focus_visible();
        self.save_board_config();
    }

    /// Toggle Grab & Drop mode for the currently selected pane in the Layout Editor.
    ///
    /// - If already grabbed, drops the pane in place.
    /// - If in `Left` or `Right` with a valid selection, grabs that pane.
    /// - If in `Available` with a valid selection, inserts the pane at the end of `Left`,
    ///   focuses `Left`, and grabs it immediately for positioning.
    pub fn toggle_grab(&mut self) {
        if self.layout_editor.grabbed {
            self.layout_editor.grabbed = false;
            return;
        }
        match self.layout_editor.column {
            EditorColumn::Left | EditorColumn::Right => {
                if self.layout_editor.selected < self.editor_column_len(self.layout_editor.column) {
                    self.layout_editor.grabbed = true;
                }
            }
            EditorColumn::Available => {
                let available = self.available_panes();
                let Some(pane) = available.get(self.layout_editor.selected).cloned() else {
                    return;
                };
                self.push_layout_undo();
                let view = self.board_config.active_view_mut();
                view.left.push(PaneSlot::new(pane, DEFAULT_PANE_WEIGHT));
                let new_idx = view.left.len() - 1;
                self.layout_editor.column = EditorColumn::Left;
                self.layout_editor.selected = new_idx;
                self.layout_editor.grabbed = true;
                self.finish_layout_mutation();
            }
        }
    }

    /// Drop any currently grabbed pane without undoing its current placement.
    pub fn cancel_grab(&mut self) {
        self.layout_editor.grabbed = false;
    }

    /// Move the layout editor cursor up, or reorder the grabbed pane up within its column.
    pub fn move_cursor_up(&mut self) {
        if self.layout_editor.grabbed {
            self.reorder_selected_pane_up();
            return;
        }
        if self.layout_editor.selected > 0 {
            self.layout_editor.selected -= 1;
        }
    }

    /// Move the layout editor cursor down, or reorder the grabbed pane down within its column.
    pub fn move_cursor_down(&mut self) {
        if self.layout_editor.grabbed {
            self.reorder_selected_pane_down();
            return;
        }
        let len = self.editor_column_len(self.layout_editor.column);
        if len > 0 && self.layout_editor.selected + 1 < len {
            self.layout_editor.selected += 1;
        }
    }

    /// Move column focus left (`Available -> Right -> Left`), or move a grabbed pane
    /// from `Right` to `Left` at `selected.min(left.len())`.
    pub fn move_column_left(&mut self) {
        if self.layout_editor.grabbed {
            if self.layout_editor.column == EditorColumn::Right {
                let idx = self.layout_editor.selected;
                if idx >= self.active_view().right.len() {
                    self.layout_editor.grabbed = false;
                    return;
                }
                self.push_layout_undo();
                let view = self.board_config.active_view_mut();
                let slot = view.right.remove(idx);
                let inserted_idx = idx.min(view.left.len());
                view.left.insert(inserted_idx, slot);
                self.layout_editor.column = EditorColumn::Left;
                self.layout_editor.selected = inserted_idx;
                self.finish_layout_mutation();
            }
            return;
        }
        self.layout_editor.column = match self.layout_editor.column {
            EditorColumn::Left => EditorColumn::Left,
            EditorColumn::Right => EditorColumn::Left,
            EditorColumn::Available => EditorColumn::Right,
        };
        self.clamp_editor_selection();
    }

    /// Move column focus right (`Left -> Right -> Available`), or move a grabbed pane
    /// from `Left` to `Right` (or from `Right` to `Available`, dropping it).
    pub fn move_column_right(&mut self) {
        if self.layout_editor.grabbed {
            match self.layout_editor.column {
                EditorColumn::Left => {
                    let idx = self.layout_editor.selected;
                    if idx >= self.active_view().left.len() {
                        self.layout_editor.grabbed = false;
                        return;
                    }
                    self.push_layout_undo();
                    let view = self.board_config.active_view_mut();
                    let slot = view.left.remove(idx);
                    let inserted_idx = idx.min(view.right.len());
                    view.right.insert(inserted_idx, slot);
                    self.layout_editor.column = EditorColumn::Right;
                    self.layout_editor.selected = inserted_idx;
                    self.finish_layout_mutation();
                }
                EditorColumn::Right => {
                    let idx = self.layout_editor.selected;
                    if idx >= self.active_view().right.len() {
                        self.layout_editor.grabbed = false;
                        return;
                    }
                    self.push_layout_undo();
                    let slot = self.board_config.active_view_mut().right.remove(idx);
                    let avail_idx = self
                        .available_panes()
                        .iter()
                        .position(|p| p == &slot.pane)
                        .unwrap_or(0);
                    self.layout_editor.column = EditorColumn::Available;
                    self.layout_editor.selected = avail_idx;
                    self.layout_editor.grabbed = false;
                    self.finish_layout_mutation();
                }
                EditorColumn::Available => {
                    self.layout_editor.grabbed = false;
                }
            }
            return;
        }
        self.layout_editor.column = match self.layout_editor.column {
            EditorColumn::Left => EditorColumn::Right,
            EditorColumn::Right => EditorColumn::Available,
            EditorColumn::Available => EditorColumn::Available,
        };
        self.clamp_editor_selection();
    }

    /// Move the currently selected pane between the Left and Right columns.
    pub fn move_pane_between_columns(&mut self) {
        let idx = self.layout_editor.selected;
        match self.layout_editor.column {
            EditorColumn::Left => {
                if idx >= self.active_view().left.len() {
                    return;
                }
                self.push_layout_undo();
                let view = self.board_config.active_view_mut();
                let slot = view.left.remove(idx);
                view.right.push(slot);
                self.finish_layout_mutation();
            }
            EditorColumn::Right => {
                if idx >= self.active_view().right.len() {
                    return;
                }
                self.push_layout_undo();
                let view = self.board_config.active_view_mut();
                let slot = view.right.remove(idx);
                view.left.push(slot);
                self.finish_layout_mutation();
            }
            EditorColumn::Available => {}
        }
    }

    /// Add the currently selected pane from `Available` into `target` (`Left` or `Right`).
    pub fn add_available_pane_to(&mut self, target: EditorColumn) {
        if target == EditorColumn::Available || self.layout_editor.column != EditorColumn::Available
        {
            return;
        }
        let available = self.available_panes();
        let Some(pane) = available.get(self.layout_editor.selected).cloned() else {
            return;
        };
        self.push_layout_undo();
        let slot = PaneSlot::new(pane, DEFAULT_PANE_WEIGHT);
        let view = self.board_config.active_view_mut();
        match target {
            EditorColumn::Left => view.left.push(slot),
            EditorColumn::Right => view.right.push(slot),
            EditorColumn::Available => unreachable!(),
        }
        self.finish_layout_mutation();
    }

    /// Remove the currently selected pane from the `Left` or `Right` column.
    pub fn remove_selected_pane(&mut self) {
        let idx = self.layout_editor.selected;
        match self.layout_editor.column {
            EditorColumn::Left => {
                if idx >= self.active_view().left.len() {
                    return;
                }
                self.push_layout_undo();
                self.board_config.active_view_mut().left.remove(idx);
                self.layout_editor.grabbed = false;
                self.finish_layout_mutation();
            }
            EditorColumn::Right => {
                if idx >= self.active_view().right.len() {
                    return;
                }
                self.push_layout_undo();
                self.board_config.active_view_mut().right.remove(idx);
                self.layout_editor.grabbed = false;
                self.finish_layout_mutation();
            }
            EditorColumn::Available => {}
        }
    }

    /// Move the selected pane up by one slot within its column.
    pub fn reorder_selected_pane_up(&mut self) {
        let idx = self.layout_editor.selected;
        if idx == 0 {
            return;
        }
        match self.layout_editor.column {
            EditorColumn::Left => {
                if idx >= self.active_view().left.len() {
                    return;
                }
                self.push_layout_undo();
                self.board_config.active_view_mut().left.swap(idx, idx - 1);
                self.layout_editor.selected -= 1;
                self.finish_layout_mutation();
            }
            EditorColumn::Right => {
                if idx >= self.active_view().right.len() {
                    return;
                }
                self.push_layout_undo();
                self.board_config.active_view_mut().right.swap(idx, idx - 1);
                self.layout_editor.selected -= 1;
                self.finish_layout_mutation();
            }
            EditorColumn::Available => {}
        }
    }

    /// Move the selected pane down by one slot within its column.
    pub fn reorder_selected_pane_down(&mut self) {
        let idx = self.layout_editor.selected;
        match self.layout_editor.column {
            EditorColumn::Left => {
                let len = self.active_view().left.len();
                if idx + 1 >= len {
                    return;
                }
                self.push_layout_undo();
                self.board_config.active_view_mut().left.swap(idx, idx + 1);
                self.layout_editor.selected += 1;
                self.finish_layout_mutation();
            }
            EditorColumn::Right => {
                let len = self.active_view().right.len();
                if idx + 1 >= len {
                    return;
                }
                self.push_layout_undo();
                self.board_config.active_view_mut().right.swap(idx, idx + 1);
                self.layout_editor.selected += 1;
                self.finish_layout_mutation();
            }
            EditorColumn::Available => {}
        }
    }

    /// Adjust the vertical weight of the selected pane in `Left` or `Right` (clamped to `5..=100`).
    pub fn adjust_selected_pane_weight(&mut self, delta: i16) {
        let idx = self.layout_editor.selected;
        let current_weight = match self.layout_editor.column {
            EditorColumn::Left => self.active_view().left.get(idx).map(|s| s.weight),
            EditorColumn::Right => self.active_view().right.get(idx).map(|s| s.weight),
            EditorColumn::Available => None,
        };
        let Some(current) = current_weight else {
            return;
        };
        let next = (current as i16 + delta)
            .clamp(MIN_EDITOR_PANE_WEIGHT as i16, MAX_PANE_WEIGHT as i16) as u16;
        if next == current {
            return;
        }
        self.push_layout_undo();
        let view = self.board_config.active_view_mut();
        match self.layout_editor.column {
            EditorColumn::Left => view.left[idx].weight = next,
            EditorColumn::Right => view.right[idx].weight = next,
            EditorColumn::Available => unreachable!(),
        }
        self.save_board_config();
    }

    /// Adjust the active view's `left_ratio` percentage (clamped to `20..=80`).
    pub fn adjust_left_ratio(&mut self, delta: i16) {
        let current = self.active_view().left_ratio;
        let next = (current as i16 + delta).clamp(MIN_LEFT_RATIO as i16, MAX_LEFT_RATIO as i16)
            as u16;
        if next == current {
            return;
        }
        self.push_layout_undo();
        self.board_config.active_view_mut().left_ratio = next;
        self.save_board_config();
    }

    /// Add a new view cloned from the default Overview layout and switch to it.
    pub fn add_view(&mut self) {
        self.push_layout_undo();
        let mut n = self.board_config.views.len() + 1;
        let name = loop {
            let candidate = format!("View {n}");
            if !self.board_config.views.iter().any(|v| v.name == candidate) {
                break candidate;
            }
            n += 1;
        };
        let mut new_view = BoardView::default_overview();
        new_view.name = name;
        self.board_config.views.push(new_view);
        self.board_config.active_view = self.board_config.views.len() - 1;
        self.layout_editor.grabbed = false;
        self.finish_layout_mutation();
    }

    /// Begin inline renaming of the active view.
    pub fn start_rename_view(&mut self) {
        self.layout_editor.grabbed = false;
        self.layout_editor.confirm_action = None;
        self.layout_editor.rename_buffer = Some(self.active_view().name.clone());
    }

    /// Commit the active view rename if non-empty, pushing an undo snapshot when changed.
    pub fn commit_rename_view(&mut self) {
        let Some(raw) = self.layout_editor.rename_buffer.take() else {
            return;
        };
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() || trimmed == self.active_view().name {
            return;
        }
        self.push_layout_undo();
        self.board_config.active_view_mut().name = trimmed;
        self.save_board_config();
    }

    /// Cancel inline view renaming without modifying `board_config`.
    pub fn cancel_rename_view(&mut self) {
        self.layout_editor.rename_buffer = None;
    }

    /// Prompt for confirmation before deleting the active view (blocked if only 1 view remains).
    pub fn request_delete_view(&mut self) {
        if self.board_config.views.len() <= 1 {
            self.message = Some("Cannot delete the last remaining view.".to_string());
            return;
        }
        self.layout_editor.grabbed = false;
        self.layout_editor.rename_buffer = None;
        self.layout_editor.confirm_action = Some(EditorConfirm::DeleteView);
    }

    /// Prompt for confirmation before resetting the layout views to default.
    pub fn request_reset_default(&mut self) {
        self.layout_editor.grabbed = false;
        self.layout_editor.rename_buffer = None;
        self.layout_editor.confirm_action = Some(EditorConfirm::ResetDefault);
    }

    /// Execute the pending [`EditorConfirm`] action.
    pub fn confirm_editor_action(&mut self) {
        let Some(action) = self.layout_editor.confirm_action.take() else {
            return;
        };
        self.layout_editor.grabbed = false;
        match action {
            EditorConfirm::DeleteView => {
                if self.board_config.views.len() <= 1 {
                    return;
                }
                self.push_layout_undo();
                let idx = self.active_view_index();
                self.board_config.views.remove(idx);
                if self.board_config.active_view >= self.board_config.views.len() {
                    self.board_config.active_view = self.board_config.views.len() - 1;
                }
                self.finish_layout_mutation();
            }
            EditorConfirm::ResetDefault => {
                self.push_layout_undo();
                let defaults = BoardConfig::default();
                self.board_config.views = defaults.views;
                self.board_config.active_view = defaults.active_view;
                self.finish_layout_mutation();
            }
        }
    }

    /// Cancel any pending [`EditorConfirm`] prompt.
    pub fn cancel_editor_action(&mut self) {
        self.layout_editor.confirm_action = None;
    }

    /// Restore the most recent layout snapshot from the undo stack.
    pub fn undo_layout_edit(&mut self) {
        let Some(prev) = self.layout_editor.undo_stack.pop() else {
            return;
        };
        self.board_config = prev;
        self.board_config.normalize();
        self.layout_editor.grabbed = false;
        self.finish_layout_mutation();
    }
}
