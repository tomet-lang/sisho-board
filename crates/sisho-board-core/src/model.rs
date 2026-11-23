//! State model and background worker coordination for `sisho-board`.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use sisho_board_sdk::{BoardItem, BoardPane};
use sisho_sdk::ExtensionInfo;

use crate::extension::{query_board_pane, query_info};
use crate::health::{self, HealthReport, PillarMetric};
use crate::links::{self, BrokenLink, Report};
use crate::runner::{Outcome, run_parallel_streaming};
use crate::tools::{Tool, Verdict, check_steps, fix_steps};
use crate::trust::{TrustConfig, TrustState, default_config_path};

/// An extension discovered on `PATH` and presented on the Board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionItem {
    /// Extension word (e.g. `"desk"` for `sisho-desk`).
    pub word: String,
    /// Full path to the executable on `PATH`.
    pub path: PathBuf,
    /// True if runnable as `sisho <word>` (not shadowed by a builtin).
    pub is_active: bool,
    /// Optional note such as `"shadowed by builtin"`.
    pub note: Option<String>,
    /// User trust state from `board.json` (`Untrusted`, `Allowed`, `Denied`).
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
        Self {
            word: word.into(),
            path: path.into(),
            is_active,
            note,
            trust: TrustState::Untrusted,
            info: None,
            board_pane: None,
            is_querying: false,
        }
    }
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

/// The currently focused pane on the Board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusedPane {
    #[default]
    Checks,
    Health,
    Extensions,
    BrokenLinks,
    ExtensionPanes,
    OutputPreview,
}

/// Which pane provides the content currently shown in the Output Preview pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewSource {
    #[default]
    CheckStep,
    HealthPillar,
    ExtensionItem,
}

/// Messages sent from background worker threads to the model.
pub enum WorkerMessage {
    CheckStepDone {
        index: usize,
        outcome: Outcome,
    },
    ChecksDone(Vec<Outcome>),
    LinksDone(Result<Report, String>),
    HealthDone(HealthReport),
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
    pub config_path: Option<PathBuf>,
    pub trust_config: TrustConfig,
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
    pub preview_scroll: usize,
    pub focused_pane: FocusedPane,
    pub preview_source: PreviewSource,
    pub confirm_fix: bool,
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
    /// Create a new `Model` using the default user config path (`~/.config/sisho/board.json`).
    pub fn new(dir: PathBuf, extensions: Vec<ExtensionItem>) -> Self {
        Self::with_config_path(dir, extensions, default_config_path())
    }

