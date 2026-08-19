//! Model for a single line of a Claude Code transcript (`~/.claude/projects/**/*.jsonl`).
//!
//! The on-disk format is an internal detail of Claude Code and drifts between
//! versions, so everything here reads through `serde_json::Value` with defaults
//! rather than a rigid struct: an unknown block type is skipped, a missing field
//! becomes an empty string. That mirrors how the original jq program used `//`.

use serde_json::Value;

/// Which block types to flatten out of `message.content`.
#[derive(Clone, Copy)]
pub struct BlockOpts {
    pub thinking: bool,
    pub tools: bool,
}

pub struct Record<'a> {
    v: &'a Value,
}

impl<'a> Record<'a> {
    pub fn new(v: &'a Value) -> Self {
        Record { v }
    }

    pub fn str_field(&self, k: &str) -> &'a str {
        self.v.get(k).and_then(Value::as_str).unwrap_or("")
    }

    fn bool_field(&self, k: &str) -> bool {
        self.v.get(k).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn kind(&self) -> &'a str {
        self.str_field("type")
    }
    pub fn cwd(&self) -> &'a str {
        self.str_field("cwd")
    }
    pub fn session_id(&self) -> &'a str {
        self.str_field("sessionId")
    }
    pub fn timestamp(&self) -> &'a str {
        self.str_field("timestamp")
    }
    pub fn is_meta(&self) -> bool {
        self.bool_field("isMeta")
    }
    pub fn is_sidechain(&self) -> bool {
        self.bool_field("isSidechain")
    }

    pub fn is_conversation(&self) -> bool {
        matches!(self.kind(), "user" | "assistant")
    }

    pub fn content(&self) -> Option<&'a Value> {
        self.v.pointer("/message/content")
    }

    /// Flatten `message.content` into text segments.
    ///
    /// `content` is either a bare string or an array of typed blocks, so both
    /// shapes are handled. Returns owned strings because `tool_use` / `tool_result`
    /// have to re-serialize their payloads.
    pub fn blocks(&self, opts: BlockOpts) -> Vec<String> {
        let Some(content) = self.content() else {
            return Vec::new();
        };
        match content {
            Value::String(s) => vec![s.clone()],
            Value::Array(items) => items
                .iter()
                .filter_map(|b| block_text(b, opts))
                .collect(),
            _ => Vec::new(),
        }
    }
}

fn block_text(b: &Value, opts: BlockOpts) -> Option<String> {
    let ty = b.get("type").and_then(Value::as_str).unwrap_or("");
    match ty {
        "text" => b.get("text").and_then(Value::as_str).map(str::to_owned),
        "thinking" if opts.thinking => {
            b.get("thinking").and_then(Value::as_str).map(str::to_owned)
        }
        "tool_use" if opts.tools => {
            let name = b.get("name").and_then(Value::as_str).unwrap_or("tool");
            let input = b.get("input").map(stringify).unwrap_or_default();
            Some(format!("{name} {input}"))
        }
        "tool_result" if opts.tools => b.get("content").map(stringify),
        _ => None,
    }
}

/// Equivalent of jq's `tostring`: strings pass through unquoted, everything
/// else is rendered as compact JSON.
pub fn stringify(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Truncate to at most `n` characters (not bytes), matching jq's `.[0:n]`.
pub fn take_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ALL: BlockOpts = BlockOpts { thinking: true, tools: true };
    const TEXT_ONLY: BlockOpts = BlockOpts { thinking: false, tools: false };

    #[test]
    fn take_chars_counts_characters_not_bytes() {
        assert_eq!(take_chars("hello", 3), "hel");
        assert_eq!(take_chars("hello", 99), "hello");
        assert_eq!(take_chars("", 5), "");
        // Would panic on a byte slice: each of these is multi-byte.
        assert_eq!(take_chars("héllo", 2), "hé");
        assert_eq!(take_chars("日本語テスト", 3), "日本語");
        assert_eq!(take_chars("🙂🙃🙂", 2), "🙂🙃");
    }

    #[test]
    fn stringify_matches_jq_tostring() {
        // jq's tostring leaves strings unquoted but renders everything else as JSON.
        assert_eq!(stringify(&json!("plain")), "plain");
        assert_eq!(stringify(&json!("has \"quotes\"")), "has \"quotes\"");
        assert_eq!(stringify(&json!(42)), "42");
        assert_eq!(stringify(&json!(null)), "null");
        assert_eq!(stringify(&json!({"a": 1})), r#"{"a":1}"#);
        assert_eq!(stringify(&json!([1, 2])), "[1,2]");
    }

    #[test]
    fn string_content_is_a_single_block() {
        let v = json!({"message": {"content": "bare string"}});
        assert_eq!(Record::new(&v).blocks(ALL), vec!["bare string"]);
    }

    #[test]
    fn missing_or_odd_content_yields_nothing() {
        for v in [
            json!({}),
            json!({"message": {}}),
            json!({"message": {"content": null}}),
            json!({"message": {"content": 7}}),
        ] {
            assert!(Record::new(&v).blocks(ALL).is_empty(), "{v}");
        }
    }

    #[test]
    fn thinking_and_tool_blocks_are_gated_by_opts() {
        let v = json!({"message": {"content": [
            {"type": "text", "text": "visible"},
            {"type": "thinking", "thinking": "pondering"},
            {"type": "tool_use", "name": "Bash", "input": {"cmd": "ls"}},
            {"type": "tool_result", "content": "output here"},
        ]}});
        let r = Record::new(&v);

        assert_eq!(r.blocks(TEXT_ONLY), vec!["visible"]);
        assert_eq!(
            r.blocks(ALL),
            vec![
                "visible",
                "pondering",
                r#"Bash {"cmd":"ls"}"#,
                "output here",
            ]
        );
        assert_eq!(
            r.blocks(BlockOpts { thinking: true, tools: false }),
            vec!["visible", "pondering"]
        );
    }

    #[test]
    fn unknown_and_malformed_blocks_are_skipped_not_fatal() {
        // The transcript format drifts; an unrecognised block must not kill the line.
        let v = json!({"message": {"content": [
            {"type": "some_future_block", "payload": "ignored"},
            {"no_type_field": true},
            {"type": "text", "text": "still here"},
        ]}});
        assert_eq!(Record::new(&v).blocks(ALL), vec!["still here"]);
    }

    #[test]
    fn tool_use_without_a_name_falls_back() {
        let v = json!({"message": {"content": [
            {"type": "tool_use", "input": {}},
        ]}});
        assert_eq!(Record::new(&v).blocks(ALL), vec!["tool {}"]);
    }

    #[test]
    fn field_accessors_default_rather_than_panic() {
        let empty = json!({});
        let r = Record::new(&empty);
        assert_eq!(r.kind(), "");
        assert_eq!(r.cwd(), "");
        assert_eq!(r.session_id(), "");
        assert_eq!(r.timestamp(), "");
        assert!(!r.is_meta());
        assert!(!r.is_sidechain());
        assert!(!r.is_conversation());
    }

    #[test]
    fn only_user_and_assistant_count_as_conversation() {
        for (ty, want) in [
            ("user", true),
            ("assistant", true),
            ("queue-operation", false),
            ("summary", false),
        ] {
            let v = json!({"type": ty});
            assert_eq!(Record::new(&v).is_conversation(), want, "type={ty}");
        }
    }
}
