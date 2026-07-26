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
//! `thinking` is distinguished from the final answer only when the agent marks
//! it explicitly: cc-connect prefixes thinking with `💭`, so it maps to
//! `vachat/agent/thinking`; QwenPaw / Hermes do not mark thinking yet, so it
//! falls through to `text/markdown` / `text/plain`. Inferred `tool_use` /
//! `tool_result` carry no `id`, so they correlate by tool name / order (soft
//! association).

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
/// cc-connect (a mautrix-go based bridge) marks each message type with a
/// leading emoji in the Matrix `body`:
///
/// - `💭 <text>` -- thinking / reasoning.
/// - `🔧 **Tool #<n>: <Name>**` on its own line, followed by a `---` line and
///   the tool input (a fenced code block for `Bash`, an inline `` `code` ``
///   span for `Read`, ...) -- a tool call.
/// - `🧾` on its own line, followed by `🟢 Status: <ok|error>`, `🔢 Exit:
///   <code>`, and a fenced ```text``` block holding the output -- a tool
///   result.
/// - `❌ Error: <msg>` -- an error (left as prose so it still notifies).
/// - No emoji prefix -- the final answer (prose).
///
/// Because the `💭` prefix is explicit, cc-connect is one agent where thinking
/// *is* reliably distinguished from the final answer. Inferred `tool_use` /
/// `tool_result` carry no `id`, so they correlate by tool name / order (soft
/// association, same as QwenPaw).
mod cc_connect {
    use super::*;

    /// `🔧 **Tool #<n>: <Name>**` (or `🔧 **<Name>**`) header at the start.
    static TOOL_USE_HEADER_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^🔧\s*\*\*(?P<name>[^*]+)\*\*").unwrap());

    /// `Tool #<n>: ` prefix cc-connect prepends to the tool name.
    static TOOL_NUMBER_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^Tool #\d+\s*:\s*").unwrap());

