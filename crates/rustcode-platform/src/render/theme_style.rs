//! Maps VS Code `ColorTheme` colors into strongly typed `WorkbenchThemeStyles` with GPUI `Rgba`.

use crate::render::color::Rgba;
use crate::theme::ColorTheme;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkbenchThemeStyles {
    // Editor styles
    pub editor_background: Rgba,
    pub editor_foreground: Rgba,
    pub editor_line_number: Rgba,
    pub editor_line_number_active: Rgba,
    pub editor_cursor: Rgba,
    pub editor_selection: Rgba,
    pub editor_current_line: Rgba,
    pub editor_gutter_background: Rgba,
    pub editor_gutter_border: Rgba,
    pub editor_error_foreground: Rgba,
    pub editor_warning_foreground: Rgba,
    pub editor_info_foreground: Rgba,
    pub editor_hint_foreground: Rgba,

    // Activity bar styles
    pub activity_bar_bg: Rgba,
    pub activity_bar_fg: Rgba,
    pub activity_bar_inactive_fg: Rgba,
    pub activity_bar_active_border: Rgba,
    pub activity_bar_badge_bg: Rgba,
    pub activity_bar_badge_fg: Rgba,

    // Sidebar styles
    pub sidebar_bg: Rgba,
    pub sidebar_fg: Rgba,
    pub sidebar_border: Rgba,
    pub sidebar_header_fg: Rgba,

    // Status bar styles
    pub statusbar_bg: Rgba,
    pub statusbar_fg: Rgba,
    pub statusbar_item_hover_bg: Rgba,

    // Tabs styles
    pub tab_active_bg: Rgba,
    pub tab_active_fg: Rgba,
    pub tab_inactive_bg: Rgba,
    pub tab_inactive_fg: Rgba,
    pub tab_border: Rgba,

    // Command palette styles
    pub command_palette_bg: Rgba,
    pub command_palette_fg: Rgba,
    pub command_palette_border: Rgba,
    pub command_palette_selected_bg: Rgba,

    // List and general UI styles
    pub list_hover_bg: Rgba,
    pub list_active_selection_bg: Rgba,
    pub panel_bg: Rgba,
    pub panel_border: Rgba,
    pub sash_hover: Rgba,
}

impl Default for WorkbenchThemeStyles {
    fn default() -> Self {
        Self::dark_default()
    }
}

impl WorkbenchThemeStyles {
    pub fn dark_default() -> Self {
        Self {
            editor_background: Rgba::from_hex("#1e1e1e").unwrap(),
            editor_foreground: Rgba::from_hex("#d4d4d4").unwrap(),
            editor_line_number: Rgba::from_hex("#858585").unwrap(),
            editor_line_number_active: Rgba::from_hex("#c6c6c6").unwrap(),
            editor_cursor: Rgba::from_hex("#aeafad").unwrap(),
            editor_selection: Rgba::from_hex("#264f78").unwrap(),
            editor_current_line: Rgba::from_hex("#282828").unwrap(),
            editor_gutter_background: Rgba::from_hex("#1e1e1e").unwrap(),
            editor_gutter_border: Rgba::from_hex("#2d2d2d").unwrap(),
            editor_error_foreground: Rgba::from_hex("#f14c4c").unwrap(),
            editor_warning_foreground: Rgba::from_hex("#cca700").unwrap(),
            editor_info_foreground: Rgba::from_hex("#3794ff").unwrap(),
            editor_hint_foreground: Rgba::from_hex("#6c6c6c").unwrap(),

            activity_bar_bg: Rgba::from_hex("#333333").unwrap(),
            activity_bar_fg: Rgba::from_hex("#ffffff").unwrap(),
            activity_bar_inactive_fg: Rgba::from_hex("#888888").unwrap(),
            activity_bar_active_border: Rgba::from_hex("#007acc").unwrap(),
            activity_bar_badge_bg: Rgba::from_hex("#007acc").unwrap(),
            activity_bar_badge_fg: Rgba::from_hex("#ffffff").unwrap(),

            sidebar_bg: Rgba::from_hex("#252526").unwrap(),
            sidebar_fg: Rgba::from_hex("#cccccc").unwrap(),
            sidebar_border: Rgba::from_hex("#333333").unwrap(),
            sidebar_header_fg: Rgba::from_hex("#bbbbbb").unwrap(),

            statusbar_bg: Rgba::from_hex("#007acc").unwrap(),
            statusbar_fg: Rgba::from_hex("#ffffff").unwrap(),
            statusbar_item_hover_bg: Rgba::from_hex("#ffffff1f").unwrap(),

            tab_active_bg: Rgba::from_hex("#1e1e1e").unwrap(),
            tab_active_fg: Rgba::from_hex("#ffffff").unwrap(),
            tab_inactive_bg: Rgba::from_hex("#2d2d2d").unwrap(),
            tab_inactive_fg: Rgba::from_hex("#969696").unwrap(),
            tab_border: Rgba::from_hex("#252526").unwrap(),

            command_palette_bg: Rgba::from_hex("#252526").unwrap(),
            command_palette_fg: Rgba::from_hex("#cccccc").unwrap(),
            command_palette_border: Rgba::from_hex("#454545").unwrap(),
            command_palette_selected_bg: Rgba::from_hex("#094771").unwrap(),

            list_hover_bg: Rgba::from_hex("#2a2d2e").unwrap(),
            list_active_selection_bg: Rgba::from_hex("#094771").unwrap(),
            panel_bg: Rgba::from_hex("#1e1e1e").unwrap(),
            panel_border: Rgba::from_hex("#333333").unwrap(),
            sash_hover: Rgba::from_hex("#007acc").unwrap(),
        }
    }

