//! Transcript view module: card icon animations, markdown blocks with Copy buttons,
//! optically aligned status badges (`exit`, `running`, elapsed), and transcript find bar.

use crate::app::PaApp;
use crate::assets::{
    ICON_BASH_CHEVRON_PATH, ICON_BASH_CURSOR_PATH, ICON_BASH_PATH, ICON_PYTHON_L_PATH,
    ICON_PYTHON_M_PATH, ICON_PYTHON_PATH, ICON_PYTHON_R_PATH, ICON_THINKING_PATH,
};
use crate::transcript::{CardVariant, ItemKind, TranscriptItem};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    text::{TextView, TextViewState},
    v_flex, Icon, IconName, Sizable as _, StyledExt as _,
};
use gpui_kit::{prelude::FluentBuilder as _, *};

pub(crate) fn lerp_hsla(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let t = t.clamp(0.0, 1.0);
    let ra: Rgba = a.into();
    let rb: Rgba = b.into();
    Rgba {
        r: ra.r + (rb.r - ra.r) * t,
        g: ra.g + (rb.g - ra.g) * t,
        b: ra.b + (rb.b - ra.b) * t,
        a: ra.a + (rb.a - ra.a) * t,
    }
    .into()
}

pub(crate) fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

pub(crate) fn python_ripple(phase_ms: u64, offset_ms: u64) -> f32 {
    let pos = ((phase_ms + 1600 - offset_ms) % 1600) as f32 / 1600.0;
    0.5 - 0.5 * (std::f32::consts::TAU * pos).cos()
}

pub(crate) fn bash_cursor_mix(phase_ms: u64) -> f32 {
    let cycle = (phase_ms % 1200) as f32 / 1200.0;
    if cycle <= 0.4 {
        1.0
    } else if cycle <= 0.55 {
        0.5 + 0.5 * (std::f32::consts::PI * (cycle - 0.4) / 0.15).cos()
    } else if cycle <= 0.85 {
        0.0
    } else {
        0.5 - 0.5 * (std::f32::consts::PI * (cycle - 0.85) / 0.15).cos()
    }
}

pub(crate) fn thinking_sweep(phase_ms: u64) -> f32 {
    let pos = (phase_ms % 1450) as f32 / 1450.0;
    0.5 - 0.5 * (std::f32::consts::PI * pos).cos()
}

pub(crate) fn pulse_mix(phase_ms: u64) -> f32 {
    let pos = (phase_ms % 1400) as f32 / 1400.0;
    0.5 - 0.5 * (std::f32::consts::TAU * pos).cos()
}

