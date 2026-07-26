//! Convert incoming Matrix (agent) messages into vachat `ChatMessageContent`.
//!
//! Agents (QwenPaw, Hermes, cc-connect, ...) connect over the Matrix protocol.
//! Their messages arrive at the Matrix bridge ([`super::rooms`]) as Matrix event
//! content objects. This module turns that content into a [`ChatMessageContent`]
//! using the `vachat/agent/*` namespace, so the frontend can render
//! thinking / tool_use / tool_result distinctly and skip push notifications for
//! process content.
//!
//! Conversion has two layers:
//!
//! 1. **Explicit** (shared): if the agent signals the type (custom `msgtype`
//!    `vachat.agent.<type>` or a `vachat_content_type` field), map directly.
//!    This is the ideal, uniform path for any cooperating agent.
//! 2. **Inference** (per-agent): otherwise infer from the `body` content. Each
//!    agent has its own message format, so each agent gets its own `infer`
//!    function. The bot's `agent_type` (set in the admin console) selects which
//!    inference function to apply. Add a new agent by implementing its `infer`
//!    and adding a `match` arm in [`convert_agent_matrix_content`].
//!
//! `thinking` is not distinguished from the final answer yet; both fall through
//! to `text/markdown` / `text/plain`. Inferred `tool_use` / `tool_result` carry
//! no `id`, so they correlate by tool name (soft association).

use std::collections::HashMap;

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

use crate::api::message::ChatMessageContent;

/// Convert a Matrix event content object into a vachat [`ChatMessageContent`].
///
/// `content` is the Matrix event `content` object (carries `body`, `msgtype`,
/// `format`, `formatted_body`, and any custom agent fields). `agent_type` is the
/// sending bot's configured agent type (e.g. `"qwenpaw"`), used to select the
/// Layer 2 inference function.
pub fn convert_agent_matrix_content(content: &Value, agent_type: Option<&str>) -> ChatMessageContent {
    let body = content.get("body").and_then(|v| v.as_str()).unwrap_or("");
    let has_format = content.get("format").is_some();

    // Layer 1: explicit type signal (uniform across agents).
    if let Some(out) = try_explicit(content, body) {
        return out;
    }

    // Layer 2: per-agent content-feature inference, dispatched by agent_type.
    let inferred = match agent_type {
        Some("qwenpaw") => qwenpaw::infer(body),
        Some("hermes") => hermes::infer(body),
        Some("cc_connect") => cc_connect::infer(body),
        _ => None,
    };
    if let Some(out) = inferred {
        return out;
    }

    // Default: prose (markdown if formatted, else plain) -- existing behavior.
    ChatMessageContent {
        properties: None,
        content_type: if has_format {
            "text/markdown".to_string()
        } else {
            "text/plain".to_string()
        },
        content: body.to_string(),
    }
}

// =============================== Layer 1 ===============================

/// Read an explicit agent content-type signal, if any.
///
/// Recognizes `vachat_content_type: "vachat/agent/<type>"` (field) and
/// `msgtype: "vachat.agent.<type>"` (custom Matrix msgtype), returning the
/// normalized `vachat/agent/<type>` string.
fn explicit_agent_type(content: &Value) -> Option<String> {
    if let Some(v) = content
        .get("vachat_content_type")
        .and_then(|v| v.as_str())
    {
        if v.starts_with("vachat/agent/") {
            return Some(v.to_string());
        }
    }
    if let Some(m) = content.get("msgtype").and_then(|v| v.as_str()) {
        if let Some(rest) = m.strip_prefix("vachat.agent.") {
            return Some(format!("vachat/agent/{}", rest));
        }
    }
    None
}

