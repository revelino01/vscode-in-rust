//! Source Control Management (SCM) view for the primary sidebar.
//!
//! Provides commit message input, Staged Changes, Changes, and Untracked trees
//! with staging/unstaging and commit execution (`Ctrl+Enter` or Commit button).

use parking_lot::RwLock;
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, Point, Quad, Rect,
    Scene, View, WorkbenchThemeStyles,
};
use rustcode_protocols::git::{
    GitBackend, GitCliBackend, GitError, GitStatusEntry, GitStatusKind, ScmRepository,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScmAction {
    Commit { message: String },
    StageFile(PathBuf),
    UnstageFile(PathBuf),
    StageAll,
    UnstageAll,
    DiscardChanges(PathBuf),
    OpenDiff {
        path: PathBuf,
        title: String,
        original: String,
        modified: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScmClickTarget {
    CommitButton,
    StageAllButton,
    UnstageAllButton,
    CommitInput,
    ToggleStaged,
    ToggleChanges,
    ToggleUntracked,
    StageItem(usize),
    UnstageItem(usize),
    StageUntrackedItem(usize),
    DiscardItem(usize),
    RowStaged(usize),
    RowChanges(usize),
    RowUntracked(usize),
}

pub struct ScmView {
    pub repo: ScmRepository,
    pub git_backend: Option<Arc<dyn GitBackend>>,
    pub commit_message: String,
    pub commit_input_focused: bool,
    pub staged_collapsed: bool,
    pub changes_collapsed: bool,
    pub untracked_collapsed: bool,
    pub scroll_top: f32,
    pub last_action: Option<ScmAction>,
    pub status_message: Option<String>,
    click_targets: RwLock<Vec<(Rect, ScmClickTarget)>>,
}

impl Default for ScmView {
    fn default() -> Self {
        Self::empty()
    }
}

impl ScmView {
    pub fn empty() -> Self {
        Self::new(ScmRepository::empty(PathBuf::from(".")))
    }

    pub fn new(repo: ScmRepository) -> Self {
        Self {
            repo,
            git_backend: None,
            commit_message: String::new(),
            commit_input_focused: false,
            staged_collapsed: false,
            changes_collapsed: false,
            untracked_collapsed: false,
            scroll_top: 0.0,
            last_action: None,
            status_message: None,
            click_targets: RwLock::new(Vec::new()),
        }
    }

    pub fn set_git_backend(&mut self, backend: Arc<dyn GitBackend>) {
        self.git_backend = Some(backend);
        if let Err(err) = self.refresh_from_git() {
            self.status_message = Some(err.to_string());
        }
    }

    pub fn clear_git_backend(&mut self) {
        self.git_backend = None;
        self.repo = ScmRepository::empty(PathBuf::from("."));
        self.status_message = None;
    }

    pub fn open_workspace_git(&mut self, workspace_root: &Path) -> Result<bool, GitError> {
        let backend = Arc::new(GitCliBackend::new(workspace_root));
        if !backend.is_git_repo() {
            self.git_backend = None;
            self.repo = ScmRepository::empty(workspace_root);
            self.status_message = None;
            return Ok(false);
        }
        self.git_backend = Some(backend);
        self.refresh_from_git()?;
        Ok(true)
    }

    pub fn refresh_from_git(&mut self) -> Result<(), GitError> {
        let Some(backend) = self.git_backend.as_ref().cloned() else {
            return Ok(());
        };
        let mut refreshed = ScmRepository::empty(backend.root());
        refreshed.refresh_from_backend(backend.as_ref())?;
        self.repo = refreshed;
        Ok(())
    }

    pub fn branch_name(&self) -> Option<&str> {
        let name = self.repo.current_branch.name.trim();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    pub fn stage_file(&mut self, path: &Path) -> Result<(), GitError> {
        if let Some(backend) = self.git_backend.as_ref().cloned() {
            backend.stage(path)?;
            self.refresh_from_git()?;
        } else {
            self.repo.stage_file(path);
        }
        self.last_action = Some(ScmAction::StageFile(path.to_path_buf()));
        Ok(())
    }

    pub fn unstage_file(&mut self, path: &Path) -> Result<(), GitError> {
        if let Some(backend) = self.git_backend.as_ref().cloned() {
            backend.unstage(path)?;
            self.refresh_from_git()?;
        } else {
            self.repo.unstage_file(path);
        }
        self.last_action = Some(ScmAction::UnstageFile(path.to_path_buf()));
        Ok(())
    }

    pub fn stage_all(&mut self) -> Result<(), GitError> {
        if let Some(backend) = self.git_backend.as_ref().cloned() {
            backend.stage_all()?;
            self.refresh_from_git()?;
        } else {
            self.repo.stage_all();
        }
        self.last_action = Some(ScmAction::StageAll);
        Ok(())
    }

    pub fn unstage_all(&mut self) -> Result<(), GitError> {
        if let Some(backend) = self.git_backend.as_ref().cloned() {
            backend.unstage_all()?;
            self.refresh_from_git()?;
        } else {
            self.repo.unstage_all();
        }
        self.last_action = Some(ScmAction::UnstageAll);
        Ok(())
    }

    pub fn discard_file(&mut self, path: &Path) -> Result<(), GitError> {
        if let Some(backend) = self.git_backend.as_ref().cloned() {
            backend.discard(path)?;
            self.refresh_from_git()?;
        } else {
            self.repo.discard_file(path);
        }
        self.last_action = Some(ScmAction::DiscardChanges(path.to_path_buf()));
        Ok(())
    }

    pub fn diff_for_file(&self, path: &Path, staged: bool) -> Result<String, GitError> {
        if let Some(backend) = &self.git_backend {
            backend.diff(path, staged)
        } else if let Some((orig, modif)) = self.repo.get_file_diff(path) {
            Ok(format!(
                "--- a/{}\n+++ b/{}\n- {}\n+ {}\n",
                path.display(),
                path.display(),
                orig,
                modif
            ))
        } else {
            Ok(String::new())
        }
    }

    pub fn set_commit_message(&mut self, msg: &str) {
        self.commit_message = msg.to_string();
    }

    pub fn commit_with_message(&mut self, message: &str) -> Result<String, String> {
        let msg = message.trim().to_string();
        if msg.is_empty() {
            let err = "Cannot commit empty message".to_string();
            self.status_message = Some(err.clone());
            return Err(err);
        }
        if let Some(backend) = self.git_backend.as_ref().cloned() {
            match backend.commit(&msg) {
                Ok(commit) => {
                    self.commit_message.clear();
                    if let Err(err) = self.refresh_from_git() {
                        self.status_message = Some(err.to_string());
                    } else {
                        self.status_message = Some(format!("Committed: {}", commit.hash));
                    }
                    self.last_action = Some(ScmAction::Commit { message: msg });
                    Ok(commit.hash)
                }
                Err(e) => {
                    let err = e.to_string();
                    self.status_message = Some(err.clone());
                    Err(err)
                }
            }
        } else {
            match self.repo.commit(&msg) {
                Ok(commit) => {
                    self.commit_message.clear();
                    self.status_message = Some(format!("Committed: {}", commit.hash));
                    self.last_action = Some(ScmAction::Commit { message: msg });
                    Ok(commit.hash)
                }
                Err(e) => {
                    self.status_message = Some(e.clone());
                    Err(e)
                }
            }
        }
    }

    pub fn execute_commit(&mut self) -> Result<String, String> {
        let msg = self.commit_message.clone();
        self.commit_with_message(&msg)
    }
}

impl View for ScmView {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        let mut targets = Vec::new();

        // 1. Sidebar Background & Border
        scene.push_quad(
            Quad::new(bounds)
                .with_background(theme.sidebar_bg)
                .with_border_edges(theme.sidebar_border, Edges::new(0.0, 1.0, 0.0, 0.0)),
        );

        // 2. Header (height 36.0)
        let header_h = 36.0;
        let header_rect = Rect::from_xywh(bounds.min_x(), bounds.min_y(), bounds.width(), header_h);
        scene.push_quad(
            Quad::new(header_rect)
                .with_background(theme.panel_bg)
                .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );

        // Title: "SOURCE CONTROL" or "SOURCE CONTROL: <branch>"
        let title_text = match self.branch_name() {
            Some(branch) => format!("SOURCE CONTROL: {}", branch),
            None => "SOURCE CONTROL".to_string(),
        };
        scene.push_text(GlyphRun::new(
            title_text.clone(),
            Point::new(bounds.min_x() + 12.0, bounds.min_y() + 10.0),
            Rect::from_xywh(bounds.min_x() + 12.0, bounds.min_y() + 10.0, 180.0, font.line_height),
            theme.sidebar_header_fg,
            font.font_size * 0.85,
        ));

        // Header Actions: Stage All [+] and Unstage All [-]
        let btn_stage_all = Rect::from_xywh(bounds.max_x() - 44.0, bounds.min_y() + 8.0, 18.0, 18.0);
        targets.push((btn_stage_all, ScmClickTarget::StageAllButton));
        scene.push_text(GlyphRun::new(
            "+".to_string(),
            Point::new(btn_stage_all.min_x() + 4.0, btn_stage_all.min_y()),
            btn_stage_all,
            theme.sidebar_header_fg,
            font.font_size,
        ));

        let btn_unstage_all = Rect::from_xywh(bounds.max_x() - 24.0, bounds.min_y() + 8.0, 18.0, 18.0);
        targets.push((btn_unstage_all, ScmClickTarget::UnstageAllButton));
        scene.push_text(GlyphRun::new(
            "-".to_string(),
            Point::new(btn_unstage_all.min_x() + 4.0, btn_unstage_all.min_y()),
            btn_unstage_all,
            theme.sidebar_header_fg,
            font.font_size,
        ));

        let mut cur_y = bounds.min_y() + header_h + 8.0;

        // 3. Commit Input Box
        let input_padding = 10.0;
        let input_w = bounds.width() - input_padding * 2.0;
        let input_h = 32.0;
        let input_rect = Rect::from_xywh(bounds.min_x() + input_padding, cur_y, input_w, input_h);
        targets.push((input_rect, ScmClickTarget::CommitInput));

        let input_border_color = if self.commit_input_focused {
            theme.command_palette_border
        } else {
            theme.panel_border
        };
        scene.push_quad(
            Quad::new(input_rect)
                .with_background(theme.command_palette_bg)
                .with_border_edges(input_border_color, Edges::all(1.0))
                .with_corner_radius(3.0),
        );

        if self.commit_message.is_empty() {
            scene.push_text(GlyphRun::new(
                "Message (Ctrl+Enter to commit)".to_string(),
                Point::new(input_rect.min_x() + 8.0, input_rect.min_y() + 7.0),
                Rect::from_xywh(input_rect.min_x() + 8.0, input_rect.min_y() + 7.0, input_w - 16.0, font.line_height),
                theme.sidebar_header_fg,
                font.font_size * 0.85,
            ));
        } else {
            scene.push_text(GlyphRun::new(
                self.commit_message.clone(),
                Point::new(input_rect.min_x() + 8.0, input_rect.min_y() + 7.0),
                Rect::from_xywh(input_rect.min_x() + 8.0, input_rect.min_y() + 7.0, input_w - 16.0, font.line_height),
                theme.editor_foreground,
                font.font_size * 0.9,
            ));
        }

        // Blinking / solid cursor if focused
        if self.commit_input_focused {
            let cursor_x = input_rect.min_x() + 8.0 + font.text_width(&self.commit_message);
            scene.push_quad(
                Quad::new(Rect::from_xywh(cursor_x, input_rect.min_y() + 6.0, 1.5, font.line_height))
                    .with_background(theme.editor_cursor),
            );
        }

        cur_y += input_h + 8.0;

        // 4. Commit Button
        let commit_btn_h = 26.0;
        let commit_btn_rect = Rect::from_xywh(bounds.min_x() + input_padding, cur_y, input_w, commit_btn_h);
        targets.push((commit_btn_rect, ScmClickTarget::CommitButton));

        scene.push_quad(
            Quad::new(commit_btn_rect)
                .with_background(theme.command_palette_selected_bg)
                .with_border_edges(theme.panel_border, Edges::all(1.0))
                .with_corner_radius(3.0),
        );

        let btn_label = "✔ Commit";
        let btn_w = font.text_width(btn_label);
        let btn_x = commit_btn_rect.min_x() + (input_w - btn_w) / 2.0;
        scene.push_text(GlyphRun::new(
            btn_label.to_string(),
            Point::new(btn_x, commit_btn_rect.min_y() + 4.0),
            Rect::from_xywh(btn_x, commit_btn_rect.min_y() + 4.0, btn_w, font.line_height),
            theme.editor_foreground,
            font.font_size * 0.9,
        ));

        cur_y += commit_btn_h + 8.0;

        // Status message (commit error or success hash)
        if let Some(status) = &self.status_message {
            let status_color = if status.starts_with("Committed:") {
                theme.editor_info_foreground
            } else {
                theme.editor_error_foreground
            };
            let status_w = font.text_width(status).min(bounds.width() - 20.0);
            scene.push_text(GlyphRun::new(
                status.clone(),
                Point::new(bounds.min_x() + input_padding, cur_y + 2.0),
                Rect::from_xywh(bounds.min_x() + input_padding, cur_y + 2.0, status_w, font.line_height),
                status_color,
                font.font_size * 0.8,
            ));
            cur_y += font.line_height + 4.0;
        }

        cur_y += 4.0;

        // 5. Tree Section: STAGED CHANGES
        let section_h = 24.0;
        let staged_hdr_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), section_h);
        targets.push((staged_hdr_rect, ScmClickTarget::ToggleStaged));

        scene.push_quad(Quad::new(staged_hdr_rect).with_background(theme.panel_bg));
        let arrow = if self.staged_collapsed { "▶" } else { "▼" };
        let staged_title = format!("{} STAGED CHANGES ({})", arrow, self.repo.staged.len());
        scene.push_text(GlyphRun::new(
            staged_title,
            Point::new(bounds.min_x() + 8.0, cur_y + 4.0),
            Rect::from_xywh(bounds.min_x() + 8.0, cur_y + 4.0, bounds.width() - 16.0, font.line_height),
            theme.sidebar_header_fg,
            font.font_size * 0.8,
        ));
        cur_y += section_h;

        if !self.staged_collapsed {
            for (idx, entry) in self.repo.staged.iter().enumerate() {
                let row_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), 22.0);
                targets.push((row_rect, ScmClickTarget::RowStaged(idx)));

                // Unstage button [-] on right
                let btn_unstage = Rect::from_xywh(bounds.max_x() - 24.0, cur_y + 2.0, 18.0, 18.0);
                targets.push((btn_unstage, ScmClickTarget::UnstageItem(idx)));
                scene.push_text(GlyphRun::new(
                    "-".to_string(),
                    Point::new(btn_unstage.min_x() + 4.0, btn_unstage.min_y()),
                    btn_unstage,
                    theme.sidebar_header_fg,
                    font.font_size * 0.85,
                ));

                self.render_file_row(row_rect, entry, bounds, theme, font, scene);
                cur_y += 22.0;
            }
        }

        cur_y += 6.0;

        // 6. Tree Section: CHANGES
        let changes_hdr_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), section_h);
        targets.push((changes_hdr_rect, ScmClickTarget::ToggleChanges));

        scene.push_quad(Quad::new(changes_hdr_rect).with_background(theme.panel_bg));
        let arrow = if self.changes_collapsed { "▶" } else { "▼" };
        let changes_title = format!("{} CHANGES ({})", arrow, self.repo.changes.len());
        scene.push_text(GlyphRun::new(
            changes_title,
            Point::new(bounds.min_x() + 8.0, cur_y + 4.0),
            Rect::from_xywh(bounds.min_x() + 8.0, cur_y + 4.0, bounds.width() - 16.0, font.line_height),
            theme.sidebar_header_fg,
            font.font_size * 0.8,
        ));
        cur_y += section_h;

        if !self.changes_collapsed {
            for (idx, entry) in self.repo.changes.iter().enumerate() {
                let row_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), 22.0);
                targets.push((row_rect, ScmClickTarget::RowChanges(idx)));

                // Stage button [+] on right
                let btn_stage = Rect::from_xywh(bounds.max_x() - 44.0, cur_y + 2.0, 18.0, 18.0);
                targets.push((btn_stage, ScmClickTarget::StageItem(idx)));
                scene.push_text(GlyphRun::new(
                    "+".to_string(),
                    Point::new(btn_stage.min_x() + 4.0, btn_stage.min_y()),
                    btn_stage,
                    theme.sidebar_header_fg,
                    font.font_size * 0.85,
                ));

                // Discard button [✕] on right
                let btn_discard = Rect::from_xywh(bounds.max_x() - 24.0, cur_y + 2.0, 18.0, 18.0);
                targets.push((btn_discard, ScmClickTarget::DiscardItem(idx)));
                scene.push_text(GlyphRun::new(
                    "✕".to_string(),
                    Point::new(btn_discard.min_x() + 4.0, btn_discard.min_y()),
                    btn_discard,
                    theme.sidebar_header_fg,
                    font.font_size * 0.75,
                ));

                self.render_file_row(row_rect, entry, bounds, theme, font, scene);
                cur_y += 22.0;
            }
        }

        cur_y += 6.0;

        // 7. Tree Section: UNTRACKED
        let untracked_hdr_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), section_h);
        targets.push((untracked_hdr_rect, ScmClickTarget::ToggleUntracked));

        scene.push_quad(Quad::new(untracked_hdr_rect).with_background(theme.panel_bg));
        let arrow = if self.untracked_collapsed { "▶" } else { "▼" };
        let untracked_title = format!("{} UNTRACKED ({})", arrow, self.repo.untracked.len());
        scene.push_text(GlyphRun::new(
            untracked_title,
            Point::new(bounds.min_x() + 8.0, cur_y + 4.0),
            Rect::from_xywh(bounds.min_x() + 8.0, cur_y + 4.0, bounds.width() - 16.0, font.line_height),
            theme.sidebar_header_fg,
            font.font_size * 0.8,
        ));
        cur_y += section_h;

        if !self.untracked_collapsed {
            for (idx, entry) in self.repo.untracked.iter().enumerate() {
                let row_rect = Rect::from_xywh(bounds.min_x(), cur_y, bounds.width(), 22.0);
                targets.push((row_rect, ScmClickTarget::RowUntracked(idx)));

                // Stage button [+] on right
                let btn_stage = Rect::from_xywh(bounds.max_x() - 24.0, cur_y + 2.0, 18.0, 18.0);
                targets.push((btn_stage, ScmClickTarget::StageUntrackedItem(idx)));
                scene.push_text(GlyphRun::new(
                    "+".to_string(),
                    Point::new(btn_stage.min_x() + 4.0, btn_stage.min_y()),
                    btn_stage,
                    theme.sidebar_header_fg,
                    font.font_size * 0.85,
                ));

                self.render_file_row(row_rect, entry, bounds, theme, font, scene);
                cur_y += 22.0;
            }
        }

        *self.click_targets.write() = targets;
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        match event {
            InputEvent::MouseDown(mouse) => {
                if !bounds.contains(mouse.position) {
                    return EventResult::Ignored;
                }

                let targets = self.click_targets.read().clone();
                // Check targets in reverse so small action buttons on rows have priority over the row itself
                for (rect, target) in targets.into_iter().rev() {
                    if rect.contains(mouse.position) {
                        match target {
                            ScmClickTarget::CommitInput => {
                                self.commit_input_focused = true;
                                return EventResult::Consumed;
                            }
                            ScmClickTarget::CommitButton => {
                                let _ = self.execute_commit();
                                return EventResult::Consumed;
                            }
                            ScmClickTarget::StageAllButton => {
                                if let Err(err) = self.stage_all() {
                                    self.status_message = Some(err.to_string());
                                }
                                return EventResult::Consumed;
                            }
                            ScmClickTarget::UnstageAllButton => {
                                if let Err(err) = self.unstage_all() {
                                    self.status_message = Some(err.to_string());
                                }
                                return EventResult::Consumed;
                            }
                            ScmClickTarget::ToggleStaged => {
                                self.staged_collapsed = !self.staged_collapsed;
                                return EventResult::Consumed;
                            }
                            ScmClickTarget::ToggleChanges => {
                                self.changes_collapsed = !self.changes_collapsed;
                                return EventResult::Consumed;
                            }
                            ScmClickTarget::ToggleUntracked => {
                                self.untracked_collapsed = !self.untracked_collapsed;
                                return EventResult::Consumed;
                            }
                            ScmClickTarget::StageItem(idx) => {
                                if let Some(entry) = self.repo.changes.get(idx).cloned() {
                                    if let Err(err) = self.stage_file(&entry.path) {
                                        self.status_message = Some(err.to_string());
                                    }
                                }
                                return EventResult::Consumed;
                            }
                            ScmClickTarget::UnstageItem(idx) => {
                                if let Some(entry) = self.repo.staged.get(idx).cloned() {
                                    if let Err(err) = self.unstage_file(&entry.path) {
                                        self.status_message = Some(err.to_string());
                                    }
                                }
                                return EventResult::Consumed;
                            }
                            ScmClickTarget::StageUntrackedItem(idx) => {
                                if let Some(entry) = self.repo.untracked.get(idx).cloned() {
                                    if let Err(err) = self.stage_file(&entry.path) {
                                        self.status_message = Some(err.to_string());
                                    }
                                }
                                return EventResult::Consumed;
                            }
                            ScmClickTarget::DiscardItem(idx) => {
                                if let Some(entry) = self.repo.changes.get(idx).cloned() {
                                    if let Err(err) = self.discard_file(&entry.path) {
                                        self.status_message = Some(err.to_string());
                                    }
                                }
                                return EventResult::Consumed;
                            }
                            ScmClickTarget::RowStaged(idx) => {
                                if let Some(entry) = self.repo.staged.get(idx).cloned() {
                                    self.open_diff_for_entry(&entry);
                                }
                                return EventResult::Consumed;
                            }
                            ScmClickTarget::RowChanges(idx) => {
                                if let Some(entry) = self.repo.changes.get(idx).cloned() {
                                    self.open_diff_for_entry(&entry);
                                }
                                return EventResult::Consumed;
                            }
                            ScmClickTarget::RowUntracked(idx) => {
                                if let Some(entry) = self.repo.untracked.get(idx).cloned() {
                                    self.open_diff_for_entry(&entry);
                                }
                                return EventResult::Consumed;
                            }
                        }
                    }
                }

                self.commit_input_focused = false;
                EventResult::Consumed
            }
            InputEvent::KeyDown(key) => {
                if !self.commit_input_focused {
                    return EventResult::Ignored;
                }

                // Ctrl+Enter commits
                if key.modifiers.ctrl && key.key == KeyCode::Enter {
                    let _ = self.execute_commit();
                    return EventResult::Consumed;
                }

                match key.key {
                    KeyCode::Backspace => {
                        self.commit_message.pop();
                        EventResult::Consumed
                    }
                    KeyCode::Escape => {
                        self.commit_input_focused = false;
                        EventResult::Consumed
                    }
                    KeyCode::Char(c) => {
                        if !key.modifiers.ctrl && !key.modifiers.alt {
                            self.commit_message.push(c);
                            EventResult::Consumed
                        } else {
                            EventResult::Ignored
                        }
                    }
                    _ => EventResult::Ignored,
                }
            }
            _ => EventResult::Ignored,
        }
    }
}

