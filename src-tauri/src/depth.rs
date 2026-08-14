#[cfg(test)]
mod tests {
    use super::{normalize, select_panes};
    use crate::events::AgentEntry;

    fn agent(pane: &str, status: &str) -> AgentEntry {
        AgentEntry {
            id: pane.to_string(),
            name: pane.to_string(),
            pane: pane.to_string(),
            status: status.to_string(),
            title: String::new(),
            cwd: String::new(),
        }
    }

    fn precise<'a>(panes: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |pane| panes.contains(&pane)
    }

    #[test]
    fn selects_blocked_without_precise_depth() {
        let roster = [agent("%1", "blocked")];
        assert_eq!(select_panes(&roster, precise(&[])), vec!["%1"]);
    }

    #[test]
    fn selects_working_without_precise_depth() {
        let roster = [agent("%1", "working")];
        assert_eq!(select_panes(&roster, precise(&[])), vec!["%1"]);
    }

    #[test]
    fn selects_idle_with_precise_depth() {
        let roster = [agent("%1", "idle")];
        assert_eq!(select_panes(&roster, precise(&["%1"])), vec!["%1"]);
    }

    #[test]
    fn excludes_idle_done_and_unknown_without_precise_depth() {
        let roster = [
            agent("%1", "idle"),
            agent("%2", "done"),
            agent("%3", "unknown"),
        ];
        assert!(select_panes(&roster, precise(&[])).is_empty());
    }

    #[test]
    fn excludes_empty_pane() {
        let roster = [agent("", "blocked")];
        assert!(select_panes(&roster, precise(&[])).is_empty());
    }

    #[test]
    fn preserves_order_and_caps_at_three() {
        let roster = [
            agent("p1", "blocked"),
            agent("p2", "blocked"),
            agent("p3", "working"),
            agent("p4", "blocked"),
        ];
        assert_eq!(
            select_panes(&roster, precise(&[])),
            vec!["p1", "p2", "p3"]
        );
    }

    #[test]
    fn keeps_newest_lines() {
        assert_eq!(normalize("a\nb\nc", 2, 100), "…b\nc");
    }

    #[test]
    fn removes_ansi_and_normalizes_crlf() {
        assert_eq!(normalize("\x1b[0m299\r\n300\r\n", 5, 100), "299\n300");
    }

    #[test]
    fn limits_unicode_by_char_count() {
        assert_eq!(normalize("你好世界", 5, 2), "…世界");
    }

    #[test]
    fn whitespace_only_is_empty() {
        assert_eq!(normalize("   \n\n\t\n", 5, 100), "");
    }

    #[test]
    fn unchanged_input_has_no_ellipsis() {
        assert_eq!(normalize("a\nb", 5, 100), "a\nb");
    }
}

use crate::events::AgentEntry;

pub(crate) fn select_panes<F>(roster: &[AgentEntry], precise: F) -> Vec<String>
where
    F: Fn(&str) -> bool,
{
    roster
        .iter()
        .filter(|agent| {
            !agent.pane.is_empty()
                && (precise(&agent.pane) || matches!(agent.status.as_str(), "blocked" | "working"))
        })
        .take(3)
        .map(|agent| agent.pane.clone())
        .collect()
}

pub fn normalize(input: &str, max_lines: usize, max_chars: usize) -> String {
    let cleaned = strip_ansi_and_controls(input);
    let normalized = cleaned.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = normalized.split('\n').collect();

    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    if lines.is_empty() || max_lines == 0 || max_chars == 0 {
        return String::new();
    }

    let line_truncated = lines.len() > max_lines;
    if line_truncated {
        let start = lines.len() - max_lines;
        lines = lines.split_off(start);
    }

    let mut tail = lines.join("\n");
    let char_count = tail.chars().count();
    let char_truncated = char_count > max_chars;
    if char_truncated {
        tail = tail.chars().skip(char_count - max_chars).collect();
    }

    if line_truncated || char_truncated {
        format!("…{tail}")
    } else {
        tail
    }
}

fn strip_ansi_and_controls(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(next) = chars.next() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
        } else if ch == '\n' || ch == '\r' {
            output.push(ch);
        } else if !ch.is_control() {
            output.push(ch);
        }
    }
    output
}