    /// Resolve color with fallback.
    fn resolve(theme: &ColorTheme, key: &str, fallback: Rgba) -> Rgba {
        theme
            .colors
            .get(key)
            .and_then(|hex| Rgba::from_hex(hex))
            .unwrap_or(fallback)
    }

    /// Extract complete workbench style mapping from any VS Code `ColorTheme`.
    pub fn from_theme(theme: &ColorTheme) -> Self {
        let d = Self::dark_default();

        Self {
            editor_background: Self::resolve(theme, "editor.background", d.editor_background),
            editor_foreground: Self::resolve(theme, "editor.foreground", d.editor_foreground),
            editor_line_number: Self::resolve(theme, "editorLineNumber.foreground", d.editor_line_number),
            editor_line_number_active: Self::resolve(
                theme,
                "editorLineNumber.activeForeground",
                d.editor_line_number_active,
            ),
            editor_cursor: Self::resolve(theme, "editorCursor.foreground", d.editor_cursor),
            editor_selection: Self::resolve(theme, "editor.selectionBackground", d.editor_selection),
            editor_current_line: Self::resolve(theme, "editor.lineHighlightBackground", d.editor_current_line),
            editor_gutter_background: Self::resolve(
                theme,
                "editorGutter.background",
                Self::resolve(theme, "editor.background", d.editor_background),
            ),
            editor_gutter_border: Self::resolve(theme, "editorGroup.border", d.editor_gutter_border),
            editor_error_foreground: Self::resolve(theme, "editorError.foreground", d.editor_error_foreground),
            editor_warning_foreground: Self::resolve(theme, "editorWarning.foreground", d.editor_warning_foreground),
            editor_info_foreground: Self::resolve(theme, "editorInfo.foreground", d.editor_info_foreground),
            editor_hint_foreground: Self::resolve(theme, "editorHint.foreground", d.editor_hint_foreground),

            activity_bar_bg: Self::resolve(theme, "activityBar.background", d.activity_bar_bg),
            activity_bar_fg: Self::resolve(theme, "activityBar.foreground", d.activity_bar_fg),
            activity_bar_inactive_fg: Self::resolve(
                theme,
                "activityBar.inactiveForeground",
                d.activity_bar_inactive_fg,
            ),
            activity_bar_active_border: Self::resolve(
                theme,
                "activityBar.activeBorder",
                d.activity_bar_active_border,
            ),
            activity_bar_badge_bg: Self::resolve(theme, "activityBarBadge.background", d.activity_bar_badge_bg),
            activity_bar_badge_fg: Self::resolve(theme, "activityBarBadge.foreground", d.activity_bar_badge_fg),

            sidebar_bg: Self::resolve(theme, "sideBar.background", d.sidebar_bg),
            sidebar_fg: Self::resolve(theme, "sideBar.foreground", d.sidebar_fg),
            sidebar_border: Self::resolve(theme, "sideBar.border", d.sidebar_border),
            sidebar_header_fg: Self::resolve(theme, "sideBarSectionHeader.foreground", d.sidebar_header_fg),

            statusbar_bg: Self::resolve(theme, "statusBar.background", d.statusbar_bg),
            statusbar_fg: Self::resolve(theme, "statusBar.foreground", d.statusbar_fg),
            statusbar_item_hover_bg: Self::resolve(
                theme,
                "statusBarItem.hoverBackground",
                d.statusbar_item_hover_bg,
            ),

            tab_active_bg: Self::resolve(theme, "tab.activeBackground", d.tab_active_bg),
            tab_active_fg: Self::resolve(theme, "tab.activeForeground", d.tab_active_fg),
            tab_inactive_bg: Self::resolve(theme, "tab.inactiveBackground", d.tab_inactive_bg),
            tab_inactive_fg: Self::resolve(theme, "tab.inactiveForeground", d.tab_inactive_fg),
            tab_border: Self::resolve(theme, "tab.border", d.tab_border),

            command_palette_bg: Self::resolve(theme, "quickInput.background", d.command_palette_bg),
            command_palette_fg: Self::resolve(theme, "quickInput.foreground", d.command_palette_fg),
            command_palette_border: Self::resolve(theme, "widget.border", d.command_palette_border),
            command_palette_selected_bg: Self::resolve(
                theme,
                "quickInputList.focusBackground",
                d.command_palette_selected_bg,
            ),

            list_hover_bg: Self::resolve(theme, "list.hoverBackground", d.list_hover_bg),
            list_active_selection_bg: Self::resolve(
                theme,
                "list.activeSelectionBackground",
                d.list_active_selection_bg,
            ),
            panel_bg: Self::resolve(theme, "panel.background", d.panel_bg),
            panel_border: Self::resolve(theme, "panel.border", d.panel_border),
            sash_hover: Self::resolve(theme, "sash.hoverBorder", d.sash_hover),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeService;

    #[test]
    fn test_theme_styles_from_theme() {
        let theme = ThemeService::default_dark_theme();
        let styles = WorkbenchThemeStyles::from_theme(&theme);

        assert_eq!(styles.editor_background, Rgba::from_hex("#1e1e1e").unwrap());
        assert_eq!(styles.editor_foreground, Rgba::from_hex("#d4d4d4").unwrap());
        assert_eq!(styles.statusbar_bg, Rgba::from_hex("#007acc").unwrap());
        assert_eq!(styles.activity_bar_bg, Rgba::from_hex("#333333").unwrap());
    }
}