    /// Create a new `Model` with an explicit or overridden `board.json` path.
    pub fn with_config_path(
        dir: PathBuf,
        mut extensions: Vec<ExtensionItem>,
        config_path: Option<PathBuf>,
    ) -> Self {
        let trust_config = config_path
            .as_deref()
            .map(TrustConfig::load_from)
            .unwrap_or_default();

        for ext in &mut extensions {
            ext.trust = trust_config.trust_of(&ext.word);
        }

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
            config_path,
            trust_config,
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
            preview_scroll: 0,
            focused_pane: FocusedPane::default(),
            preview_source: PreviewSource::default(),
            confirm_fix: false,
            message: Some("Running checks...".to_string()),
            is_checking: false,
            is_checking_links: false,
            is_checking_health: false,
            is_fixing: false,
            tick: 0,
            sender,
            receiver,
        };
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
            let info = query_info(&path);
            let board_pane = query_board_pane(&path, &dir);
            let _ = sender.send(WorkerMessage::ExtensionQueried {
                word,
                info,
                board_pane,
            });
        });
    }

    /// Toggle trust state (`Untrusted` -> `Allowed` -> `Denied` -> `Allowed`)
    /// for the currently selected extension in `Tools & Extensions`, persist to
    /// `board.json`, and trigger or revoke extension data accordingly.
    pub fn toggle_selected_extension_trust(&mut self) {
        let Some(ext) = self.extensions.get_mut(self.selected_extension) else {
            return;
        };
        let new_trust = ext.trust.toggle();
        let word = ext.word.clone();
        let is_active = ext.is_active;
        ext.trust = new_trust;

        if new_trust != TrustState::Allowed {
            ext.info = None;
            ext.board_pane = None;
            ext.is_querying = false;
        }

        self.trust_config.set_trust(&word, new_trust);
        if let Some(path) = &self.config_path
            && let Err(err) = self.trust_config.save_to(path)
        {
            self.message = Some(format!("Failed to save board.json: {err}"));
            return;
        }

        match new_trust {
            TrustState::Allowed => {
                if is_active {
                    let idx = self.selected_extension;
                    self.spawn_extension_query(idx);
                    self.message = Some(format!(
                        "Allowed sisho-{word} (saved to board.json). Querying extension..."
                    ));
                } else {
                    self.message = Some(format!("Allowed sisho-{word} (saved to board.json)."));
                }
            }
            TrustState::Denied => {
                let total_items = self.extension_pane_items().len();
                if self.selected_ext_item >= total_items {
                    self.selected_ext_item = total_items.saturating_sub(1);
                }
                if !self.has_extension_panes() && self.preview_source == PreviewSource::ExtensionItem
                {
                    self.preview_source = PreviewSource::CheckStep;
                }
                self.message = Some(format!("Denied sisho-{word} (saved to board.json)."));
            }
            TrustState::Untrusted => {}
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
            let _ = sender.send(WorkerMessage::HealthDone(report));
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
                    self.health_report = Some(report);
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
                        }
                    }
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

    /// True if at least one allowed extension currently has a [`BoardPane`].
    pub fn has_extension_panes(&self) -> bool {
        self.extensions
            .iter()
            .any(|e| e.trust == TrustState::Allowed && e.board_pane.is_some())
    }

    /// Return all allowed extensions that have emitted a [`BoardPane`].
    pub fn active_extension_panes(&self) -> Vec<(&str, &BoardPane)> {
        self.extensions
            .iter()
            .filter(|e| e.trust == TrustState::Allowed)
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

    /// Currently selected item in [`FocusedPane::ExtensionPanes`].
    pub fn selected_extension_pane_item(&self) -> Option<ExtensionPaneItemRef<'_>> {
        let items = self.extension_pane_items();
        items.get(self.selected_ext_item).cloned()
    }

    /// Currently selected pillar in [`FocusedPane::Health`].
    pub fn selected_health_pillar_metric(&self) -> Option<&PillarMetric> {
        self.health_report
            .as_ref()
            .and_then(|r| r.pillars().get(self.selected_health_pillar).copied())
    }

    /// Return the preview text for the currently active [`PreviewSource`].
    pub fn selected_preview_text(&self) -> Option<String> {
        match self.preview_source {
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

    fn pane_order(&self) -> Vec<FocusedPane> {
        if self.has_extension_panes() {
            vec![
                FocusedPane::Checks,
                FocusedPane::Health,
                FocusedPane::ExtensionPanes,
                FocusedPane::Extensions,
                FocusedPane::BrokenLinks,
                FocusedPane::OutputPreview,
            ]
        } else {
            vec![
                FocusedPane::Checks,
                FocusedPane::Health,
                FocusedPane::Extensions,
                FocusedPane::BrokenLinks,
                FocusedPane::OutputPreview,
            ]
        }
    }

    pub fn next_pane(&mut self) {
        let order = self.pane_order();
        let pos = order
            .iter()
            .position(|&p| p == self.focused_pane)
            .unwrap_or(0);
        self.focused_pane = order[(pos + 1) % order.len()];
        self.sync_preview_source();
    }

    pub fn prev_pane(&mut self) {
        let order = self.pane_order();
        let pos = order
            .iter()
            .position(|&p| p == self.focused_pane)
            .unwrap_or(0);
        self.focused_pane = order[(pos + order.len() - 1) % order.len()];
        self.sync_preview_source();
    }

    fn sync_preview_source(&mut self) {
        match self.focused_pane {
            FocusedPane::Checks => {
                self.preview_source = PreviewSource::CheckStep;
                self.preview_scroll = 0;
            }
            FocusedPane::Health => {
                self.preview_source = PreviewSource::HealthPillar;
                self.preview_scroll = 0;
            }
            FocusedPane::ExtensionPanes => {
                self.preview_source = PreviewSource::ExtensionItem;
                self.preview_scroll = 0;
            }
            _ => {}
        }
    }

    pub fn move_selection_up(&mut self) {
        match self.focused_pane {
            FocusedPane::Checks => self.move_check_selection_up(),
            FocusedPane::Health => self.move_health_selection_up(),
            FocusedPane::Extensions => self.move_extension_selection_up(),
            FocusedPane::BrokenLinks => self.move_link_selection_up(),
            FocusedPane::ExtensionPanes => self.move_ext_item_selection_up(),
            FocusedPane::OutputPreview => self.scroll_preview_up(1),
        }
    }

    pub fn move_selection_down(&mut self) {
        match self.focused_pane {
            FocusedPane::Checks => self.move_check_selection_down(),
            FocusedPane::Health => self.move_health_selection_down(),
            FocusedPane::Extensions => self.move_extension_selection_down(),
            FocusedPane::BrokenLinks => self.move_link_selection_down(),
            FocusedPane::ExtensionPanes => self.move_ext_item_selection_down(),
            FocusedPane::OutputPreview => self.scroll_preview_down(1),
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
}
