//! Pure appearance tokens mapped from persistent Settings.

use crate::settings::Settings;
use crate::{AppWindow, Appearance, Type};
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use serde_json::Value;
use slint::ComponentHandle;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum InterfaceSize {
    Compact,
    Default,
    Large,
}

impl InterfaceSize {
    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "Compact" => Self::Compact,
            "Large" => Self::Large,
            _ => Self::Default,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CodeFont {
    CascadiaCode,
    Consolas,
    JetBrainsMono,
}

impl CodeFont {
    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "Consolas" => Self::Consolas,
            "JetBrains Mono" => Self::JetBrainsMono,
            _ => Self::CascadiaCode,
        }
    }

    pub(crate) fn family(self) -> &'static str {
        match self {
            Self::CascadiaCode => "Cascadia Code",
            Self::Consolas => "Consolas",
            Self::JetBrainsMono => "JetBrains Mono",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RowSpacing {
    Comfortable,
    Compact,
}

impl RowSpacing {
    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "Compact" => Self::Compact,
            _ => Self::Comfortable,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct SizeTokens {
    pub(crate) body_size: f32,
    pub(crate) item_size: f32,
    pub(crate) label_size: f32,
    pub(crate) meta_size: f32,
    pub(crate) session_header_size: f32,
    pub(crate) session_count_size: f32,
    pub(crate) session_row_height: f32,
    pub(crate) code_size: f32,
    pub(crate) cell_code_size: f32,
    pub(crate) cell_output_size: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct SpacingTokens {
    pub(crate) row_pad_y: f32,
    pub(crate) cell_pad_y: f32,
    pub(crate) code_pad_y: f32,
    pub(crate) cell_gap: f32,
    pub(crate) expanded_pad_x: f32,
    pub(crate) expanded_pad_y: f32,
    pub(crate) expanded_gap: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct MotionTokens {
    pub(crate) hover_ms: i64,
    pub(crate) menu_ms: i64,
    pub(crate) fold_ms: i64,
    pub(crate) fold_caret_ms: i64,
    pub(crate) band_ms: i64,
    pub(crate) smooth_scroll: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct AppearanceTokens {
    pub(crate) size: SizeTokens,
    pub(crate) code_font: &'static str,
    pub(crate) spacing: SpacingTokens,
    pub(crate) show_timestamps: bool,
    pub(crate) motion: MotionTokens,
}

pub(crate) fn size_tokens(size: InterfaceSize) -> SizeTokens {
    match size {
        InterfaceSize::Compact => SizeTokens {
            body_size: 13.5,
            item_size: 12.0,
            label_size: 10.0,
            meta_size: 9.0,
            session_header_size: 11.5,
            session_count_size: 10.5,
            session_row_height: 28.0,
            code_size: 10.5,
            cell_code_size: 11.0,
            cell_output_size: 10.0,
        },
        InterfaceSize::Default => SizeTokens {
            body_size: 15.0,
            item_size: 13.0,
            label_size: 11.0,
            meta_size: 10.0,
            session_header_size: 12.5,
            session_count_size: 11.5,
            session_row_height: 32.0,
            code_size: 11.5,
            cell_code_size: 12.0,
            cell_output_size: 11.0,
        },
        InterfaceSize::Large => SizeTokens {
            body_size: 17.0,
            item_size: 15.0,
            label_size: 12.5,
            meta_size: 11.0,
            session_header_size: 14.0,
            session_count_size: 13.0,
            session_row_height: 36.0,
            code_size: 13.0,
            cell_code_size: 13.5,
            cell_output_size: 12.5,
        },
    }
}

pub(crate) fn spacing_tokens(spacing: RowSpacing) -> SpacingTokens {
    match spacing {
        RowSpacing::Comfortable => SpacingTokens {
            row_pad_y: 6.0,
            cell_pad_y: 4.0,
            code_pad_y: 5.0,
            cell_gap: 6.0,
            expanded_pad_x: 12.0,
            expanded_pad_y: 10.0,
            expanded_gap: 8.0,
        },
        RowSpacing::Compact => SpacingTokens {
            row_pad_y: 3.0,
            cell_pad_y: 2.0,
            code_pad_y: 2.0,
            cell_gap: 4.0,
            expanded_pad_x: 9.0,
            expanded_pad_y: 6.0,
            expanded_gap: 5.0,
        },
    }
}

pub(crate) fn motion_tokens(reduce_motion: bool) -> MotionTokens {
    if reduce_motion {
        MotionTokens {
            hover_ms: 0,
            menu_ms: 0,
            fold_ms: 0,
            fold_caret_ms: 0,
            band_ms: 0,
            smooth_scroll: false,
        }
    } else {
        MotionTokens {
            hover_ms: 120,
            menu_ms: 90,
            fold_ms: 220,
            fold_caret_ms: 160,
            band_ms: 170,
            smooth_scroll: true,
        }
    }
}

pub(crate) fn tokens(settings: &Settings) -> AppearanceTokens {
    AppearanceTokens {
        size: size_tokens(InterfaceSize::parse(&settings.interface_size)),
        code_font: CodeFont::parse(&settings.code_font).family(),
        spacing: spacing_tokens(RowSpacing::parse(&settings.row_spacing)),
        show_timestamps: settings.show_timestamps,
        motion: motion_tokens(settings.reduce_motion),
    }
}

pub(crate) fn parse_timestamp_utc(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(raw) = value.as_i64() {
        if raw <= 0 {
            return None;
        }
        return if raw >= 1_000_000_000_000 {
            DateTime::from_timestamp_millis(raw)
        } else {
            DateTime::from_timestamp(raw, 0)
        };
    }
    if let Some(raw) = value.as_str() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
            return Some(parsed.with_timezone(&Utc));
        }
    }
    None
}

pub(crate) fn format_timestamp_with<Tz: TimeZone>(
    value: Option<&Value>,
    today: NaiveDate,
    tz: &Tz,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let Some(utc) = value.and_then(parse_timestamp_utc) else {
        return String::new();
    };
    let local = utc.with_timezone(tz);
    if local.date_naive() == today {
        local.format("%H:%M").to_string()
    } else {
        local.format("%Y-%m-%d %H:%M").to_string()
    }
}

pub(crate) fn format_timestamp(value: Option<&Value>) -> String {
    let tz = Local;
    let today = tz.from_utc_datetime(&Utc::now().naive_utc()).date_naive();
    format_timestamp_with(value, today, &tz)
}

pub(crate) fn apply(ui: &AppWindow, settings: &Settings) {
    let t = tokens(settings);
    let typ = ui.global::<Type>();
    typ.set_body(t.size.body_size);
    typ.set_item(t.size.item_size);

    let g = ui.global::<Appearance>();
    g.set_body_size(t.size.body_size);
    g.set_item_size(t.size.item_size);
    g.set_label_size(t.size.label_size);
    g.set_meta_size(t.size.meta_size);
    g.set_session_header_size(t.size.session_header_size);
    g.set_session_count_size(t.size.session_count_size);
    g.set_session_row_height(t.size.session_row_height);
    g.set_code_size(t.size.code_size);
    g.set_cell_code_size(t.size.cell_code_size);
    g.set_cell_output_size(t.size.cell_output_size);
    g.set_code_font(t.code_font.into());
    g.set_row_pad_y(t.spacing.row_pad_y);
    g.set_cell_pad_y(t.spacing.cell_pad_y);
    g.set_code_pad_y(t.spacing.code_pad_y);
    g.set_cell_gap(t.spacing.cell_gap);
    g.set_expanded_pad_x(t.spacing.expanded_pad_x);
    g.set_expanded_pad_y(t.spacing.expanded_pad_y);
    g.set_expanded_gap(t.spacing.expanded_gap);
    g.set_show_timestamps(t.show_timestamps);
    g.set_hover_duration(t.motion.hover_ms);
    g.set_menu_duration(t.motion.menu_ms);
    g.set_fold_duration(t.motion.fold_ms);
    g.set_fold_caret_duration(t.motion.fold_caret_ms);
    g.set_band_duration(t.motion.band_ms);
    g.set_smooth_scroll(t.motion.smooth_scroll);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_size_parses_options_and_falls_back_to_default() {
        assert_eq!(InterfaceSize::parse("Compact"), InterfaceSize::Compact);
        assert_eq!(InterfaceSize::parse("Default"), InterfaceSize::Default);
        assert_eq!(InterfaceSize::parse("Large"), InterfaceSize::Large);
        assert_eq!(InterfaceSize::parse(""), InterfaceSize::Default);
        assert_eq!(InterfaceSize::parse("Huge"), InterfaceSize::Default);

        let compact = size_tokens(InterfaceSize::Compact);
        let default = size_tokens(InterfaceSize::Default);
        let large = size_tokens(InterfaceSize::Large);
        assert!(compact.body_size < default.body_size && default.body_size < large.body_size);
        assert!(compact.item_size < default.item_size && default.item_size < large.item_size);
        assert!(compact.label_size < default.label_size && default.label_size < large.label_size);
        assert!(
            compact.session_row_height < default.session_row_height
                && default.session_row_height < large.session_row_height
        );
        assert_eq!(default.body_size, 15.0);
        assert_eq!(default.item_size, 13.0);
        assert_eq!(default.session_row_height, 32.0);
    }

    #[test]
    fn code_font_maps_all_options_and_falls_back_to_cascadia_code() {
        assert_eq!(CodeFont::parse("Cascadia Code").family(), "Cascadia Code");
        assert_eq!(CodeFont::parse("Consolas").family(), "Consolas");
        assert_eq!(CodeFont::parse("JetBrains Mono").family(), "JetBrains Mono");
        assert_eq!(CodeFont::parse("Unknown").family(), "Cascadia Code");
    }

    #[test]
    fn row_spacing_maps_comfortable_and_compact() {
        let comfortable = spacing_tokens(RowSpacing::parse("Comfortable"));
        let compact = spacing_tokens(RowSpacing::parse("Compact"));
        let fallback = spacing_tokens(RowSpacing::parse("Weird"));
        assert_eq!(fallback, comfortable);
        assert!(compact.row_pad_y < comfortable.row_pad_y);
        assert!(compact.cell_pad_y < comfortable.cell_pad_y);
        assert!(compact.code_pad_y < comfortable.code_pad_y);
        assert!(compact.expanded_pad_y < comfortable.expanded_pad_y);
    }

    #[test]
    fn reduce_motion_zeroes_nonessential_durations_and_disables_glide() {
        let normal = motion_tokens(false);
        let reduced = motion_tokens(true);
        assert!(normal.hover_ms > 0 && normal.fold_ms > 0 && normal.smooth_scroll);
        assert_eq!(reduced.hover_ms, 0);
        assert_eq!(reduced.menu_ms, 0);
        assert_eq!(reduced.fold_ms, 0);
        assert_eq!(reduced.fold_caret_ms, 0);
        assert_eq!(reduced.band_ms, 0);
        assert!(!reduced.smooth_scroll);
    }

    #[test]
    fn tokens_combine_all_five_settings() {
        let settings = Settings {
            interface_size: "Large".to_string(),
            code_font: "JetBrains Mono".to_string(),
            row_spacing: "Compact".to_string(),
            show_timestamps: true,
            reduce_motion: true,
            ..Settings::default()
        };
        let t = tokens(&settings);
        assert_eq!(t.size, size_tokens(InterfaceSize::Large));
        assert_eq!(t.code_font, "JetBrains Mono");
        assert_eq!(t.spacing, spacing_tokens(RowSpacing::Compact));
        assert!(t.show_timestamps);
        assert_eq!(t.motion, motion_tokens(true));
    }

    #[test]
    fn timestamps_format_in_local_time_and_omit_missing_values() {
        use chrono::FixedOffset;
        use serde_json::json;

        let offset = FixedOffset::east_opt(2 * 3600).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();

        let same_day_ms = json!(1_794_500_700_000_i64); // 2026-11-12T16:25:00Z -> 18:25 at +02:00
        assert_eq!(
            format_timestamp_with(Some(&same_day_ms), today, &offset),
            "18:25"
        );

        let same_day_s = json!(1_794_500_700_i64);
        assert_eq!(
            format_timestamp_with(Some(&same_day_s), today, &offset),
            "18:25"
        );

        let older_iso = json!("2026-11-10T23:30:00Z"); // 2026-11-11 01:30 at +02:00
        assert_eq!(
            format_timestamp_with(Some(&older_iso), today, &offset),
            "2026-11-11 01:30"
        );

        assert_eq!(format_timestamp_with(None, today, &offset), "");
        assert_eq!(format_timestamp_with(Some(&json!(null)), today, &offset), "");
        assert_eq!(format_timestamp_with(Some(&json!(0)), today, &offset), "");
        assert_eq!(format_timestamp_with(Some(&json!(-5)), today, &offset), "");
        assert_eq!(format_timestamp_with(Some(&json!("not-a-date")), today, &offset), "");
    }
}
