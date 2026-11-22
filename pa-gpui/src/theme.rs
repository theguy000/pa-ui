use gpui_kit::*;

#[derive(Clone, Copy)]
pub struct PaPalette {
    pub window_surface: Hsla,
    pub sidebar_surface: Hsla,
    pub sidebar_group_surface: Hsla,
    pub sidebar_group_edge: Hsla,
    pub sidebar_group_empty_edge: Hsla,
    pub sidebar_group_ink: Hsla,
    pub sidebar_group_empty_ink: Hsla,
    pub sidebar_row_hover: Hsla,
    pub sidebar_row_selected: Hsla,
    pub sidebar_new_fill: Hsla,
    pub sidebar_new_fill_hover: Hsla,
    pub sidebar_new_edge: Hsla,
    pub transcript_surface: Hsla,
    pub transcript_user_fill: Hsla,
    pub transcript_cell_fill: Hsla,
    pub transcript_cell_edge: Hsla,
    pub composer_surface: Hsla,
    pub composer_edge: Hsla,
    pub composer_edge_focus: Hsla,
    pub composer_handle: Hsla,
    pub composer_attach_fill: Hsla,
    pub composer_send_fill: Hsla,
    pub composer_send_mark: Hsla,
    pub panel_surface: Hsla,
    pub panel_edge: Hsla,
    pub ink: Hsla,
    pub quiet_ink: Hsla,
    pub working: Hsla,
    pub danger: Hsla,
    pub danger_surface: Hsla,
    pub tag_idle: Hsla,
    pub selection: Hsla,
}

impl PaPalette {
    pub fn for_mode(dark: bool) -> Self {
        if dark {
            Self::dark()
        } else {
            Self::light()
        }
    }

    pub fn dark() -> Self {
        Self {
            window_surface: rgb(0x09090b).into(),
            sidebar_surface: rgb(0x09090b).into(),
            sidebar_group_surface: rgb(0x111113).into(),
            sidebar_group_edge: rgba(0xffffff14).into(),
            sidebar_group_empty_edge: rgba(0xffffff0c).into(),
            sidebar_group_ink: rgb(0x9a9aa2).into(),
            sidebar_group_empty_ink: rgb(0x5c5c64).into(),
            sidebar_row_hover: rgba(0xffffff0d).into(),
            sidebar_row_selected: rgba(0xffffff16).into(),
            sidebar_new_fill: rgb(0x131316).into(),
            sidebar_new_fill_hover: rgb(0x1c1c20).into(),
            sidebar_new_edge: rgba(0xffffff18).into(),
            transcript_surface: rgb(0x0d0d0f).into(),
            transcript_user_fill: rgb(0x18181b).into(),
            transcript_cell_fill: rgb(0x131316).into(),
            transcript_cell_edge: rgb(0x232328).into(),
            composer_surface: rgb(0x111114).into(),
            composer_edge: rgb(0x27272c).into(),
            composer_edge_focus: rgb(0x52525b).into(),
            composer_handle: rgba(0xffffff26).into(),
            composer_attach_fill: rgba(0xffffff12).into(),
            composer_send_fill: rgb(0xf4f4f5).into(),
            composer_send_mark: rgb(0x09090b).into(),
            panel_surface: rgb(0x141417).into(),
            panel_edge: rgb(0x2b2b31).into(),
            ink: rgb(0xf4f4f5).into(),
            quiet_ink: rgb(0x9a9aa2).into(),
            working: rgb(0xd4b254).into(),
            danger: rgb(0xef4444).into(),
            danger_surface: rgba(0xef44441f).into(),
            tag_idle: rgb(0x4ade80).into(),
            selection: rgba(0xffffff24).into(),
        }
    }

    pub fn light() -> Self {
        Self {
            window_surface: rgb(0xf4f4f5).into(),
            sidebar_surface: rgb(0xf4f4f5).into(),
            sidebar_group_surface: rgb(0xededf0).into(),
            sidebar_group_edge: rgba(0x00000014).into(),
            sidebar_group_empty_edge: rgba(0x0000000c).into(),
            sidebar_group_ink: rgb(0x52525b).into(),
            sidebar_group_empty_ink: rgb(0x8e8e96).into(),
            sidebar_row_hover: rgba(0x0000000a).into(),
            sidebar_row_selected: rgba(0x00000014).into(),
            sidebar_new_fill: rgb(0xffffff).into(),
            sidebar_new_fill_hover: rgb(0xe4e4e7).into(),
            sidebar_new_edge: rgba(0x00000016).into(),
            transcript_surface: rgb(0xffffff).into(),
            transcript_user_fill: rgb(0xf4f4f5).into(),
            transcript_cell_fill: rgb(0xfafafa).into(),
            transcript_cell_edge: rgb(0xe4e4e7).into(),
            composer_surface: rgb(0xffffff).into(),
            composer_edge: rgb(0xd4d4d8).into(),
            composer_edge_focus: rgb(0x71717a).into(),
            composer_handle: rgba(0x00000024).into(),
            composer_attach_fill: rgba(0x0000000e).into(),
            composer_send_fill: rgb(0x18181b).into(),
            composer_send_mark: rgb(0xffffff).into(),
            panel_surface: rgb(0xffffff).into(),
            panel_edge: rgb(0xd4d4d8).into(),
            ink: rgb(0x09090b).into(),
            quiet_ink: rgb(0x71717a).into(),
            working: rgb(0xa18e41).into(),
            danger: rgb(0xdc2626).into(),
            danger_surface: rgba(0xdc262618).into(),
            tag_idle: rgb(0x16a34a).into(),
            selection: rgba(0x00000018).into(),
        }
    }
}

pub fn apply_theme(mode: gpui_kit::component::ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    use gpui_kit::component::Theme;

    Theme::change(mode, window, cx);
    let p = PaPalette::for_mode(mode.is_dark());
    Theme::update(cx, |theme| {
        theme.colors.background = p.window_surface;
        theme.colors.foreground = p.ink;
        theme.colors.border = p.transcript_cell_edge;
        theme.colors.muted = p.sidebar_group_surface;
        theme.colors.muted_foreground = p.quiet_ink;
        theme.colors.list_hover = p.sidebar_row_hover;
        theme.colors.accent = p.sidebar_row_selected;
        theme.colors.accent_foreground = p.ink;
        theme.colors.primary = p.composer_send_fill;
        theme.colors.primary_foreground = p.composer_send_mark;
        theme.colors.secondary = p.sidebar_group_surface;
        theme.colors.secondary_foreground = p.ink;
        theme.colors.popover = p.panel_surface;
        theme.colors.popover_foreground = p.ink;
        theme.colors.sidebar = p.sidebar_surface;
        theme.colors.scrollbar_thumb = p.composer_handle;
        theme.colors.success = p.tag_idle;
        theme.colors.danger = p.danger;
        theme.colors.warning = p.working;
        theme.colors.selection = p.selection;
    });
}