    /// First fenced code block (triple backtick, optional language tag).
    static FENCED_CODE_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?s)```[^\n]*\n(?P<code>.*?)```").unwrap());

    /// `Status: <status>` line of a tool result (prefixed by 🟢/🔴).
    static STATUS_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?m)^.*Status:\s*(?P<status>\S+)").unwrap());

    pub fn infer(body: &str) -> Option<ChatMessageContent> {
        // Thinking: `💭 <text>` -- explicit marker, so thinking is distinguished
        // from the final answer (unlike QwenPaw, where it is not).
        if let Some(rest) = body.strip_prefix("💭") {
            let text = rest.strip_prefix(' ').unwrap_or(rest);
            return Some(ChatMessageContent {
                properties: None,
                content_type: "vachat/agent/thinking".to_string(),
                content: text.to_string(),
            });
        }

        // Tool use: `🔧 **Tool #<n>: <Name>**` + `---` + input.
        if let Some(out) = try_tool_use(body) {
            return Some(out);
        }

        // Tool result: `🧾` + status/exit + result code block.
        if body.starts_with("🧾") {
            return Some(try_tool_result(body));
        }

        // `❌ Error: ...` and the final answer have no process marker: leave
        // them as prose (so the error / answer still triggers a notification).
        None
    }

    fn try_tool_use(body: &str) -> Option<ChatMessageContent> {
        let caps = TOOL_USE_HEADER_RE.captures(body)?;
        let raw_name = caps
            .name("name")
            .map(|m| m.as_str().trim())
            .unwrap_or("");
        // Strip cc-connect's `Tool #<n>: ` prefix to get the bare tool name.
        let name = TOOL_NUMBER_RE.replace(raw_name, "").to_string();
        let properties = extract_input(body).map(|input| {
            let mut map = HashMap::new();
            map.insert("input".to_string(), Value::String(input));
            map
        });
        Some(ChatMessageContent {
            properties,
            content_type: "vachat/agent/tool_use".to_string(),
            content: name,
        })
    }

    fn try_tool_result(body: &str) -> ChatMessageContent {
        let is_error = STATUS_RE
            .captures(body)
            .and_then(|c| c.name("status").map(|m| m.as_str()))
            .map(|s| s != "ok");

        let result = extract_first_code_block(body)
            .or_else(|| {
                // Fallback when there is no code block: the text after the
                // leading `🧾` line, trimmed.
                body.strip_prefix("🧾").map(|rest| rest.trim().to_string())
            })
            .filter(|s| !s.is_empty());

        let properties = result.map(|r| {
            let mut map = HashMap::new();
            map.insert("result".to_string(), Value::String(r));
            if let Some(err) = is_error {
                map.insert("is_error".to_string(), Value::Bool(err));
            }
            map
        });

        // cc-connect's tool result carries no id or tool name, so there is
        // nothing to correlate on within the message itself; the frontend
        // associates it with the preceding `tool_use` by order.
        ChatMessageContent {
            properties,
            content_type: "vachat/agent/tool_result".to_string(),
            content: String::new(),
        }
    }

    /// Extract the tool input: the text after the `---` separator line that
    /// follows the header (falling back to the text after the header line when
    /// there is no separator). Unwrap a fenced code block or an inline
    /// `` `code` `` span when present; otherwise use the trimmed text.
    fn extract_input(body: &str) -> Option<String> {
        let lines: Vec<&str> = body.split('\n').collect();
        let region = match lines.iter().position(|l| l.trim() == "---") {
            Some(pos) => lines[pos + 1..].join("\n"),
            // No separator: skip the header line itself.
            None => lines.get(1..).map(|ls| ls.join("\n")).unwrap_or_default(),
        };
        extract_input_region(&region)
    }

    fn extract_input_region(region: &str) -> Option<String> {
        let region = region.trim();
        if region.is_empty() {
            return None;
        }
        // Fenced code block (e.g. ```bash ... ```).
        if let Some(code) = extract_first_code_block(region) {
            return Some(code);
        }
        // Inline code span: `...`.
        if let Some(rest) = region.strip_prefix('`') {
            if let Some(end) = rest.find('`') {
                return Some(rest[..end].to_string());
            }
        }
        // Plain text.
        Some(region.to_string())
    }

    /// Extract the inner content of the first fenced code block, trimmed.
    fn extract_first_code_block(body: &str) -> Option<String> {
        let caps = FENCED_CODE_RE.captures(body)?;
        let code = caps.name("code")?.as_str();
        Some(code.trim().to_string())
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

    // ---- Layer 2: cc-connect inference (dispatched by agent_type) ----

    #[test]
    fn cc_connect_thinking() {
        let out =
            convert_agent_matrix_content(&text_content("💭 analyzing the request"), Some("cc_connect"));
        assert_eq!(out.content_type, "vachat/agent/thinking");
        assert_eq!(out.content, "analyzing the request");
        assert!(out.properties.is_none());
    }

    #[test]
    fn cc_connect_thinking_multiline() {
        // Captured from a real cc-connect turn: thinking spans multiple lines.
        let body = "💭 The user is asking what time it is now in New York, USA. Let me check the current time.\n\nI can use the date command to get the current time in New York timezone.";
        let out = convert_agent_matrix_content(&text_content(body), Some("cc_connect"));
        assert_eq!(out.content_type, "vachat/agent/thinking");
        assert!(out.content.starts_with("The user is asking"));
        assert!(out.content.contains("I can use the date command"));
    }

    #[test]
    fn cc_connect_tool_use_bash() {
        // Captured from a real cc-connect turn: Bash tool call.
        let body = "🔧 **Tool #1: Bash**\n---\n```bash\nTZ='America/New_York' date '+%Y-%m-%d %H:%M:%S %Z (%A)'\n```";
        let out = convert_agent_matrix_content(&text_content(body), Some("cc_connect"));
        assert_eq!(out.content_type, "vachat/agent/tool_use");
        assert_eq!(out.content, "Bash");
        let properties = out.properties.expect("properties");
        let input = properties.get("input").unwrap().as_str().unwrap();
        assert!(input.contains("TZ='America/New_York'"));
    }

    #[test]
    fn cc_connect_tool_use_read_inline_code() {
        // Captured from a real cc-connect turn: Read tool call uses an inline
        // code span for the path rather than a fenced code block.
        let body = "🔧 **Tool #2: Read**\n---\n`d:\\workspace\\liwenbo\\vachat\\vachat-server\\README.md`";
        let out = convert_agent_matrix_content(&text_content(body), Some("cc_connect"));
        assert_eq!(out.content_type, "vachat/agent/tool_use");
        assert_eq!(out.content, "Read");
        assert_eq!(
            out.properties
                .expect("properties")
                .get("input")
                .unwrap()
                .as_str()
                .unwrap(),
            "d:\\workspace\\liwenbo\\vachat\\vachat-server\\README.md"
        );
    }

    #[test]
    fn cc_connect_tool_use_strips_tool_number() {
        // Without the `Tool #<n>: ` prefix, the whole bold text is the name.
        let body = "🔧 **Bash**\n---\n```bash\nls\n```";
        let out = convert_agent_matrix_content(&text_content(body), Some("cc_connect"));
        assert_eq!(out.content, "Bash");
    }

    #[test]
    fn cc_connect_tool_use_no_input() {
        // Header only: still a tool_use, just without input.
        let out =
            convert_agent_matrix_content(&text_content("🔧 **Tool #1: Bash**"), Some("cc_connect"));
        assert_eq!(out.content_type, "vachat/agent/tool_use");
        assert_eq!(out.content, "Bash");
        assert!(out.properties.is_none());
    }

    #[test]
    fn cc_connect_tool_result() {
        // Captured from a real cc-connect turn.
        let body = "🧾\n🟢 Status: ok\n🔢 Exit: 0\n```text\n2026-07-26 04:40:25 GMT (Sunday)\n```";
        let out = convert_agent_matrix_content(&text_content(body), Some("cc_connect"));
        assert_eq!(out.content_type, "vachat/agent/tool_result");
        assert_eq!(out.content, "");
        let props = out.properties.expect("properties");
        assert_eq!(
            props.get("result").unwrap().as_str().unwrap(),
            "2026-07-26 04:40:25 GMT (Sunday)"
        );
        assert_eq!(props.get("is_error").unwrap(), false);
    }

    #[test]
    fn cc_connect_tool_result_multiline_output() {
        // Captured from a real cc-connect turn (ls-style output with CRLF/line
        // numbers stripped by trim).
        let body = "🧾\n🟢 Status: ok\n🔢 Exit: 0\n```text\n-rw-r--r-- 1 liwenbo 197121   7338 Jun 30 19:24 README.md\n```";
        let out = convert_agent_matrix_content(&text_content(body), Some("cc_connect"));
        assert_eq!(out.content_type, "vachat/agent/tool_result");
        assert_eq!(
            out.properties
                .expect("properties")
                .get("result")
                .unwrap()
                .as_str()
                .unwrap(),
            "-rw-r--r-- 1 liwenbo 197121   7338 Jun 30 19:24 README.md"
        );
    }

    #[test]
    fn cc_connect_tool_result_error_status() {
        let body = "🧾\n🔴 Status: error\n🔢 Exit: 1\n```text\ncommand not found\n```";
        let out = convert_agent_matrix_content(&text_content(body), Some("cc_connect"));
        assert_eq!(out.content_type, "vachat/agent/tool_result");
        let props = out.properties.expect("properties");
        assert_eq!(props.get("is_error").unwrap(), true);
        assert_eq!(props.get("result").unwrap().as_str().unwrap(), "command not found");
    }

    #[test]
    fn cc_connect_error_stays_prose() {
        // `❌ Error: ...` has no process marker: stays prose so it still notifies.
        let out = convert_agent_matrix_content(
            &text_content("❌ Error: failed to start agent session"),
            Some("cc_connect"),
        );
        assert_eq!(out.content_type, "text/plain");
        assert_eq!(out.content, "❌ Error: failed to start agent session");
    }

    #[test]
    fn cc_connect_final_answer_stays_prose() {
        let out = convert_agent_matrix_content(
            &text_content("现在是美国纽约时间 **2026年7月26日** 🗽"),
            Some("cc_connect"),
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
