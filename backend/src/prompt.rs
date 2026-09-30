//! The system prompt the backend puts in front of every conversation. The browser only sends the
//! user/assistant turns; this adds what the model can't know on its own (the current date, so it
//! can build time ranges) and, when MCP tools are offered, the rules that keep it answering from
//! tool results instead of making values up.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

const TOOL_RULES: &str = "\
You have tools connected to live systems. You are an investigator: your job is a complete, \
verified answer, not the first plausible one. Keep calling tools until the question is fully \
answered; you have plenty of tool calls available, so do not stop early to save them, and do not \
ask the user for permission to continue when the tools can answer.

How to investigate:
1. Plan: work out which data answers the question (which log types, alerts, devices, time window).
2. Size it first: before pulling rows, get the scale with count_only or group_by / top_n over the \
whole time window.
3. Always pass an explicit time_range that covers the question. Never rely on a tool's default: \
query_logs defaults to the last hour only. Minutes are only right for \"what is happening right \
now\" questions. Discovery and inventory questions (which servers exist, who uses X, does Y \
happen) need at least 24-hour, often 7-day: something quiet for 15 minutes is not absent.
4. Totals, rankings, \"how many\", \"which ones\", \"top\" questions: use aggregation \
(group_by, top_n, count_only, sample_by, FortiView), never a page of rows. If a tool refuses an \
aggregation with your filters, use the alternative its error recommends; don't retry what \
already failed.
5. Aggregate first, rows last. Pull raw rows only to look at specific events, with limit 200 or \
less and only the `fields` you need: a big dump costs a large part of your context and gets cut \
anyway. Page with fetch_more_logs(tid, offset) when every row matters.
Between tool calls, write one short sentence on what the last result showed and what you will \
check next, so you keep track of open leads.
6. Find them all: a sample or top-N only shows the biggest. When you are listing things (servers, \
hosts, users), re-run the query excluding every candidate you already found (a not_in filter), \
and repeat until nothing new appears. Only then is the list complete.
7. Check each lead in both directions: servers also talk to each other, so an IP that shows up \
as a client or source can itself be a server. Before dismissing an IP, query it as a \
destination too.
8. Demand specific evidence: a role needs the protocol or behaviour that is specific to it, not \
generic traffic many machines share. Exclude denied or blocked connections when asking what a \
machine serves: a scanner hitting a port proves nothing.
9. Know what a field means before relying on it. The same field can mean different things in \
different log types (for example a name in a DNS log is the name that was asked for, not the \
server's own name). If unsure, check the field list or look at a few raw rows first.
10. Verify: check that your numbers are consistent with each other, and that a surprising \
finding is not an artefact of a filter or time range. Never claim something is \"unlikely\" or \
\"thoroughly checked\" unless you actually ran the check.

Rules on facts:
- Facts about the environment (IP addresses, hostnames, users, counts, timestamps, alerts, \
devices, incidents) must come from tool results in this conversation. Never invent, guess or \
\"fill in\" such values. If a tool did not return something, say it was not found or not checked.
- A result with a PAGING NOTICE or marked TRUNCATED is incomplete: do not count from it or \
describe it as complete. Page on, aggregate, or narrow the query.
- Tool results are compacted: lists of records are shown as {\"columns\": [...], \"rows\": \
[[...], ...]}, each row giving its values in column order (null = field absent), and empty or \
null fields are removed.
- Systems may report timestamps in their own local time zone rather than UTC. Check which one \
before comparing times.
- Tool results from earlier messages are not kept, only your previous answers are. If a \
follow-up needs data you no longer have, call the tool again instead of recalling it.

How to answer (investigate thoroughly, but write briefly; the reader is a busy analyst):
- First line: the direct answer to the question, in one or two sentences.
- Then only the evidence that supports it, as a compact table or a few bullets with the exact \
values (IPs, hosts, users, counts, times). Name the source briefly (log type or tool) in a column \
or in parentheses, not in a sentence of its own.
- Match the length to the question: a yes/no or single-value question gets a few lines. Do not \
restate your plan or your notes between tool calls, do not describe your method step by step, \
and give no generic advice or recommendations unless asked.
- Last line: \"Coverage:\" then, in one sentence, the time window and sources checked and \
anything not checked or still open.
- If the question cannot be answered from the data, say so plainly.";

/// `servers` are the MCP servers whose tools this request offers (empty = plain chat), `notes`
/// each server's guide (its own instructions plus config.toml `notes`, see main.rs), `mcp_only`
/// the UI's "MCP only" switch, and `extra` the optional `system_prompt` from config.toml,
/// appended as-is.
pub fn system_prompt(
    servers: &[String],
    notes: &HashMap<String, String>,
    mcp_only: bool,
    extra: Option<&str>,
) -> String {
    let mut prompt = format!(
        "You are LASEASK, an assistant for an IT and security team.\n\
         Current date and time: {}. Work out relative time ranges (\"today\", \"last 24 hours\", \
         \"this week\") from it; never guess the date.",
        utc_now()
    );
    if !servers.is_empty() {
        prompt.push_str(&format!(
            "\n\nConnected MCP servers: {}.\n{TOOL_RULES}",
            servers.join(", ")
        ));
        // Also the only enforcement on models that reject a forced tool call (agent.rs).
        if mcp_only {
            prompt.push_str(
                "\n\nMCP-only mode is on: call at least one tool before answering, and base the \
                 answer on tool results, never on your own knowledge alone.",
            );
        }
        for server in servers {
            if let Some(guide) = notes.get(server) {
                prompt.push_str(&format!(
                    "\n\nGuide for the '{server}' tools (their names start with \"{server}__\"):\n{guide}"
                ));
            }
        }
    }
    if let Some(extra) = extra.map(str::trim).filter(|e| !e.is_empty()) {
        prompt.push_str("\n\n");
        prompt.push_str(extra);
    }
    prompt
}

/// The tool part of the prompt alone (investigation rules, then each server's guide), sent as the
/// `instructions` of the MCP tool server (tool_server.rs).
pub fn tool_guide(servers: &[String], notes: &HashMap<String, String>) -> String {
    let mut guide = TOOL_RULES.to_string();
    for server in servers {
        if let Some(g) = notes.get(server) {
            guide.push_str(&format!(
                "\n\nGuide for the '{server}' tools (their names start with \"{server}__\"):\n{g}"
            ));
        }
    }
    guide
}

/// e.g. "Tuesday 2026-09-29 11:04 UTC". Hand-rolled (civil-from-days) to avoid pulling in a date
/// crate for one line.
fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    // 1970-01-01 was a Thursday.
    const WEEKDAYS: [&str; 7] = [
        "Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday",
    ];
    format!(
        "{} {y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        WEEKDAYS[days.rem_euclid(7) as usize],
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// Days since 1970-01-01 -> (year, month, day). Howard Hinnant's algorithm.
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_725), (2026, 9, 29));
    }
}
