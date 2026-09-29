//! Turns a raw MCP tool result into what the model gets to see: compacted (same information, far
//! fewer characters), capped in size, and with a paging notice when it only holds part of what
//! matched. The system prompt (prompt.rs) explains the compacted shape and the notices to the
//! model.

use serde_json::{json, Value};
use std::collections::HashSet;

/// Tools whose integer `tid` is a reusable paging handle for `fetch_more_logs` (the FortiAnalyzer
/// MCP's `search_ips_logs` also returns a `tid`, but a dead one).
const PAGEABLE_TOOLS: &[&str] = &["query_logs", "fetch_more_logs"];

/// Fields a result may use to hold its list of records, when it doesn't give a `count`.
const RECORD_FIELDS: &[&str] = &["logs", "data", "results", "items", "alerts", "incidents"];

/// `tool` is the MCP tool name (without the "{server}__" prefix), `input` its arguments, and
/// `max_chars` the size cap (`[agent] max_tool_result_chars`).
pub fn prepare(tool: &str, input: &Value, text: String, max_chars: usize) -> String {
    let raw_len = text.len();
    let (body, notice) = match serde_json::from_str::<Value>(&text) {
        Ok(v) => {
            let notice = paging_notice(tool, input, &v);
            (serde_json::to_string(&compact_value(v)).unwrap_or(text), notice)
        }
        Err(_) => (text, None),
    };
    tracing::info!(
        "tool {tool}: {raw_len} chars, {} after compaction{}",
        body.len(),
        if notice.is_some() { ", partial (paging notice added)" } else { "" }
    );
    let mut out = truncate(body, max_chars);
    if let Some(notice) = notice {
        out.push_str("\n\n");
        out.push_str(&notice);
    }
    out
}

/// A page of 100 rows out of 12,345 matches looks like a complete answer unless someone says
/// otherwise; models reliably treat it as the whole picture. So when a result reports a `total`
/// larger than what it returned, say so in plain words and say how to get the rest.
fn paging_notice(tool: &str, input: &Value, v: &Value) -> Option<String> {
    // Aggregates and counts already cover every match.
    let is_true = |k: &str| input.get(k).and_then(Value::as_bool) == Some(true);
    let is_set = |k: &str| input.get(k).is_some_and(|x| !x.is_null());
    if is_true("count_only") || is_set("group_by") || is_set("sample_by") {
        return None;
    }

    let obj = v.as_object()?;
    let total = ["total", "page_total"]
        .iter()
        .find_map(|k| obj.get(*k).and_then(Value::as_u64))?;
    let returned = obj.get("count").and_then(Value::as_u64).or_else(|| {
        RECORD_FIELDS
            .iter()
            .find_map(|k| obj.get(*k).and_then(Value::as_array).map(|a| a.len() as u64))
    })?;
    let offset = input.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let seen = offset + returned;
    if returned == 0 || seen >= total {
        return None;
    }

    let next = match obj.get("tid").and_then(Value::as_u64) {
        Some(tid) if PAGEABLE_TOOLS.contains(&tool) => {
            format!("Get the next page with fetch_more_logs(tid={tid}, offset={seen})")
        }
        _ => "Get more by raising `limit` or paging with `offset`".to_string(),
    };
    Some(format!(
        "[PAGING NOTICE: this result holds matches {}-{seen} of {total}. It is a SAMPLE, not the \
         full result: do not count from it or describe it as complete. {next}, or use \
         count_only / group_by to get totals over all {total} matches.]",
        offset + 1
    ))
}

/// Shrinks a JSON value without losing information: drops null/empty fields and turns lists of
/// records into a column header plus rows (log results repeat every key on every record).
fn compact_value(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, compact_value(v)))
                .filter(|(_, v)| !is_empty(v))
                .collect(),
        ),
        Value::Array(items) => {
            let items: Vec<Value> = items.into_iter().map(compact_value).collect();
            if items.len() >= 2 && items.iter().all(Value::is_object) {
                tabulate(items)
            } else {
                Value::Array(items)
            }
        }
        other => other,
    }
}

fn is_empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// `items` must all be objects. Columns keep first-seen order; a record without a column gets null.
fn tabulate(items: Vec<Value>) -> Value {
    let mut columns: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for item in &items {
        if let Value::Object(map) = item {
            for key in map.keys() {
                if seen.insert(key.clone()) {
                    columns.push(key.clone());
                }
            }
        }
    }
    let rows: Vec<Value> = items
        .into_iter()
        .map(|item| match item {
            Value::Object(mut map) => Value::Array(
                columns.iter().map(|c| map.remove(c).unwrap_or(Value::Null)).collect(),
            ),
            other => other,
        })
        .collect();
    json!({ "columns": columns, "rows": rows })
}

/// Tool results are fed back to the model on every later turn, so one huge result (e.g. every
/// UEBA endpoint) can push the conversation past the model's context window. Cap each result and
/// tell the model it was cut, so it can retry with a narrower query instead of trusting a partial.
fn truncate(text: String, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text;
    }
    let mut cut = max_chars;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        "{}\n\n[TRUNCATED: this tool result was {} characters and only the first {} are shown. \
         Do not treat it as complete. Re-run the tool with a narrower query (only the `fields` \
         you need, stricter filters, a shorter time range), or use count_only / group_by.]",
        &text[..cut],
        text.len(),
        cut
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacts_records_into_a_table() {
        let raw: Value = serde_json::from_str(
            r#"{"total": 2, "note": "", "data": [
                {"srcip": "10.0.0.1", "action": "deny", "user": null},
                {"srcip": "10.0.0.2", "action": "accept", "port": 443}
            ]}"#,
        )
        .unwrap();
        let data = &compact_value(raw)["data"];
        assert_eq!(data["columns"], json!(["action", "srcip", "port"]));
        assert_eq!(
            data["rows"],
            json!([["deny", "10.0.0.1", null], ["accept", "10.0.0.2", 443]])
        );
    }

    #[test]
    fn leaves_non_json_alone() {
        assert_eq!(prepare("x", &json!({}), "not json".into(), 1000), "not json");
    }

    #[test]
    fn flags_a_partial_page() {
        let v = json!({ "total": 500, "count": 100, "tid": 7, "logs": [] });
        let notice = paging_notice("query_logs", &json!({}), &v).unwrap();
        assert!(notice.contains("1-100 of 500"));
        assert!(notice.contains("fetch_more_logs(tid=7, offset=100)"));

        let page = paging_notice("fetch_more_logs", &json!({ "offset": 100 }), &v).unwrap();
        assert!(page.contains("101-200 of 500"));
        assert!(page.contains("offset=200"));
    }

    #[test]
    fn no_notice_when_complete_or_aggregated() {
        let v = json!({ "total": 100, "count": 100 });
        assert!(paging_notice("query_logs", &json!({}), &v).is_none());
        let partial = json!({ "total": 500, "count": 10 });
        assert!(paging_notice("query_logs", &json!({ "group_by": "srcip" }), &partial).is_none());
        // search_ips_logs' tid can't be paged with.
        let ips = json!({ "total": 500, "count": 100, "tid": 3 });
        let notice = paging_notice("search_ips_logs", &json!({}), &ips).unwrap();
        assert!(!notice.contains("fetch_more_logs"));
    }
}
