use colored::*;
use std::sync::OnceLock;

const BRAND: &str = "NeoDEM";

pub fn banner() {
    println!();
    println!(
        "  {} {}",
        "//".bright_black(),
        BRAND.truecolor(255, 103, 0).bold()
    );
    println!("  {}", "━".repeat(50).bright_black());
}

pub fn section(icon: &str, title: &str) {
    println!();
    println!("  {} {}", icon.truecolor(255, 103, 0), title.bold());
    println!("  {}", "┄".repeat(50).bright_black());
}

pub fn field(label: &str, value: &str) {
    println!(
        "    {} {} {}",
        "│".bright_black(),
        format!("{:<14}", label).bright_black(),
        value
    );
}

pub fn field_colored(label: &str, value: ColoredString) {
    println!(
        "    {} {} {}",
        "│".bright_black(),
        format!("{:<14}", label).bright_black(),
        value
    );
}

pub fn table_header(columns: &[(&str, usize)]) {
    let header: String = columns
        .iter()
        .map(|(name, width)| format!("{:<width$}", name, width = width))
        .collect::<Vec<_>>()
        .join(" ");
    println!("    {}", header.bright_black());
    let rule: String = columns
        .iter()
        .map(|(_, width)| "─".repeat(*width))
        .collect::<Vec<_>>()
        .join("─");
    println!("    {}", rule.bright_black());
}

pub fn success(msg: &str) {
    println!("  {} {}", "[OK]".truecolor(255, 103, 0).bold(), msg);
}

pub fn info(msg: &str) {
    println!("  {} {}", "[--]".bright_black(), msg.bright_black());
}

pub fn status_indicator(status: &str) -> String {
    let (icon, colored) = match status.to_lowercase().as_str() {
        // A latched stop is the one state that must never read as normal.
        "emergency_stop" | "estop" | "e_stop" | "protective_stop" | "safety_stop" => {
            ("■", status.to_uppercase().red().bold())
        }
        "online" | "active" | "connected" | "completed" | "done" | "succeeded" | "success"
        | "healthy" | "published" | "compliant" | "resolved" | "closed" | "approved" => {
            ("●", status.green().bold())
        }
        "offline" | "disconnected" | "failed" | "error" | "rejected" | "aborted" | "expired"
        | "overdue" | "non_compliant" => ("●", status.red().bold()),
        "degraded" | "warning" | "pending" | "queued" | "paused" | "stale" | "at_risk"
        | "partial" | "awaiting_approval" | "in_progress" | "open" => ("●", status.yellow().bold()),
        // Doing something on purpose: not a fault, not idle.
        "busy" | "running" | "charging" | "moving" | "executing" | "training" | "canary"
        | "staging" | "manual" | "teleop" => ("◐", status.truecolor(255, 103, 0).bold()),
        "idle" | "unknown" | "ready" | "draft" | "created" | "archived" | "cancelled"
        | "canceled" | "deprecated" | "skipped" => ("○", status.bright_black()),
        _ => ("○", status.normal()),
    };
    format!("{} {}", icon.bright_black(), colored)
}

/// True/false as a glyph, for columns like ACK or ENABLED.
pub fn bool_indicator(v: Option<bool>) -> String {
    match v {
        Some(true) => "✓".green().to_string(),
        Some(false) => "·".bright_black().to_string(),
        None => "-".bright_black().to_string(),
    }
}

pub fn severity_indicator(severity: &str) -> String {
    let (icon, colored) = match severity.to_lowercase().as_str() {
        "critical" => ("▲", severity.to_uppercase().red().bold()),
        "error" => ("▲", severity.to_uppercase().red()),
        "warning" => ("◆", severity.to_uppercase().yellow()),
        "info" => ("●", severity.to_uppercase().blue()),
        _ => ("○", severity.to_uppercase().normal()),
    };
    format!("{} {}", icon, colored)
}

/// Shorten to `max` *characters*, never bytes.
///
/// Slicing by byte index panics the moment a multi-byte character straddles the
/// cut, which server-supplied names (robot names, dataset titles) reach easily.
pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let kept: String = s.chars().take(max - 1).collect();
        format!("{kept}…")
    }
}

/// Truncate for diagnostics, where a hard character budget matters more than
/// looking tidy.
pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let kept: String = s.chars().take(max).collect();
        format!("{kept}… [{} chars total]", s.chars().count())
    }
}

/// Visible length of a string, ignoring ANSI escape codes.
fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else if c == '\x1b' {
            in_escape = true;
        } else {
            len += 1;
        }
    }
    len
}

/// Pad a string (possibly containing ANSI codes) to a visible width.
pub fn pad(s: &str, width: usize) -> String {
    let vis = visible_len(s);
    if vis >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - vis))
    }
}