/// Layer 1: map an explicit agent content-type signal to a [`ChatMessageContent`].
fn try_explicit(content: &Value, body: &str) -> Option<ChatMessageContent> {
    let ty = explicit_agent_type(content)?;

    match ty.as_str() {
        "vachat/agent/thinking" => Some(ChatMessageContent {
            properties: None,
            content_type: "vachat/agent/thinking".to_string(),
            content: body.to_string(),
        }),
        "vachat/agent/tool_use" => {
            let name = content
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let properties = collect_properties([
                ("id", content.get("id")),
                ("input", content.get("input")),
            ]);
            Some(ChatMessageContent {
                properties,
                content_type: "vachat/agent/tool_use".to_string(),
                content: name,
            })
        }
        "vachat/agent/tool_result" => {
            let tool_use_id = content
                .get("tool_use_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let properties = collect_properties([
                ("result", content.get("result")),
                ("is_error", content.get("is_error")),
            ]);
            Some(ChatMessageContent {
                properties,
                content_type: "vachat/agent/tool_result".to_string(),
                content: tool_use_id,
            })
        }
        // Unknown / future vachat/agent/* value: pass through, no notification.
        other => Some(ChatMessageContent {
            properties: None,
            content_type: other.to_string(),
            content: body.to_string(),
        }),
    }
}

/// Collect present `(key, value)` pairs into a properties map, or `None` if empty.
fn collect_properties<'a>(
    pairs: impl IntoIterator<Item = (&'a str, Option<&'a Value>)>,
) -> Option<HashMap<String, Value>> {
    let mut properties = HashMap::new();
    for (key, value) in pairs {
        if let Some(value) = value {
            properties.insert(key.to_string(), value.clone());
        }
    }
    if properties.is_empty() {
        None
    } else {
        Some(properties)
    }
}

// =============================== Layer 2 ===============================

/// QwenPaw (AgentScope-based) message format.
///
/// Renders a tool call as `🔧 **<name>**` followed by a fenced code block
/// holding the JSON input, and a tool result as `✅ **<name>**:` followed by a
/// fenced code block holding the result text.
mod qwenpaw {
    use super::*;

    /// `🔧 **<name>**` header at the start of the body -> tool_use.
    static TOOL_USE_HEADER_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^🔧\s*\*\*(?P<name>[^*]+)\*\*").unwrap());

    /// `✅ **<name>**` header at the start of the body -> tool_result.
    static TOOL_RESULT_HEADER_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^✅\s*\*\*(?P<name>[^*]+)\*\*").unwrap());

    /// First fenced code block (triple backtick, optional language tag).
    static FENCED_CODE_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?s)```[^\n]*\n(?P<code>.*?)```").unwrap());

    pub fn infer(body: &str) -> Option<ChatMessageContent> {
        try_tool_use(body).or_else(|| try_tool_result(body))
    }

    fn try_tool_use(body: &str) -> Option<ChatMessageContent> {
        let caps = TOOL_USE_HEADER_RE.captures(body)?;
        let name = caps
            .name("name")
            .map(|m| m.as_str().trim())
            .unwrap_or("")
            .to_string();
        let properties = extract_first_code_block(body).map(|code| {
            // Parse the code block as JSON when possible; otherwise keep the string.
            let input =
                serde_json::from_str::<Value>(&code).unwrap_or_else(|_| Value::String(code));
            let mut map = HashMap::new();
            map.insert("input".to_string(), input);
            map
        });
        Some(ChatMessageContent {
            properties,
            content_type: "vachat/agent/tool_use".to_string(),
            content: name,
        })
    }

    fn try_tool_result(body: &str) -> Option<ChatMessageContent> {
        let caps = TOOL_RESULT_HEADER_RE.captures(body)?;
        let name = caps
            .name("name")
            .map(|m| m.as_str().trim())
            .unwrap_or("")
            .to_string();
        let properties = extract_first_code_block(body).map(|code| {
            let mut map = HashMap::new();
            map.insert("result".to_string(), Value::String(code));
            map
        });
        Some(ChatMessageContent {
            properties,
            content_type: "vachat/agent/tool_result".to_string(),
            content: name,
        })
    }

    /// Extract the inner content of the first fenced code block, trimmed.
    fn extract_first_code_block(body: &str) -> Option<String> {
        let caps = FENCED_CODE_RE.captures(body)?;
        let code = caps.name("code")?.as_str();
        Some(code.trim().to_string())
    }
}

