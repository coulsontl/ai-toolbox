//! Tray "current usage" block.
//!
//! Shows the gateway statistics page's selected time range — all CLIs combined,
//! same token/cost semantics as `usage_stats::usage_summary` — as a single
//! menu line: `{title}  {tokens} ({cost})`.
//!
//! Formatting lives here as pure functions so the menu text can be unit-tested
//! without a running tray or database.

use std::str::FromStr;

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::settings::GatewayUsageRangeSelection;

/// Menu item id; `on_menu_event` dispatches on it.
pub const TRAY_GATEWAY_USAGE_MENU_ID: &str = "gateway_usage_open";

/// Bilingual labels for the block. Kept next to the formatter rather than in
/// `TrayTexts` because the title is composed from the current range preset,
/// which `TrayTexts` (a static struct) cannot express.
#[derive(Clone, Copy)]
pub struct UsageTexts {
    pub today: &'static str,
    pub yesterday: &'static str,
    pub this_week: &'static str,
    pub last_week: &'static str,
    pub this_month: &'static str,
    pub last_month: &'static str,
    pub this_year: &'static str,
    pub custom: &'static str,
    /// Separator between the title and the numbers: the full-width colon for
    /// Chinese, `": "` for English.
    pub separator: &'static str,
}

pub fn usage_texts(language: &str) -> UsageTexts {
    if language != "zh-CN" {
        UsageTexts {
            today: "Today's usage",
            yesterday: "Yesterday's usage",
            this_week: "This week's usage",
            last_week: "Last week's usage",
            this_month: "This month's usage",
            last_month: "Last month's usage",
            this_year: "This year's usage",
            custom: "Custom usage",
            separator: ": ",
        }
    } else {
        UsageTexts {
            today: "今日用量",
            yesterday: "昨日用量",
            this_week: "本周用量",
            last_week: "上周用量",
            this_month: "本月用量",
            last_month: "上月用量",
            this_year: "本年用量",
            custom: "自定义用量",
            separator: "：",
        }
    }
}

/// Format a token count in millions (`>= 1B` switches to billions).
///
/// At least one unit shows up to two decimals, trailing zeros trimmed:
/// `1M`, `12.34M`, `1.23B`. Values below one million keep up to three decimals,
/// so a tiny day reads `0.001M` rather than rounding to a misleading `0M`.
pub fn format_token_amount(tokens: u64) -> String {
    const MILLION: f64 = 1_000_000.0;
    const BILLION: f64 = 1_000_000_000.0;

    if tokens as f64 >= BILLION {
        return format!("{}B", trim_decimal(tokens as f64 / BILLION, 2));
    }
    let millions = tokens as f64 / MILLION;
    if millions >= 1.0 {
        format!("{}M", trim_decimal(millions, 2))
    } else {
        format!("{}M", trim_decimal(millions, 3))
    }
}

/// Format the cost for the parenthesised suffix. `None` means "omit the
/// parentheses" — the caller decides that from the rounded value, so a
/// sub-cent day never renders as `($0.00)`.
pub fn format_cost_suffix(cost_usd: &str) -> Option<String> {
    let cost = Decimal::from_str(cost_usd.trim()).ok()?;
    // Currency rounds half away from zero; Decimal's default is banker's
    // rounding, which would turn 12.345 into 12.34. `f64` is fine past two
    // decimals — `format!` re-adds the trailing zeros the menu wants ($1.00).
    let rounded = cost
        .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
        .to_f64()
        .unwrap_or_default();
    if rounded == 0.0 {
        return None;
    }
    Some(format!("(${rounded:.2})"))
}