pub fn meter(label: &str, value: f64, max: f64, color: &str) {
    let pct = if max > 0.0 { value / max } else { 0.0 };
    let filled = (pct * 20.0) as usize;
    let empty = 20 - filled.min(20);
    let bar_filled = "█".repeat(filled.min(20));
    let bar_empty = "░".repeat(empty);

    let colored_bar = match color {
        "green" => bar_filled.green(),
        "red" => bar_filled.red(),
        "yellow" => bar_filled.yellow(),
        "orange" => bar_filled.truecolor(255, 103, 0),
        _ => bar_filled.normal(),
    };

    println!(
        "    {} {} {}{} {:>5.1}%",
        "│".bright_black(),
        format!("{:<14}", label).bright_black(),
        colored_bar,
        bar_empty.bright_black(),
        pct * 100.0
    );
}

pub fn stat_line(icon: &str, label: &str, value: &str, color: &str) {
    let colored_value = match color {
        "green" => value.green().bold(),
        "red" => value.red().bold(),
        "yellow" => value.yellow().bold(),
        "orange" => value.truecolor(255, 103, 0).bold(),
        "dim" => value.bright_black(),
        _ => value.normal(),
    };
    println!(
        "    {} {:<18} {}",
        icon.bright_black(),
        label.bright_black(),
        colored_value
    );
}

pub fn warn(msg: &str) {
    println!("  {} {}", "[!]".yellow().bold(), msg);
}

/// Header for a block inside a detail view.
pub fn subsection(label: &str) {
    println!();
    println!("    {} {}", "│".bright_black(), label.bold().bright_black());
}

/// Print a server payload verbatim for `--json`.
pub fn json_out(value: &serde_json::Value) -> crate::error::RmsResult<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// The `--page` the user asked for, so list footers can say where they are.
///
/// Set once from the global flags. A footer cannot take this as a parameter
/// without threading it through all sixty call sites, and it is genuinely
/// process-wide input rather than per-command state.
static REQUESTED_PAGE: OnceLock<u32> = OnceLock::new();

pub fn set_requested_page(page: Option<u32>) {
    if let Some(p) = page.filter(|p| *p > 0) {
        let _ = REQUESTED_PAGE.set(p);
    }
}

fn requested_page() -> u32 {
    REQUESTED_PAGE.get().copied().unwrap_or(1)
}

/// Footer for a list, honest about pagination.
///
/// The old `count_label` printed the envelope's `total` under a table holding
/// only one page, so `alerts list` claimed 75 rows while showing 50. When the
/// two disagree, say so and name the flag that gets the rest.
pub fn count_paged(label: &str, shown: usize, total: Option<u32>, total_pages: Option<u32>) {
    println!();
    let total = total.map(|t| t as usize);
    match total {
        Some(t) if t > shown => {
            println!(
                "  {} {}",
                format!("{shown} of {t}").truecolor(255, 103, 0).bold(),
                label.bright_black()
            );
            let page = requested_page();
            let more = match total_pages.filter(|p| *p > 1) {
                Some(p) => format!("page {page} of {p} — use --page N, or --limit {t} for all"),
                None => format!("use --limit {t} to fetch all"),
            };
            println!("  {} {}", "[--]".bright_black(), more.bright_black());
        }
        _ => {
            let n = total.unwrap_or(shown);
            println!(
                "  {} {}",
                format!("{n}").truecolor(255, 103, 0).bold(),
                label.bright_black()
            );
        }
    }
}

/// Gate for anything that changes fleet or deployment state.
///
/// Returns true when the caller may proceed. Without `--yes` it describes the
/// action and refuses, so a mistyped robot ID costs nothing.
pub fn confirm(action: &str, yes: bool) -> bool {
    if yes {
        return true;
    }
    println!();
    println!("  {} {}", "[!]".yellow().bold(), action.bold());
    println!(
        "  {} {}",
        "[-->]".truecolor(255, 103, 0),
        "Re-run with --yes to execute.".bright_black()
    );
    println!();
    false
}

/// Width that fits a full 36-character UUID plus padding.
pub const ID_W: usize = 38;

/// Columns of the terminal stdout is attached to; `None` when it is a pipe or
/// a file, where there is no width to fit.
pub fn term_width() -> Option<usize> {
    terminal_size::terminal_size().map(|(w, _)| usize::from(w.0))
}

/// Width for the one column of a list table that may grow — the name.
///
/// `min` is the width the table had when it was fixed, and what it keeps when
/// there is no terminal or the terminal is too narrow: a table never gets
/// narrower than before. `fixed` is what every other column takes together
/// (see [`fixed_width`]), `longest` the longest value in the column, so a wide
/// terminal does not stretch the table past what the names need.
pub fn flex_width(min: usize, fixed: usize, longest: usize, term: Option<usize>) -> usize {
    let Some(term) = term else {
        return min;
    };
    // One column spare: a row that ends exactly at the edge wraps on some terminals.
    let available = term.saturating_sub(fixed + 1);
    available.min(longest).max(min)
}

