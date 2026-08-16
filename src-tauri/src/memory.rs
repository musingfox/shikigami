use serde::Serialize;
pub const HISTORY_DEPTH: usize = 6;


#[derive(Serialize)]
struct ActionRow<'a> {
    ts: String,
    verb: &'a str,
    pane: &'a str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<&'a str>,
}

fn action_row(
    verb: &str,
    pane: &str,
    text: &str,
    project: Option<&str>,
    agent: Option<&str>,
    unix_secs: i64,
) -> String {
    serde_json::to_string(&ActionRow {
        ts: crate::sumvox::rfc3339_utc(unix_secs),
        verb,
        pane,
        text,
        project,
        agent,
    })
    .expect("serializing a memory row cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_row_encodes_known_fields_and_omits_unknowns() {
        assert_eq!(
            action_row("inject", "%1", "跑測試", None, Some("cyris"), 0),
            r#"{"ts":"1970-01-01T00:00:00Z","verb":"inject","pane":"%1","text":"跑測試","agent":"cyris"}"#
        );
        assert_eq!(
            action_row("summon", "%9", "跑測試", Some("cyris"), None, 0),
            r#"{"ts":"1970-01-01T00:00:00Z","verb":"summon","pane":"%9","text":"跑測試","project":"cyris"}"#
        );
        let row = action_row("inject", "%1", "hi", None, None, 0);
        assert!(!row.contains("agent"));
        assert!(!row.contains("project"));
    }
}