impl PaApp {
    pub(crate) fn render_find_bar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.find_open {
            return None;
        }
        let p = self.palette();
        let matches = self.find_matches(cx);
        let count_label = if matches.is_empty() {
            "0 matches".to_string()
        } else {
            format!(
                "{}/{}",
                (self.find_index % matches.len()) + 1,
                matches.len()
            )
        };
        Some(
            h_flex()
                .w_full()
                .px_5()
                .py_1p5()
                .border_b_1()
                .border_color(p.transcript_cell_edge)
                .bg(p.transcript_cell_fill)
                .gap_2()
                .items_center()
                .child(
                    div()
                        .w(px(260.))
                        .child(self.render_slim_input(&self.find_input, window, cx)),
                )
                .child(div().text_xs().text_color(p.quiet_ink).child(count_label))
                .child(
                    Button::new("find-prev")
                        .ghost()
                        .xsmall()
                        .label("Prev")
                        .on_click(cx.listener(|this, _, _, cx| this.step_find(-1, cx))),
                )
                .child(
                    Button::new("find-next")
                        .ghost()
                        .xsmall()
                        .label("Next")
                        .on_click(cx.listener(|this, _, _, cx| this.step_find(1, cx))),
                )
                .child(
                    Button::new("find-close")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.find_open = false;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    pub(crate) fn render_card_icon(&self, item: &TranscriptItem, color: Hsla) -> AnyElement {
        let p = self.palette();
        let phase = self.anim_phase_ms;
        let working_dim = p.working.opacity(0.28);

        match (item.kind, item.variant) {
            (ItemKind::Thinking, _) if item.active => {
                let win_x = -7.0 + 21.0 * thinking_sweep(phase);
                div()
                    .relative()
                    .size(px(14.))
                    .overflow_hidden()
                    .child(
                        svg()
                            .path(ICON_THINKING_PATH)
                            .size(px(14.))
                            .text_color(working_dim),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left(px(win_x))
                            .w(px(7.))
                            .h(px(14.))
                            .overflow_hidden()
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left(px(-win_x))
                                    .size(px(14.))
                                    .child(
                                        svg()
                                            .path(ICON_THINKING_PATH)
                                            .size(px(14.))
                                            .text_color(p.working),
                                    ),
                            ),
                    )
                    .into_any_element()
            }
            (ItemKind::Thinking, _) => svg()
                .path(ICON_THINKING_PATH)
                .size(px(14.))
                .text_color(color)
                .into_any_element(),
            (ItemKind::ToolCall, CardVariant::Bash) | (_, CardVariant::BashDone) if item.active => {
                let cursor_color = lerp_hsla(p.working, working_dim, bash_cursor_mix(phase));
                div()
                    .relative()
                    .size(px(14.))
                    .child(
                        div().absolute().inset_0().child(
                            svg()
                                .path(ICON_BASH_CHEVRON_PATH)
                                .size(px(14.))
                                .text_color(p.working),
                        ),
                    )
                    .child(
                        div().absolute().inset_0().child(
                            svg()
                                .path(ICON_BASH_CURSOR_PATH)
                                .size(px(14.))
                                .text_color(cursor_color),
                        ),
                    )
                    .into_any_element()
            }
            (ItemKind::ToolCall, CardVariant::Bash) | (_, CardVariant::BashDone) => svg()
                .path(ICON_BASH_PATH)
                .size(px(14.))
                .text_color(color)
                .into_any_element(),
            (ItemKind::ToolCall, _) if item.active => {
                let c_l = lerp_hsla(p.working, working_dim, python_ripple(phase, 0));
                let c_m = lerp_hsla(p.working, working_dim, python_ripple(phase, 320));
                let c_r = lerp_hsla(p.working, working_dim, python_ripple(phase, 640));
                div()
                    .relative()
                    .size(px(14.))
                    .child(
                        div().absolute().inset_0().child(
                            svg()
                                .path(ICON_PYTHON_L_PATH)
                                .size(px(14.))
                                .text_color(c_l),
                        ),
                    )
                    .child(
                        div().absolute().inset_0().child(
                            svg()
                                .path(ICON_PYTHON_M_PATH)
                                .size(px(14.))
                                .text_color(c_m),
                        ),
                    )
                    .child(
                        div().absolute().inset_0().child(
                            svg()
                                .path(ICON_PYTHON_R_PATH)
                                .size(px(14.))
                                .text_color(c_r),
                        ),
                    )
                    .into_any_element()
            }
            (ItemKind::ToolCall, _) => svg()
                .path(ICON_PYTHON_PATH)
                .size(px(14.))
                .text_color(color)
                .into_any_element(),
            (_, CardVariant::AgentMessage) => {
                let tint = if item.active {
                    lerp_hsla(p.working, working_dim, pulse_mix(phase))
                } else {
                    color
                };
                Icon::new(IconName::Bot)
                    .size(px(14.))
                    .text_color(tint)
                    .into_any_element()
            }
            (_, CardVariant::ProviderRetry) => {
                let mut icon = Icon::new(IconName::Loader)
                    .size(px(14.))
                    .text_color(color);
                if item.active {
                    let angle = ((phase % 1000) as f32 / 1000.0) * std::f32::consts::TAU;
                    icon = icon.rotate(radians(angle));
                }
                icon.into_any_element()
            }
            _ => {
                let tint = if item.active {
                    lerp_hsla(p.working, working_dim, pulse_mix(phase))
                } else {
                    color
                };
                Icon::new(IconName::Inbox)
                    .size(px(14.))
                    .text_color(tint)
                    .into_any_element()
            }
        }
    }

    pub(crate) fn render_markdown(
        &self,
        md: &Entity<TextViewState>,
        item_id: usize,
        stream_fade: bool,
    ) -> TextView {
        let p = self.palette();
        TextView::new(md)
            .selectable(true)
            .stream_fade(stream_fade)
            .code_block_actions(move |block, _window, _cx| {
                use std::hash::{Hash as _, Hasher as _};
                let code = block.code();
                let lang = block.lang().filter(|l| !l.is_empty());
                let mut h = std::collections::hash_map::DefaultHasher::new();
                item_id.hash(&mut h);
                code.hash(&mut h);
                let btn_id = h.finish() as usize;
                h_flex()
                    .items_center()
                    .gap(px(6.))
                    .when_some(lang, |row, l| {
                        row.child(
                            div()
                                .text_size(px(10.5))
                                .text_color(p.sidebar_group_empty_ink)
                                .child(l),
                        )
                    })
                    .child(
                        div()
                            .id(("code-copy-btn", btn_id))
                            .size(px(22.))
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|d| d.bg(p.sidebar_row_hover))
                            .child(
                                Icon::new(IconName::Copy)
                                    .size(px(12.))
                                    .text_color(p.quiet_ink),
                            )
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()));
                            }),
                    )
            })
    }

    pub(crate) fn render_item(&self, item: &TranscriptItem, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        match item.kind {
            ItemKind::User => v_flex()
                .w_full()
                .rounded(px(10.))
                .bg(p.transcript_user_fill)
                .px_4()
                .py_3()
                .gap_2()
                .children(
                    item.body_md
                        .as_ref()
                        .map(|md| self.render_markdown(md, item.id, false)),
                )
                .when(!item.images.is_empty(), |col| {
                    col.child(
                        h_flex().gap_2().flex_wrap().children(
                            item.images.iter().map(|ti| {
                                let (w, h) = crate::transcript::fitted_image_size(
                                    ti.width, ti.height, 360.0, 220.0,
                                );
                                div()
                                    .w(px(w))
                                    .h(px(h))
                                    .rounded(px(8.))
                                    .border_1()
                                    .border_color(p.transcript_cell_edge)
                                    .bg(p.sidebar_group_surface)
                                    .overflow_hidden()
                                    .child(
                                        img(ti.image.clone())
                                            .size_full()
                                            .object_fit(ObjectFit::Contain),
                                    )
                            }),
                        ),
                    )
                })
                .into_any_element(),
            ItemKind::Assistant => div()
                .w_full()
                .py_1()
                .children(
                    item.body_md
                        .as_ref()
                        .map(|md| self.render_markdown(md, item.id, item.active)),
                )
                .into_any_element(),
            ItemKind::Error => div()
                .w_full()
                .rounded(px(8.))
                .border_1()
                .border_color(p.danger)
                .bg(p.danger_surface)
                .px_3()
                .py_2()
                .text_sm()
                .text_color(p.ink)
                .child(item.raw_body.clone())
                .into_any_element(),
            ItemKind::Thinking | ItemKind::ToolCall | ItemKind::Summary | ItemKind::Custom => {
                let id = item.id;
                let has_body = !item.raw_body.trim().is_empty();
                let icon_color = if item.active {
                    p.working
                } else if item.failed {
                    p.danger
                } else {
                    p.quiet_ink
                };

                let (border_color, card_bg) = if item.active {
                    (
                        lerp_hsla(p.transcript_cell_edge, p.working, 0.10),
                        lerp_hsla(p.transcript_cell_fill, p.working, 0.045),
                    )
                } else {
                    (p.transcript_cell_edge, p.transcript_cell_fill)
                };

                v_flex()
                    .w_full()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border_color)
                    .bg(card_bg)
                    .child(
                        h_flex()
                            .id(("card-hdr", id))
                            .h(px(30.))
                            .px_3()
                            .gap_2()
                            .items_center()
                            .when(has_body, |row| {
                                row.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle_item(id, cx);
                                }))
                            })
                            .child(
                                div()
                                    .size(px(14.))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(self.render_card_icon(item, icon_color)),
                            )
                            .when(!item.verb.is_empty(), |row| {
                                row.child(
                                    div()
                                        .text_xs()
                                        .line_height(px(14.))
                                        .font_medium()
                                        .text_color(if item.active {
                                            p.working
                                        } else {
                                            p.quiet_ink
                                        })
                                        .child(item.verb.clone()),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .line_height(px(14.))
                                    .text_color(if item.failed { p.danger } else { p.ink })
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(item.title.clone()),
                            )
                            .when(!item.meta.is_empty(), |row| {
                                // Optically centered 18px badge: pb(1px) compensates for DirectWrite
                                // lowercase ascender headroom so `exit 1` and `running` sit dead-center.
                                let badge = if item.failed {
                                    div()
                                        .h(px(18.))
                                        .px(px(6.5))
                                        .pb(px(1.))
                                        .rounded(px(4.5))
                                        .bg(p.danger_surface)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_size(px(10.5))
                                        .line_height(px(11.))
                                        .text_color(p.danger)
                                        .child(item.meta.clone())
                                } else if item.active {
                                    div()
                                        .h(px(18.))
                                        .px(px(6.5))
                                        .pb(px(1.))
                                        .rounded(px(4.5))
                                        .bg(p.working.opacity(0.14))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_size(px(10.5))
                                        .line_height(px(11.))
                                        .text_color(p.working)
                                        .child(item.meta.clone())
                                } else {
                                    div()
                                        .h(px(18.))
                                        .pb(px(0.5))
                                        .flex()
                                        .items_center()
                                        .text_size(px(11.))
                                        .line_height(px(12.))
                                        .text_color(p.quiet_ink)
                                        .child(item.meta.clone())
                                };
                                row.child(badge)
                            })
                            .when(has_body, |row| {
                                row.child(
                                    Icon::new(if item.expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .size(px(12.))
                                    .text_color(p.quiet_ink),
                                )
                            }),
                    )
                    .when(item.expanded && has_body, |card| {
                        card.child(
                            h_flex()
                                .w_full()
                                .border_t_1()
                                .border_color(p.transcript_cell_edge)
                                .items_stretch()
                                .child(
                                    div()
                                        .id(("card-rail", id))
                                        .group("card-rail")
                                        .w(px(16.))
                                        .flex_shrink_0()
                                        .py_2()
                                        .flex()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|d| d.bg(p.sidebar_row_hover))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.toggle_item(id, cx);
                                        }))
                                        .child(
                                            div()
                                                .w(px(2.))
                                                .h_full()
                                                .rounded(px(1.))
                                                .bg(p.transcript_cell_edge)
                                                .group_hover("card-rail", |s| s.bg(p.working)),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .pl_1()
                                        .pr_3()
                                        .py_2()
                                        .children(
                                            item.body_md
                                                .as_ref()
                                                .map(|md| self.render_markdown(md, id, false)),
                                        ),
                                ),
                        )
                    })
                    .into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn row_icon_animation_curves_stay_bounded_and_vary() {
        for ms in [0u64, 200, 400, 600, 800, 1000, 1200, 1450] {
            let r = python_ripple(ms, 320);
            let b = bash_cursor_mix(ms);
            let t = thinking_sweep(ms);
            let p = pulse_mix(ms);
            assert!((0.0..=1.0).contains(&r));
            assert!((0.0..=1.0).contains(&b));
            assert!((0.0..=1.0).contains(&t));
            assert!((0.0..=1.0).contains(&p));
        }
        assert!(bash_cursor_mix(100) > 0.99);
        assert!(bash_cursor_mix(800) < 0.01);
        assert!((python_ripple(0, 0) - python_ripple(0, 320)).abs() > 0.1);
    }
}