impl ScmView {
    fn render_file_row(
        &self,
        row_rect: Rect,
        entry: &GitStatusEntry,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        let file_name = entry
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| entry.path.to_string_lossy().to_string());

        let dir_path = entry
            .path
            .parent()
            .and_then(|p| p.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("");

        let text_x = bounds.min_x() + 20.0;
        let file_w = font.text_width(&file_name);

        // File name
        scene.push_text(GlyphRun::new(
            file_name,
            Point::new(text_x, row_rect.min_y() + 3.0),
            Rect::from_xywh(text_x, row_rect.min_y() + 3.0, file_w, font.line_height),
            theme.editor_foreground,
            font.font_size * 0.85,
        ));

        // Directory hint
        if !dir_path.is_empty() {
            let dir_x = text_x + file_w + 6.0;
            let dir_w = font.text_width(dir_path);
            scene.push_text(GlyphRun::new(
                dir_path.to_string(),
                Point::new(dir_x, row_rect.min_y() + 3.0),
                Rect::from_xywh(dir_x, row_rect.min_y() + 3.0, dir_w, font.line_height),
                theme.sidebar_header_fg,
                font.font_size * 0.75,
            ));
        }

        // Status badge: M / A / D / U
        let badge = entry.status.badge_str();
        let badge_color = match entry.status {
            GitStatusKind::Modified => theme.editor_warning_foreground,
            GitStatusKind::Added | GitStatusKind::Untracked => theme.editor_info_foreground,
            GitStatusKind::Deleted => theme.editor_error_foreground,
            _ => theme.sidebar_header_fg,
        };
        let badge_x = bounds.max_x() - 60.0;
        scene.push_text(GlyphRun::new(
            badge.to_string(),
            Point::new(badge_x, row_rect.min_y() + 3.0),
            Rect::from_xywh(badge_x, row_rect.min_y() + 3.0, 16.0, font.line_height),
            badge_color,
            font.font_size * 0.85,
        ));
    }