/// Compose the whole menu line: `{title}  {tokens} ({cost})`.
///
/// `start_seconds` / `end_seconds` are only used for the `custom` preset's
/// date suffix, so the label always says which window the number covers.
pub fn format_usage_menu_text(
    texts: UsageTexts,
    selection: &GatewayUsageRangeSelection,
    tokens: u64,
    cost_usd: &str,
    start_seconds: i64,
    end_seconds: i64,
) -> String {
    let title = match selection.preset.as_str() {
        "yesterday" => texts.yesterday,
        "thisWeek" => texts.this_week,
        "lastWeek" => texts.last_week,
        "thisMonth" => texts.this_month,
        "lastMonth" => texts.last_month,
        "thisYear" => texts.this_year,
        "custom" => texts.custom,
        _ => texts.today,
    };

    let mut label = title.to_string();
    if selection.preset == "custom" {
        let start = format_local_date(start_seconds);
        let end = format_local_date(end_seconds);
        if !start.is_empty() && !end.is_empty() {
            label = format!("{title} {start}–{end}");
        }
    }

    let tokens = format_token_amount(tokens);
    let amount = match format_cost_suffix(cost_usd) {
        Some(cost) => format!("{tokens} {cost}"),
        None => tokens,
    };
    format!("{label}{}{amount}", texts.separator)
}

fn trim_decimal(value: f64, decimals: usize) -> String {
    let formatted = format!("{value:.decimals$}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// `YYYY-MM-DD` in local time; empty when the timestamp is out of range.
fn format_local_date(timestamp_seconds: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(timestamp_seconds, 0).single() {
        Some(date) => date.format("%Y-%m-%d").to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(preset: &str) -> GatewayUsageRangeSelection {
        GatewayUsageRangeSelection {
            preset: preset.to_string(),
            start_date: None,
            end_date: None,
        }
    }

    #[test]
    fn token_amounts_switch_to_billions_and_trim_trailing_zeros() {
        assert_eq!(format_token_amount(0), "0M");
        assert_eq!(format_token_amount(1_000), "0.001M");
        assert_eq!(format_token_amount(350_000), "0.35M");
        assert_eq!(format_token_amount(12_300_000), "12.3M");
        assert_eq!(format_token_amount(57_260_000), "57.26M");
        assert_eq!(format_token_amount(57_263_000), "57.26M");
        assert_eq!(format_token_amount(999_999_999), "1000M");
        assert_eq!(format_token_amount(1_000_000_000), "1B");
        assert_eq!(format_token_amount(1_234_000_000), "1.23B");
        assert_eq!(format_token_amount(12_300_000_000), "12.3B");
    }

    #[test]
    fn cost_suffix_is_omitted_below_one_cent() {
        assert_eq!(format_cost_suffix("0"), None);
        assert_eq!(format_cost_suffix("0.001"), None);
        assert_eq!(format_cost_suffix("0.004"), None);
        assert_eq!(format_cost_suffix("0.42"), Some("($0.42)".to_string()));
        assert_eq!(format_cost_suffix("12.345"), Some("($12.35)".to_string()));
        assert_eq!(format_cost_suffix("not-a-number"), None);
    }

    #[test]
    fn menu_text_names_the_selected_preset() {
        let texts = usage_texts("en-US");
        assert!(format_usage_menu_text(texts, &selection("today"), 350_000, "0.42", 0, 0)
            .starts_with("Today's usage: 0.35M ($0.42)"));
        assert!(format_usage_menu_text(texts, &selection("yesterday"), 0, "0", 0, 0)
            .starts_with("Yesterday's usage: 0M"));
        assert!(format_usage_menu_text(texts, &selection("lastMonth"), 2_000_000, "1", 0, 0)
            .starts_with("Last month's usage: 2M ($1.00)"));
    }

    #[test]
    fn chinese_labels_are_used_for_chinese() {
        let texts = usage_texts("zh-CN");
        assert!(format_usage_menu_text(texts, &selection("today"), 1_000_000, "0", 0, 0)
            .starts_with("今日用量：1M"));
    }

    #[test]
    fn custom_preset_appends_the_date_window() {
        let texts = usage_texts("en-US");
        let start = 1_700_000_000;
        let end = start + 86_400;
        let text = format_usage_menu_text(texts, &selection("custom"), 1_000_000, "0", start, end);
        assert!(text.starts_with("Custom usage "));
        assert!(text.contains('–'));
        assert!(text.ends_with(": 1M"));
    }
}
