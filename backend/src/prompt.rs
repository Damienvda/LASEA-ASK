//! The system prompt the backend puts in front of every conversation. The browser only sends the
//! user/assistant turns; this adds what the model can't know on its own (the current date, so it
//! can build time ranges) and, when MCP tools are offered, the rules that keep it answering from
//! tool results instead of making values up.

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
query_logs defaults to the last hour only.
4. Totals, rankings, \"how many\", \"which ones\", \"top\" questions: use aggregation \
(group_by, top_n, count_only, FortiView), which covers every match. Never count from a page of rows.
5. When you need the rows themselves: request only the `fields` you need, raise `limit` where \
useful, and page with fetch_more_logs(tid, offset) until you have them all or can show the rest \
does not change the answer.
6. Follow every lead: each IP, user, host, alert or incident that matters to the question gets \
its own check (what else did it do, is it known, where else does it appear).
7. Verify: check that your numbers are consistent with each other, and that a surprising finding \
is not an artefact of a filter or time range.

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

How to answer:
- Lead with the direct answer, then the supporting findings, each naming the tool it came from.
- End with a short \"Coverage\" section: what you checked (time window, sources), and anything \
not checked or still open, with the reason.
- If the question cannot be answered from the data, say so plainly.";

/// `servers` are the MCP servers whose tools this request offers (empty = plain chat); `extra` is
/// the optional `system_prompt` from config.toml, appended as-is.
pub fn system_prompt(servers: &[String], extra: Option<&str>) -> String {
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
    }
    if let Some(extra) = extra.map(str::trim).filter(|e| !e.is_empty()) {
        prompt.push_str("\n\n");
        prompt.push_str(extra);
    }
    prompt
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
fn civil_from_days(days: i64) -> (i64, u32, u32) {
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
