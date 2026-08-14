#[cfg(test)]
mod tests {
    use super::{collect_with, normalize, select_panes, AgentDepth, PreciseDepth};
    use crate::cchooks::HookDepth;
    use crate::events::AgentEntry;
    use std::cell::Cell;

    fn agent(pane: &str, status: &str) -> AgentEntry {
        AgentEntry {
            id: pane.into(), name: pane.into(), pane: pane.into(), status: status.into(),
            title: String::new(), cwd: String::new(),
        }
    }

    fn precise<'a>(panes: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |pane| panes.contains(&pane)
    }

    fn hook(pane: &str, detail: &str) -> HookDepth {
        HookDepth { pane: pane.into(), label: "stop".into(), detail: detail.into(), ts: "t".into() }
    }

    #[test]
    fn collects_screen_for_blocked_without_precise_depth() {
        let reads = Cell::new(0);
        let got = collect_with(&[agent("p1", "blocked")], |_| None, |_| {
            reads.set(reads.get() + 1);
            Ok("x\ny".to_string())
        });
        assert_eq!(reads.get(), 1);
        assert_eq!(got, vec![AgentDepth {
            pane: "p1".into(),
            precise: None,
            screen: Some("x\ny".into()),
        }]);
    }

    #[test]
    fn retains_precise_depth_when_screen_read_fails() {
        let reads = Cell::new(0);
        let got = collect_with(&[agent("p1", "blocked")], |_| Some(hook("p1", "卡在權限")), |_| {
            reads.set(reads.get() + 1);
            Err("herdr pane.read: unavailable".into())
        });
        assert_eq!(reads.get(), 1);
        assert_eq!(got[0].precise, Some(PreciseDepth {
            label: "stop".into(),
            detail: "卡在權限".into(),
        }));
        assert_eq!(got[0].screen, None);
    }

    #[test]
    fn idle_without_hook_is_empty_and_does_not_read() {
        let reads = Cell::new(0);
        let got = collect_with(&[agent("p1", "idle")], |_| None, |_| {
            reads.set(reads.get() + 1); Ok("unexpected".into())
        });
        assert!(got.is_empty());
        assert_eq!(reads.get(), 0);
    }

    #[test]
    fn precise_budget_keeps_first_ten_and_blocks_screens() {
        let roster: Vec<_> = (0..11).map(|i| agent(&format!("p{i}"), "working")).collect();
        let reads = Cell::new(0);
        let got = collect_with(&roster, |pane| Some(hook(pane, &"x".repeat(200))), |_| {
            reads.set(reads.get() + 1); Ok("screen".into())
        });
        assert_eq!(got.len(), 10);
        assert_eq!(got.iter().map(|a| a.pane.as_str()).collect::<Vec<_>>(),
            (0..10).map(|i| format!("p{i}")).collect::<Vec<_>>().iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(reads.get(), 0);
        assert!(got.iter().all(|depth| depth.screen.is_none()));
    }

    #[test]
    fn precise_detail_is_unicode_safe_and_capped() {
        let got = collect_with(&[agent("p1", "blocked")], |_| Some(hook("p1", &"界".repeat(10_000))), |_| {
            Ok("screen".into())
        });
        let detail = &got[0].precise.as_ref().unwrap().detail;
        assert_eq!(detail.chars().count(), 200);
        assert!(detail.starts_with('…'));
    }

    #[test]
    fn selects_blocked_without_precise_depth() {
        let roster = vec![agent("p1", "blocked")];
        assert_eq!(select_panes(&roster, precise(&[])), vec!["p1"]);
    }

    #[test]
    fn selects_working_without_precise_depth() {
        let roster = vec![agent("p1", "working")];
        assert_eq!(select_panes(&roster, precise(&[])), vec!["p1"]);
    }

    #[test]
    fn selects_idle_with_precise_depth() {
        let roster = vec![agent("p1", "idle")];
        assert_eq!(select_panes(&roster, precise(&["p1"])), vec!["p1"]);
    }

    #[test]
    fn excludes_idle_done_and_unknown_without_precise_depth() {
        let roster = vec![agent("i", "idle"), agent("d", "done"), agent("u", "unknown")];
        assert!(select_panes(&roster, precise(&[])).is_empty());
    }

    #[test]
    fn excludes_empty_pane() {
        assert!(select_panes(&[agent("", "blocked")], precise(&[])).is_empty());
    }

    #[test]
    fn preserves_order_and_caps_at_three() {
        let roster = vec![
            agent("p1", "blocked"),
            agent("p2", "blocked"),
            agent("p3", "working"),
            agent("p4", "blocked"),
        ];
        assert_eq!(select_panes(&roster, precise(&[])), vec!["p1", "p2", "p3"]);
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreciseDepth {
    pub label: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentDepth {
    pub pane: String,
    pub precise: Option<PreciseDepth>,
    pub screen: Option<String>,
}

pub(crate) fn collect_with<PF, SF>(
    roster: &[AgentEntry],
    mut precise_for: PF,
    mut screen_for: SF,
) -> Vec<AgentDepth>
where
    PF: FnMut(&str) -> Option<crate::cchooks::HookDepth>,
    SF: FnMut(&str) -> Result<String, String>,
{
    let mut depths = Vec::with_capacity(roster.len());
    let mut running = 0;
    for agent in roster {
        let precise = precise_for(&agent.pane).map(|depth| PreciseDepth {
            label: normalize_precise(&depth.label),
            detail: normalize_precise(&depth.detail),
        });
        if let Some(depth) = &precise {
            let chars = depth.detail.chars().count();
            if running + chars > 2000 {
                return depths;
            }
            running += chars;
        }
        depths.push(AgentDepth {
            pane: agent.pane.clone(),
            precise,
            screen: None,
        });
    }

    let selected = select_panes(roster, |pane| {
        depths.iter().any(|depth| depth.pane == pane && depth.precise.is_some())
    });
    for pane in selected {
        if let Ok(text) = screen_for(&pane) {
            let screen = normalize(&text, 20, 600);
            let chars = screen.chars().count();
            if running + chars > 2000 {
                break;
            }
            if !screen.is_empty() {
                running += chars;
                if let Some(depth) = depths.iter_mut().find(|depth| depth.pane == pane) {
                    depth.screen = Some(screen);
                }
            }
        }
    }
    depths.retain(|depth| depth.precise.is_some() || depth.screen.is_some());
    depths
}

pub(crate) fn collect(roster: &[AgentEntry]) -> Vec<AgentDepth> {
    collect_with(roster, crate::cchooks::depth_for, crate::herdr::pane_recent_text)
}

fn normalize_precise(input: &str) -> String {
    let normalized = normalize(input, usize::MAX, 200);
    if normalized.chars().count() > 200 {
        let tail: String = normalized.chars().skip(normalized.chars().count() - 199).collect();
        format!("…{tail}")
    } else {
        normalized
    }
}
