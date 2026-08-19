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
