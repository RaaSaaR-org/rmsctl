use colored::*;

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
    println!(
        "  {} {}",
        icon.truecolor(255, 103, 0),
        title.bold()
    );
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
    println!(
        "  {} {}",
        "[OK]".truecolor(255, 103, 0).bold(),
        msg
    );
}

pub fn info(msg: &str) {
    println!(
        "  {} {}",
        "[--]".bright_black(),
        msg.bright_black()
    );
}

pub fn count_label(label: &str, count: u32) {
    println!();
    println!(
        "  {} {}",
        format!("{count}").truecolor(255, 103, 0).bold(),
        label.bright_black()
    );
}

pub fn status_indicator(status: &str) -> String {
    let (icon, colored) = match status.to_lowercase().as_str() {
        "online" | "active" | "connected" | "completed" => {
            ("●", status.green().bold())
        }
        "offline" | "disconnected" | "failed" => {
            ("●", status.red().bold())
        }
        "error" => {
            ("●", status.red().bold())
        }
        "degraded" | "warning" | "pending" => {
            ("●", status.yellow().bold())
        }
        "idle" | "unknown" => {
            ("○", status.bright_black())
        }
        _ => ("○", status.normal()),
    };
    format!("{} {}", icon.bright_black(), colored)
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

pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
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
