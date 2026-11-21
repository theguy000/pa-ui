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
            window_surface: rgb(0x10110e).into(),
            sidebar_surface: rgb(0x10110e).into(),
            sidebar_group_surface: rgb(0x151613).into(),
            sidebar_group_edge: rgba(0xe7e5df16).into(),
            sidebar_group_empty_edge: rgba(0xe7e5df10).into(),
            sidebar_group_ink: rgb(0x9a9b92).into(),
            sidebar_group_empty_ink: rgb(0x63645e).into(),
            sidebar_row_hover: rgba(0xe7e5df0d).into(),
            sidebar_row_selected: rgba(0xe7e5df18).into(),
            sidebar_new_fill: rgba(0xe7e5df0c).into(),
            sidebar_new_fill_hover: rgba(0xe7e5df16).into(),
            sidebar_new_edge: rgba(0xe7e5df18).into(),
            transcript_surface: rgb(0x141512).into(),
            transcript_user_fill: rgb(0x1b1c18).into(),
            transcript_cell_fill: rgb(0x181915).into(),
            transcript_cell_edge: rgb(0x262722).into(),
            composer_surface: rgb(0x0f100d).into(),
            composer_edge: rgb(0x272823).into(),
            composer_handle: rgba(0xe7e5df28).into(),
            composer_attach_fill: rgba(0xe7e5df10).into(),
            composer_send_fill: rgb(0xa18e41).into(),
            composer_send_mark: rgb(0x12130f).into(),
            panel_surface: rgb(0x171814).into(),
            panel_edge: rgb(0x2d2e28).into(),
            ink: rgb(0xe7e5df).into(),
            quiet_ink: rgb(0x9a9b92).into(),
            working: rgb(0xd4b254).into(),
            danger: rgb(0xc42b1c).into(),
            danger_surface: rgba(0xc42b1c1f).into(),
            tag_idle: rgb(0x6d9e7c).into(),
        }
    }

    pub fn light() -> Self {
        Self {
            window_surface: rgb(0xefeee9).into(),
            sidebar_surface: rgb(0xefeee9).into(),
            sidebar_group_surface: rgb(0xe8e7e1).into(),
            sidebar_group_edge: rgba(0x1a1b1818).into(),
            sidebar_group_empty_edge: rgba(0x1a1b1810).into(),
            sidebar_group_ink: rgb(0x5b5c55).into(),
            sidebar_group_empty_ink: rgb(0x8e8f87).into(),
            sidebar_row_hover: rgba(0x1a1b180c).into(),
            sidebar_row_selected: rgba(0x1a1b1816).into(),
            sidebar_new_fill: rgba(0x1a1b180b).into(),
            sidebar_new_fill_hover: rgba(0x1a1b1814).into(),
            sidebar_new_edge: rgba(0x1a1b1818).into(),
            transcript_surface: rgb(0xf8f7f3).into(),
            transcript_user_fill: rgb(0xeeede8).into(),
            transcript_cell_fill: rgb(0xf1f0eb).into(),
            transcript_cell_edge: rgb(0xdad9d2).into(),
            composer_surface: rgb(0xffffff).into(),
            composer_edge: rgb(0xd5d4cd).into(),
            composer_handle: rgba(0x1a1b1826).into(),
            composer_attach_fill: rgba(0x1a1b180e).into(),
            composer_send_fill: rgb(0xa18e41).into(),
            composer_send_mark: rgb(0x12130f).into(),
            panel_surface: rgb(0xffffff).into(),
            panel_edge: rgb(0xd5d4cd).into(),
            ink: rgb(0x1a1b18).into(),
            quiet_ink: rgb(0x61625b).into(),
            working: rgb(0xa18e41).into(),
            danger: rgb(0xc42b1c).into(),
            danger_surface: rgba(0xc42b1c18).into(),
            tag_idle: rgb(0x3a7a52).into(),
        }
    }
}