    fn file_diff_contents(&self, path: &Path) -> (String, String) {
        if let Some((o, m)) = self.repo.get_file_diff(path) {
            return (o.to_string(), m.to_string());
        }
        if let Some(backend) = &self.git_backend {
            let orig = backend.show_head(path).ok().flatten().unwrap_or_default();
            let full_path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                backend.root().join(path)
            };
            let modif = std::fs::read_to_string(&full_path).unwrap_or_default();
            return (orig, modif);
        }
        let full_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.repo.root.join(path)
        };
        let modif = std::fs::read_to_string(&full_path)
            .or_else(|_| std::fs::read_to_string(path))
            .unwrap_or_default();
        (String::new(), modif)
    }

    fn open_diff_for_entry(&mut self, entry: &GitStatusEntry) {
        let (orig, modif) = self.file_diff_contents(&entry.path);

        let file_name = entry
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "diff".to_string());
        let title = format!("{} (Working Tree)", file_name);

        self.last_action = Some(ScmAction::OpenDiff {
            path: entry.path.clone(),
            title,
            original: orig,
            modified: modif,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_protocols::git::InMemoryGitBackend;

    #[test]
    fn test_scm_view_default_is_empty_no_sample_data() {
        let scm = ScmView::default();
        assert!(scm.git_backend.is_none());
        assert!(scm.branch_name().is_none());
        assert!(scm.repo.staged.is_empty());
        assert!(scm.repo.changes.is_empty());
        assert!(scm.repo.untracked.is_empty());
        assert!(scm.repo.commits.is_empty());
    }

    #[test]
    fn test_scm_view_commit_and_staging_actions() {
        let mut scm = ScmView::default();
        let backend = Arc::new(InMemoryGitBackend::new(ScmRepository::sample_repository()));
        scm.set_git_backend(backend);
        assert_eq!(scm.branch_name(), Some("main"));
        assert_eq!(scm.repo.staged.len(), 1);
        assert_eq!(scm.repo.changes.len(), 1);
        assert_eq!(scm.repo.untracked.len(), 1);

        let bounds = Rect::from_xywh(0.0, 0.0, 300.0, 600.0);
        let font = FontMetrics::default_monospace();
        let theme = WorkbenchThemeStyles::dark_default();

        let mut scene = Scene::new();
        scm.render(bounds, &theme, &font, &mut scene);
        assert!(!scene.quads().is_empty());
        assert!(scene.contains_text("SOURCE CONTROL: main"));

        // Focus and type commit message
        scm.commit_input_focused = true;
        let key_feat = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Char('f'),
            rustcode_platform::keybindings::Modifiers::default(),
        ));
        scm.handle_event(&key_feat, bounds);
        assert_eq!(scm.commit_message, "f");

        // Type full message and press Ctrl+Enter
        scm.commit_message = "feat: initial commit".to_string();
        let enter_key = InputEvent::KeyDown(rustcode_platform::render::KeyEvent::new(
            KeyCode::Enter,
            rustcode_platform::keybindings::Modifiers {
                ctrl: true,
                ..Default::default()
            },
        ));
        let res = scm.handle_event(&enter_key, bounds);
        assert_eq!(res, EventResult::Consumed);
        assert!(scm.commit_message.is_empty());
        assert!(scm.repo.staged.is_empty());
        assert_eq!(scm.repo.commits.len(), 1);
    }

    #[test]
    fn test_scm_view_git_cli_backend_real_repo() {
        let tmp = std::env::temp_dir().join(format!("rc_scm_cli_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&tmp).unwrap();

        let run_git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&tmp)
                .args(args)
                .output()
                .expect("git should run");
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };

        run_git(&["init", "-b", "main"]);
        run_git(&["config", "user.name", "RustCode Test"]);
        run_git(&["config", "user.email", "test@rustcode.local"]);

        let tracked = tmp.join("hello.txt");
        std::fs::write(&tracked, "v1\n").unwrap();
        run_git(&["add", "hello.txt"]);
        run_git(&["commit", "-m", "initial"]);

        std::fs::write(&tracked, "v2\n").unwrap();
        std::fs::write(tmp.join("notes.txt"), "untracked\n").unwrap();

        let mut scm = ScmView::default();
        assert!(scm.open_workspace_git(&tmp).unwrap());
        assert_eq!(scm.branch_name(), Some("main"));
        assert_eq!(scm.repo.changes.len(), 1);
        assert_eq!(scm.repo.untracked.len(), 1);

        let diff = scm.diff_for_file(Path::new("hello.txt"), false).unwrap();
        assert!(diff.contains("-v1"));
        assert!(diff.contains("+v2"));

        scm.stage_file(Path::new("hello.txt")).unwrap();
        assert_eq!(scm.repo.staged.len(), 1);
        assert!(scm.repo.changes.is_empty());

        scm.unstage_file(Path::new("hello.txt")).unwrap();
        assert!(scm.repo.staged.is_empty());
        assert_eq!(scm.repo.changes.len(), 1);

        scm.stage_all().unwrap();
        assert_eq!(scm.repo.staged.len(), 2);
        let commit_hash = scm.commit_with_message("feat: update and add notes").unwrap();
        assert!(!commit_hash.is_empty());
        assert!(scm.repo.staged.is_empty());
        assert!(scm.repo.changes.is_empty());
        assert!(scm.repo.untracked.is_empty());
        assert_eq!(scm.repo.commits.len(), 2);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