/// Hermes agent message format.
///
/// Not yet characterized: the Hermes manual only documents installation and
/// Matrix configuration, not its message rendering. [`infer`] returns `None` so
/// Hermes messages fall through to prose (`text/markdown` / `text/plain`).
/// Provide a sample Hermes turn (thinking / tool_use / tool_result / answer) to
/// implement this.
mod hermes {
    use super::*;

    pub fn infer(_body: &str) -> Option<ChatMessageContent> {
        None
    }
}

/// cc-connect (Claude Code) message format.
///
/// Not yet characterized. If cc-connect shares QwenPaw's `🔧`/`✅` rendering,
/// route it through [`super::qwenpaw::infer`] instead of implementing a
/// duplicate here.
mod cc_connect {
    use super::*;

    pub fn infer(_body: &str) -> Option<ChatMessageContent> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text_content(body: &str) -> Value {
        json!({ "msgtype": "m.text", "body": body })
    }

    // ---- Layer 2: QwenPaw inference (dispatched by agent_type) ----

    #[test]
    fn qwenpaw_tool_use_json_block() {
        let body = "🔧 **execute_shell_command**\n```\n{\"command\": \"ls\"}\n```";
        let out = convert_agent_matrix_content(&text_content(body), Some("qwenpaw"));
        assert_eq!(out.content_type, "vachat/agent/tool_use");
        assert_eq!(out.content, "execute_shell_command");
        let properties = out.properties.expect("properties");
        let input = properties.get("input").unwrap();
        assert_eq!(input.get("command").unwrap(), "ls");
    }

    #[test]
    fn qwenpaw_tool_use_non_json_block() {
        let body = "🔧 **search**\n```\nnot json at all\n```";
        let out = convert_agent_matrix_content(&text_content(body), Some("qwenpaw"));
        assert_eq!(out.content_type, "vachat/agent/tool_use");
        assert_eq!(out.content, "search");
        let properties = out.properties.expect("properties");
        let input = properties.get("input").unwrap();
        assert_eq!(input.as_str().unwrap(), "not json at all");
    }

    #[test]
    fn qwenpaw_tool_use_no_code_block() {
        let out = convert_agent_matrix_content(&text_content("🔧 **foo**"), Some("qwenpaw"));
        assert_eq!(out.content_type, "vachat/agent/tool_use");
        assert_eq!(out.content, "foo");
        assert!(out.properties.is_none());
    }

    #[test]
    fn qwenpaw_tool_result() {
        let body = "✅ **execute_shell_command**:\n```\n2026年7月25日 22:03:29\n```";
        let out = convert_agent_matrix_content(&text_content(body), Some("qwenpaw"));
        assert_eq!(out.content_type, "vachat/agent/tool_result");
        assert_eq!(out.content, "execute_shell_command");
        assert_eq!(
            out.properties
                .expect("properties")
                .get("result")
                .unwrap()
                .as_str()
                .unwrap(),
            "2026年7月25日 22:03:29"
        );
    }

    #[test]
    fn qwenpaw_real_tool_use_body() {
        // Captured from a real QwenPaw turn (powershell tool call).
        let body = "🔧 **execute_shell_command**\n```\n{\"command\": \"powershell -Command \\\"[System.TimeZoneInfo]::ConvertTimeBySystemTimeZoneId([DateTime]::UtcNow, 'Eastern Standard Time')\\\"\"}\n```";
        let out = convert_agent_matrix_content(&text_content(body), Some("qwenpaw"));
        assert_eq!(out.content_type, "vachat/agent/tool_use");
        assert_eq!(out.content, "execute_shell_command");
        let properties = out.properties.expect("properties");
        let input = properties.get("input").unwrap();
        assert!(input.get("command").unwrap().as_str().unwrap().contains("powershell"));
    }

    #[test]
    fn qwenpaw_real_tool_result_body() {
        // Captured from a real QwenPaw turn (CRLF inside the code block).
        let body = "✅ **execute_shell_command**:\n```\n\r\n2026年7月25日 22:03:29\r\n\r\n\r\n```";
        let out = convert_agent_matrix_content(&text_content(body), Some("qwenpaw"));
        assert_eq!(out.content_type, "vachat/agent/tool_result");
        assert_eq!(
            out.properties
                .expect("properties")
                .get("result")
                .unwrap()
                .as_str()
                .unwrap(),
            "2026年7月25日 22:03:29"
        );
    }

