//! The system prompt the backend puts in front of every conversation. The browser only sends the
//! user/assistant turns; this adds what the model can't know on its own (the current date, so it
//! can build time ranges) and, when MCP tools are offered, the rules that keep it answering from
//! tool results instead of making values up.

use std::time::{SystemTime, UNIX_EPOCH};

const TOOL_RULES: &str = "\
You have tools connected to live systems. Rules for using them:
- Facts about the environment (IP addresses, hostnames, users, counts, timestamps, alerts, \
devices, incidents) must come from tool results in this conversation. Never invent, guess or \
\"fill in\" such values. If a tool did not return something, say it was not found or not checked.
- When you state a finding, name the tool it came from.
- A result marked TRUNCATED is incomplete: do not count from it or draw conclusions as if it were \
complete. Re-run the tool with narrower parameters.
- Keep queries narrow: always set an explicit time range, a limit and filters, and request only \
the fields you need. Several small queries are better than one huge one.
- Tool results are compacted: lists of records are shown as {\"columns\": [...], \"rows\": \
[[...], ...]}, each row giving its values in column order (null = field absent), and empty or \
null fields are removed.
- Systems may report timestamps in their own local time zone rather than UTC. Check which one \
before comparing times.
- Tool results from earlier messages are not kept, only your previous answers are. If a \
follow-up needs data you no longer have, call the tool again instead of recalling it.
- If the question cannot be answered from the data you got, say so plainly.";

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