/// What the other columns of a table take: the 4-space indent, each width, and
/// one separating space per column (the flexible one's included).
pub fn fixed_width(others: &[usize]) -> usize {
    4 + others.iter().sum::<usize>() + others.len()
}

/// Longest value of a column, in characters.
pub fn longest<'a>(values: impl Iterator<Item = &'a str>) -> usize {
    values.map(|v| v.chars().count()).max().unwrap_or(0)
}

/// The status cell of a robot: `expected` is the wording for a robot that is
/// on the fleet but has never connected (`Robot::expected_label`), which must
/// not read as the red "offline" of a robot that dropped out.
pub fn robot_status(status: &str, expected: Option<&str>) -> String {
    match expected {
        Some(label) => format!("{} {}", "○".bright_black(), label.bright_black()),
        None => status_indicator(status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flex_width_keeps_the_old_width_without_a_terminal_or_on_a_narrow_one() {
        // Piped output: no width to fit, so the table is what it always was.
        assert_eq!(flex_width(26, 100, 44, None), 26);
        // An 80-column terminal the fixed columns already overflow.
        assert_eq!(flex_width(26, 100, 44, Some(80)), 26);
        assert_eq!(flex_width(26, 100, 44, Some(0)), 26);
    }

    #[test]
    fn flex_width_grows_to_the_terminal_but_not_past_the_longest_name() {
        // Room for 39 columns of name (one spare), names need 44: use the room.
        assert_eq!(flex_width(26, 100, 44, Some(140)), 39);
        // Room for more than the names need: stop at the longest name.
        assert_eq!(flex_width(26, 100, 44, Some(200)), 44);
        // Short names never shrink the column below its minimum.
        assert_eq!(flex_width(26, 100, 9, Some(200)), 26);
    }

    #[test]
    fn fixed_width_counts_indent_columns_and_separators() {
        assert_eq!(fixed_width(&[38, 14, 11, 9, 9, 14]), 4 + 95 + 6);
        assert_eq!(longest(["ab", "Apple — G1", ""].into_iter()), 10);
        assert_eq!(longest(std::iter::empty()), 0);
    }

    #[test]
    fn an_expected_robot_is_not_drawn_as_offline() {
        colored::control::set_override(false);
        assert_eq!(
            robot_status("offline", Some("not connected")),
            "○ not connected"
        );
        assert_eq!(robot_status("offline", None), "● offline");
        colored::control::unset_override();
    }

    /// The pre-audit `truncate` sliced by byte index, so any name longer than
    /// the limit whose cut landed inside a multi-byte character panicked with
    /// "byte index N is not a char boundary" and killed the whole command.
    #[test]
    fn truncate_does_not_split_multibyte_characters() {
        let cases = [
            "Roboter-Übergabestation Süd-Ost",
            "机器人装配单元一号线",
            "Poste d'assemblage — atelier nº 4",
            "🤖🤖🤖 pick-and-place cell 🤖🤖🤖",
            "Väderstation för lagerhallen i Göteborg",
        ];
        for s in cases {
            for max in 1..=40 {
                let out = truncate(s, max);
                assert!(
                    out.chars().count() <= max,
                    "{s:?} at max={max} produced {} chars",
                    out.chars().count()
                );
            }
        }
    }

    #[test]
    fn truncate_leaves_short_strings_alone() {
        assert_eq!(truncate("SimBot-01", 24), "SimBot-01");
        assert_eq!(truncate("Übergabe", 8), "Übergabe");
        assert_eq!(truncate("", 10), "");
        assert_eq!(truncate("anything", 0), "");
    }

    #[test]
    fn truncate_marks_what_it_cut() {
        let out = truncate("G1-EDU-Bot (Agent Mode)", 12);
        assert_eq!(out.chars().count(), 12);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn clip_is_char_safe_too() {
        for max in 1..=20 {
            let out = clip("Überwachung der Roboterflotte", max);
            assert!(
                out.starts_with(
                    &"Überwachung der Roboterflotte"
                        .chars()
                        .take(max)
                        .collect::<String>()
                )
            );
        }
    }

    /// `pad` measures visible width, so a coloured cell must still line up.
    #[test]
    fn pad_ignores_ansi_escapes() {
        let plain = pad("online", 12);
        let coloured = pad(&"online".green().to_string(), 12);
        assert_eq!(visible_len(&plain), 12);
        assert_eq!(visible_len(&coloured), 12);
    }

    #[test]
    fn pad_does_not_truncate_when_over_width() {
        assert_eq!(pad("sim-robot-g1-edu-b", 4), "sim-robot-g1-edu-b");
    }
}