    #[test]
    fn qwenpaw_thinking_stays_prose() {
        // thinking is not distinguished from the final answer yet.
        let out = convert_agent_matrix_content(
            &text_content("The user wants to know the time in New York."),
            Some("qwenpaw"),
        );
        assert_eq!(out.content_type, "text/plain");
    }

    // ---- Agent type dispatch ----

    #[test]
    fn unknown_agent_type_falls_through_to_prose() {
        let out = convert_agent_matrix_content(&text_content("🔧 **foo**"), Some("hermes"));
        assert_eq!(out.content_type, "text/plain");
    }

    #[test]
    fn no_agent_type_falls_through_to_prose() {
        let out = convert_agent_matrix_content(&text_content("🔧 **foo**"), None);
        assert_eq!(out.content_type, "text/plain");
    }

    // ---- Prose fallback ----

    #[test]
    fn prose_markdown_with_format() {
        let c = json!({
            "msgtype": "m.text",
            "body": "# hi",
            "format": "org.matrix.custom.html",
            "formatted_body": "<h1>hi</h1>"
        });
        let out = convert_agent_matrix_content(&c, Some("qwenpaw"));
        assert_eq!(out.content_type, "text/markdown");
        assert_eq!(out.content, "# hi");
    }

    #[test]
    fn prose_plain_without_format() {
        let out = convert_agent_matrix_content(&text_content("hello"), Some("qwenpaw"));
        assert_eq!(out.content_type, "text/plain");
        assert_eq!(out.content, "hello");
    }

    // ---- Layer 1: explicit signal (uniform, takes precedence) ----

    #[test]
    fn explicit_thinking_msgtype() {
        let c = json!({ "msgtype": "vachat.agent.thinking", "body": "analyzing" });
        let out = convert_agent_matrix_content(&c, Some("qwenpaw"));
        assert_eq!(out.content_type, "vachat/agent/thinking");
        assert_eq!(out.content, "analyzing");
        assert!(out.properties.is_none());
    }

    #[test]
    fn explicit_tool_use_fields() {
        let c = json!({
            "msgtype": "vachat.agent.tool_use",
            "name": "search",
            "id": "tu_1",
            "input": { "query": "rust" }
        });
        let out = convert_agent_matrix_content(&c, Some("qwenpaw"));
        assert_eq!(out.content_type, "vachat/agent/tool_use");
        assert_eq!(out.content, "search");
        let props = out.properties.expect("properties");
        assert_eq!(props.get("id").unwrap(), "tu_1");
        assert_eq!(props.get("input").unwrap().get("query").unwrap(), "rust");
    }

    #[test]
    fn explicit_tool_result_fields() {
        let c = json!({
            "vachat_content_type": "vachat/agent/tool_result",
            "tool_use_id": "tu_1",
            "result": "found 3 hits",
            "is_error": false
        });
        let out = convert_agent_matrix_content(&c, Some("qwenpaw"));
        assert_eq!(out.content_type, "vachat/agent/tool_result");
        assert_eq!(out.content, "tu_1");
        let props = out.properties.expect("properties");
        assert_eq!(props.get("result").unwrap(), "found 3 hits");
        assert_eq!(props.get("is_error").unwrap(), false);
    }

    #[test]
    fn explicit_unknown_agent_type_passes_through() {
        let c = json!({ "msgtype": "vachat.agent.status", "body": "running" });
        let out = convert_agent_matrix_content(&c, Some("qwenpaw"));
        assert_eq!(out.content_type, "vachat/agent/status");
        assert_eq!(out.content, "running");
        assert!(out.properties.is_none());
    }

    #[test]
    fn explicit_takes_precedence_over_inference() {
        // Even with a 🔧 body, an explicit thinking signal wins.
        let c = json!({ "msgtype": "vachat.agent.thinking", "body": "🔧 **foo**" });
        let out = convert_agent_matrix_content(&c, Some("qwenpaw"));
        assert_eq!(out.content_type, "vachat/agent/thinking");
    }
}
