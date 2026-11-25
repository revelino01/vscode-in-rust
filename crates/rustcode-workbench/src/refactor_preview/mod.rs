//! Multi-file Refactoring Preview Overlay and Atomic WorkspaceEdit Undo Stack.

use parking_lot::Mutex;
use rustcode_platform::render::{
    Edges, EventResult, FontMetrics, GlyphRun, InputEvent, KeyCode, MouseButton, Point, Quad, Rect,
    Rgba, Scene, Shadow, View, WorkbenchThemeStyles,
};
use rustcode_protocols::lsp::{DocumentChange, WorkspaceEdit};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEditPreview {
    pub uri: String,
    pub display_path: String,
    pub summary: String,
    pub edit_count: usize,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefactorPreviewAction {
    ApplySelected,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUndoSnapshot {
    pub uri: String,
    pub path: Option<PathBuf>,
    pub existed_before: bool,
    pub previous_content: Option<String>,
    pub was_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceEditUndoEntry {
    pub label: String,
    pub snapshots: Vec<FileUndoSnapshot>,
}

#[derive(Debug, Default)]
pub struct WorkspaceEditUndoStack {
    entries: Vec<WorkspaceEditUndoEntry>,
}

impl WorkspaceEditUndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, entry: WorkspaceEditUndoEntry) {
        if !entry.snapshots.is_empty() {
            self.entries.push(entry);
        }
    }

    pub fn pop(&mut self) -> Option<WorkspaceEditUndoEntry> {
        self.entries.pop()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

pub struct RefactorPreviewOverlay {
    pub is_visible: bool,
    pub title: String,
    pub edit: Option<WorkspaceEdit>,
    pub files: Vec<FileEditPreview>,
    pub selected_index: usize,
    pub last_action: Option<RefactorPreviewAction>,
    pub last_bounds: Mutex<Option<Rect>>,
}

impl Default for RefactorPreviewOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl RefactorPreviewOverlay {
    pub const HEADER_HEIGHT: f32 = 34.0;
    pub const ROW_HEIGHT: f32 = 26.0;
    pub const FOOTER_HEIGHT: f32 = 38.0;

    pub fn new() -> Self {
        Self {
            is_visible: false,
            title: String::new(),
            edit: None,
            files: Vec::new(),
            selected_index: 0,
            last_action: None,
            last_bounds: Mutex::new(None),
        }
    }

    fn display_name_for_uri(uri: &str) -> String {
        rustcode_platform::file_uri_to_path(uri)
            .file_name()
            .and_then(|n| n.to_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| uri.to_string())
    }

    pub fn open(&mut self, title: impl Into<String>, edit: WorkspaceEdit) {
        let grouped = edit.all_text_edits();
        let mut files: Vec<FileEditPreview> = grouped
            .into_iter()
            .map(|(uri, edits)| {
                let display_path = Self::display_name_for_uri(&uri);
                let sample = edits
                    .first()
                    .map(|e| format!("L{}: -> \"{}\"", e.range.start.line + 1, e.new_text))
                    .unwrap_or_default();
                FileEditPreview {
                    uri,
                    display_path,
                    summary: sample,
                    edit_count: edits.len(),
                    enabled: true,
                }
            })
            .collect();
        let mut seen_uris: HashSet<String> = files.iter().map(|f| f.uri.clone()).collect();
        if let Some(changes) = edit.document_changes.as_ref() {
            for change in changes {
                let (uri, summary) = match change {
                    DocumentChange::Edit(_) => continue,
                    DocumentChange::CreateFile { uri } => (uri.clone(), "Create file".to_string()),
                    DocumentChange::RenameFile { old_uri, new_uri } => (
                        old_uri.clone(),
                        format!("Rename -> {}", Self::display_name_for_uri(new_uri)),
                    ),
                    DocumentChange::DeleteFile { uri } => (uri.clone(), "Delete file".to_string()),
                };
                if seen_uris.insert(uri.clone()) {
                    let display_path = Self::display_name_for_uri(&uri);
                    files.push(FileEditPreview {
                        uri,
                        display_path,
                        summary,
                        edit_count: 1,
                        enabled: true,
                    });
                }
            }
        }
        files.sort_by(|a, b| a.display_path.cmp(&b.display_path));

        self.title = title.into();
        self.edit = Some(edit);
        self.files = files;
        self.selected_index = 0;
        self.last_action = None;
        self.is_visible = !self.files.is_empty();
    }

    pub fn close(&mut self) {
        self.is_visible = false;
        self.edit = None;
        self.files.clear();
        self.selected_index = 0;
        *self.last_bounds.lock() = None;
    }

    pub fn toggle_file(&mut self, idx: usize) {
        if let Some(f) = self.files.get_mut(idx) {
            f.enabled = !f.enabled;
        }
    }

    /// Build a filtered `WorkspaceEdit` containing only enabled files.
    pub fn filtered_workspace_edit(&self) -> Option<WorkspaceEdit> {
        let orig = self.edit.as_ref()?;
        let allowed: HashSet<&str> = self
            .files
            .iter()
            .filter(|f| f.enabled)
            .map(|f| f.uri.as_str())
            .collect();
        if allowed.is_empty() {
            return None;
        }

        let changes = orig.changes.as_ref().map(|map| {
            map.iter()
                .filter(|(uri, _)| allowed.contains(uri.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<HashMap<_, _>>()
        });
        let document_changes = orig.document_changes.as_ref().map(|dc| {
            dc.iter()
                .filter(|change| match change {
                    DocumentChange::Edit(e) => allowed.contains(e.text_document.uri.as_str()),
                    DocumentChange::CreateFile { uri } => allowed.contains(uri.as_str()),
                    DocumentChange::RenameFile { old_uri, new_uri } => {
                        allowed.contains(old_uri.as_str()) || allowed.contains(new_uri.as_str())
                    }
                    DocumentChange::DeleteFile { uri } => allowed.contains(uri.as_str()),
                })
                .cloned()
                .collect::<Vec<_>>()
        });
        Some(WorkspaceEdit {
            changes,
            document_changes,
        })
    }

    pub fn compute_bounds(&self, workbench_bounds: Rect) -> Option<Rect> {
        if !self.is_visible || !workbench_bounds.is_finite_positive() {
            return None;
        }
        let w = (workbench_bounds.width() * 0.62).clamp(320.0, 680.0);
        let max_h = (workbench_bounds.height() - 48.0).max(160.0);
        let h = (Self::HEADER_HEIGHT
            + Self::FOOTER_HEIGHT
            + (self.files.len().max(1) as f32) * Self::ROW_HEIGHT
            + 12.0)
            .clamp(160.0, max_h);
        let x = workbench_bounds.min_x() + (workbench_bounds.width() - w) * 0.5;
        let y = workbench_bounds.min_y() + (workbench_bounds.height() - h) * 0.5;
        let r = Rect::from_xywh(x, y, w, h);
        r.is_finite_positive().then_some(r)
    }
}

impl View for RefactorPreviewOverlay {
    fn render(
        &self,
        bounds: Rect,
        theme: &WorkbenchThemeStyles,
        font: &FontMetrics,
        scene: &mut Scene,
    ) {
        let Some(rect) = self.compute_bounds(bounds) else {
            return;
        };
        *self.last_bounds.lock() = Some(rect);

        scene.push_finite_shadow(Shadow {
            bounds: rect,
            offset: Point::new(0.0, 6.0),
            blur: 14.0,
            spread: 0.0,
            color: Rgba::new(0.0, 0.0, 0.0, 0.45),
        });
        scene.push_finite_quad(
            Quad::new(rect)
                .with_background(theme.command_palette_bg)
                .with_border_edges(theme.activity_bar_active_border, Edges::all(1.5))
                .with_corner_radius(4.0),
        );

        let header = Rect::from_xywh(rect.min_x(), rect.min_y(), rect.width(), Self::HEADER_HEIGHT);
        scene.push_finite_quad(
            Quad::new(header)
                .with_background(theme.panel_bg)
                .with_border_edges(theme.panel_border, Edges::new(0.0, 0.0, 1.0, 0.0)),
        );
        scene.push_finite_text(GlyphRun::new(
            self.title.clone(),
            Point::new(header.min_x() + 12.0, header.min_y() + 8.0),
            header,
            theme.editor_foreground,
            font.font_size,
        ));

        let rows_top = rect.min_y() + Self::HEADER_HEIGHT + 4.0;
        let rows_bottom = rect.max_y() - Self::FOOTER_HEIGHT;
        let max_rows = ((rows_bottom - rows_top).max(0.0) / Self::ROW_HEIGHT).floor() as usize;
        for (idx, file) in self.files.iter().take(max_rows.max(1)).enumerate() {
            let ry = rows_top + idx as f32 * Self::ROW_HEIGHT;
            let row_rect = Rect::from_xywh(rect.min_x() + 4.0, ry, rect.width() - 8.0, Self::ROW_HEIGHT);
            if idx == self.selected_index {
                scene.push_finite_quad(Quad::new(row_rect).with_background(theme.editor_selection));
            }
            let check = if file.enabled { "[x]" } else { "[ ]" };
            let text = format!(
                "{} {} ({} edits)  {}",
                check, file.display_path, file.edit_count, file.summary
            );
            scene.push_finite_text(GlyphRun::new(
                text,
                Point::new(row_rect.min_x() + 8.0, ry + 5.0),
                row_rect,
                theme.editor_foreground,
                font.font_size * 0.92,
            ));
        }

        // Footer buttons: Apply & Cancel
        let footer_y = rect.max_y() - Self::FOOTER_HEIGHT + 6.0;
        let apply_rect = Rect::from_xywh(rect.max_x() - 188.0, footer_y, 86.0, 26.0);
        let cancel_rect = Rect::from_xywh(rect.max_x() - 94.0, footer_y, 82.0, 26.0);
        scene.push_finite_quad(
            Quad::new(apply_rect)
                .with_background(theme.activity_bar_active_border)
                .with_corner_radius(3.0),
        );
        scene.push_finite_text(GlyphRun::new(
            "Apply".to_string(),
            Point::new(apply_rect.min_x() + 24.0, apply_rect.min_y() + 5.0),
            apply_rect,
            theme.editor_foreground,
            font.font_size * 0.92,
        ));
        scene.push_finite_quad(
            Quad::new(cancel_rect)
                .with_background(theme.panel_bg)
                .with_border_edges(theme.panel_border, Edges::all(1.0))
                .with_corner_radius(3.0),
        );
        scene.push_finite_text(GlyphRun::new(
            "Cancel".to_string(),
            Point::new(cancel_rect.min_x() + 18.0, cancel_rect.min_y() + 5.0),
            cancel_rect,
            theme.editor_foreground,
            font.font_size * 0.92,
        ));
    }

    fn handle_event(&mut self, event: &InputEvent, bounds: Rect) -> EventResult {
        let Some(rect) = self.compute_bounds(bounds) else {
            return EventResult::Ignored;
        };
        match event {
            InputEvent::MouseDown(mouse) if mouse.button == MouseButton::Left => {
                if !rect.contains(mouse.position) {
                    self.last_action = Some(RefactorPreviewAction::Cancel);
                    self.close();
                    return EventResult::Consumed;
                }
                let footer_y = rect.max_y() - Self::FOOTER_HEIGHT + 6.0;
                let apply_rect = Rect::from_xywh(rect.max_x() - 188.0, footer_y, 86.0, 26.0);
                let cancel_rect = Rect::from_xywh(rect.max_x() - 94.0, footer_y, 82.0, 26.0);
                if apply_rect.contains(mouse.position) {
                    self.last_action = Some(RefactorPreviewAction::ApplySelected);
                    return EventResult::Consumed;
                }
                if cancel_rect.contains(mouse.position) {
                    self.last_action = Some(RefactorPreviewAction::Cancel);
                    self.close();
                    return EventResult::Consumed;
                }
                let rows_top = rect.min_y() + Self::HEADER_HEIGHT + 4.0;
                if mouse.position.y >= rows_top && mouse.position.y < rect.max_y() - Self::FOOTER_HEIGHT
                {
                    let idx = ((mouse.position.y - rows_top) / Self::ROW_HEIGHT).floor().max(0.0) as usize;
                    if idx < self.files.len() {
                        self.selected_index = idx;
                        self.toggle_file(idx);
                    }
                }
                EventResult::Consumed
            }
            InputEvent::KeyDown(key) => match key.key {
                KeyCode::Escape => {
                    self.last_action = Some(RefactorPreviewAction::Cancel);
                    self.close();
                    EventResult::Consumed
                }
                KeyCode::Enter => {
                    self.last_action = Some(RefactorPreviewAction::ApplySelected);
                    EventResult::Consumed
                }
                KeyCode::Space => {
                    let idx = self.selected_index;
                    self.toggle_file(idx);
                    EventResult::Consumed
                }
                KeyCode::ArrowDown => {
                    if !self.files.is_empty() {
                        self.selected_index = (self.selected_index + 1) % self.files.len();
                    }
                    EventResult::Consumed
                }
                KeyCode::ArrowUp => {
                    if !self.files.is_empty() {
                        self.selected_index = if self.selected_index == 0 {
                            self.files.len() - 1
                        } else {
                            self.selected_index - 1
                        };
                    }
                    EventResult::Consumed
                }
                _ => EventResult::Consumed,
            },
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustcode_protocols::lsp::{LspPosition, LspRange, LspTextEdit};

    #[test]
    fn test_refactor_preview_filtering_and_undo_stack() {
        let mut changes = HashMap::new();
        changes.insert(
            "file:///src/a.rs".to_string(),
            vec![LspTextEdit {
                range: LspRange::new(LspPosition::new(0, 0), LspPosition::new(0, 3)),
                new_text: "bar".to_string(),
            }],
        );
        changes.insert(
            "file:///src/b.rs".to_string(),
            vec![LspTextEdit {
                range: LspRange::new(LspPosition::new(2, 0), LspPosition::new(2, 3)),
                new_text: "bar".to_string(),
            }],
        );
        let ws = WorkspaceEdit::with_changes(changes);
        let mut preview = RefactorPreviewOverlay::new();
        preview.open("Rename Preview: foo -> bar", ws);
        assert!(preview.is_visible);
        assert_eq!(preview.files.len(), 2);

        // Uncheck first file (`a.rs`), keep second (`b.rs`)
        preview.toggle_file(0);
        let filtered = preview.filtered_workspace_edit().unwrap();
        let edited_uris = filtered.all_text_edits();
        assert_eq!(edited_uris.len(), 1);
        assert!(edited_uris.contains_key("file:///src/b.rs"));

        let mut undo = WorkspaceEditUndoStack::new();
        undo.push(WorkspaceEditUndoEntry {
            label: "Rename foo -> bar".to_string(),
            snapshots: vec![FileUndoSnapshot {
                uri: "file:///src/b.rs".to_string(),
                path: Some(PathBuf::from("/src/b.rs")),
                existed_before: true,
                previous_content: Some("foo".to_string()),
                was_open: true,
            }],
        });
        assert_eq!(undo.len(), 1);
        assert!(undo.pop().is_some());
        assert!(undo.is_empty());
    }
}
