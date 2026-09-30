//! External threat intelligence on public IP addresses and domains: who owns them (RDAP, the
//! successor of WHOIS), their reputation (AbuseIPDB, VirusTotal) and the threat-intel reports
//! they appear in (AlienVault OTX). Offered to the model as the built-in `intel__lookup` tool,
//! and to the UI's hover cards through `/api/intel`.
//!
//! Only public indicators leave the network: private, reserved and documentation ranges, and
//! domains listed in `[intel] internal_domains`, are answered locally without any lookup. Results
//! are cached (`cache_hours`), which also keeps the free API quotas in check.
//!
//! One instance for the whole process (`init` at start-up, `get` afterwards), so the tool-use
//! loops can reach it without threading it through every call.

use crate::config::IntelConfig;
use crate::prompt::civil_from_days;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The tool's "{server}__{tool}" name parts.
pub const SERVER: &str = "intel";
pub const TOOL: &str = "lookup";
/// Indicators per tool call: about 1-2k characters of results each.
const MAX_PER_CALL: usize = 10;
const MAX_CACHE_ENTRIES: usize = 5000;

static INTEL: OnceLock<Intel> = OnceLock::new();

pub struct Intel {
    cfg: IntelConfig,
    client: reqwest::Client,
    cache: Mutex<HashMap<String, (Instant, Value)>>,
    /// When the last VirusTotal requests went out, for its per-minute quota.
    vt_calls: Mutex<VecDeque<Instant>>,
}

/// Sets up the lookups from `[intel]`. Returns false when they're turned off.
pub fn init(mut cfg: IntelConfig) -> bool {
    if !cfg.enabled {
        return false;
    }
    // An empty key or the example's placeholder = that source is off.
    for key in [&mut cfg.abuseipdb_api_key, &mut cfg.virustotal_api_key, &mut cfg.otx_api_key] {
        if key.as_deref().is_some_and(|k| k.trim().is_empty() || k.contains("REPLACE")) {
            *key = None;
        }
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(cfg.timeout_seconds.max(1)))
        .user_agent(concat!("laseask/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let sources: Vec<&str> = [
        ("RDAP", cfg.rdap),
        ("AbuseIPDB", cfg.abuseipdb_api_key.is_some()),
        ("VirusTotal", cfg.virustotal_api_key.is_some()),
        ("OTX", cfg.otx_api_key.is_some()),
    ]
    .into_iter()
    .filter_map(|(name, on)| on.then_some(name))
    .collect();
    tracing::info!("intel: lookups enabled via {}", if sources.is_empty() { "no source".to_string() } else { sources.join(", ") });
    let _ = INTEL.set(Intel {
        cfg,
        client,
        cache: Mutex::new(HashMap::new()),
        vt_calls: Mutex::new(VecDeque::new()),
    });
    true
}

pub fn get() -> Option<&'static Intel> {
    INTEL.get()
}

/// Tool description and input schema for the model.
pub fn tool_description() -> String {
    format!(
        "Look up public IP addresses and domain names in external threat intelligence: owner, \
network and country (RDAP/WHOIS), domain registration date, abuse reports and their categories \
(AbuseIPDB), antivirus and blocklist detections (VirusTotal), and threat-intel reports naming \
the indicator, with their campaigns and malware families (AlienVault OTX). Returns a heuristic \
verdict (malicious / suspicious / clean / unknown) with its reasons. Private IPs and internal \
domains are not looked up. Up to {MAX_PER_CALL} indicators per call."
    )
}

pub fn tool_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "indicators": {
                "type": "array",
                "items": { "type": "string" },
                "maxItems": MAX_PER_CALL,
                "description": "IPv4/IPv6 addresses or domain names, e.g. [\"203.0.113.7\", \"example.com\"]."
            }
        },
        "required": ["indicators"]
    })
}

/// Guide added to the system prompt when the tool is offered (see main.rs).
pub const GUIDE: &str = "\
- Look up only the public IPs and domains that matter to the answer: suspects, unusual external \
destinations, top external talkers, anything tied to an alert. Put them in one call.
- The verdict is a heuristic: say which source reported what. A clean or unknown result does not \
prove an address is safe, and shared cloud or CDN ranges (AWS, Azure, Google, Cloudflare, Akamai) \
say little about one connection.
- In the answer, give each key external IP or domain its context in a few words: owner or ASN, \
country, verdict and, if any, the linked threats (abuse categories, detections, campaign or \
malware names).";

enum Kind {
    Ip(IpAddr),
    Domain(String),
}

impl Intel {
    /// The tool call: `{"indicators": [...]}`, returned as the JSON text for the model.
    pub async fn lookup_many(&self, input: &Value) -> String {
        let mut wanted: Vec<String> = Vec::new();
        let items = match input.get("indicators") {
            Some(Value::Array(a)) => a.clone(),
            Some(v @ Value::String(_)) => vec![v.clone()],
            _ => input.get("indicator").cloned().into_iter().collect(),
        };
        for item in items {
            if let Some(s) = item.as_str() {
                let s = s.trim().to_string();
                if !s.is_empty() && !wanted.contains(&s) {
                    wanted.push(s);
                }
            }
        }
        if wanted.is_empty() {
            return "Error: pass {\"indicators\": [\"<ip or domain>\", ...]}".to_string();
        }
        let skipped = wanted.len().saturating_sub(MAX_PER_CALL);
        wanted.truncate(MAX_PER_CALL);
        let results = futures::future::join_all(wanted.iter().map(|w| self.lookup(w))).await;
        let mut out = json!({ "results": results });
        if skipped > 0 {
            out["note"] = json!(format!(
                "{skipped} more indicator(s) not looked up: at most {MAX_PER_CALL} per call."
            ));
        }
        out.to_string()
    }

    /// Everything known about one indicator.
    pub async fn lookup(&self, raw: &str) -> Value {
        let Some((indicator, kind)) = normalize(raw) else {
            return json!({ "indicator": raw, "error": "not an IP address or a domain name" });
        };
        let type_name = match kind {
            Kind::Ip(_) => "ip",
            Kind::Domain(_) => "domain",
        };
        let private = match &kind {
            Kind::Ip(ip) => is_private_ip(ip),
            Kind::Domain(d) => self.is_internal_domain(d),
        };
        if private {
            return json!({
                "indicator": indicator,
                "type": type_name,
                "scope": "private",
                "note": "Private address or internal domain: not looked up externally.",
            });
        }
        if let Some(hit) = self.cached(&indicator) {
            return hit;
        }

        let (whois, abuse, vt, otx) =
            tokio::join!(self.rdap(&kind), self.abuseipdb(&kind), self.virustotal(&kind), self.otx(&kind));

        let mut sources = Map::new();
        let mut errors = Vec::new();
        for (name, result) in [("whois", whois), ("abuseipdb", abuse), ("virustotal", vt), ("otx", otx)] {
            match result {
                Some(Ok(v)) => {
                    sources.insert(name.to_string(), v);
                }
                Some(Err(e)) => errors.push(format!("{name}: {e:#}")),
                None => {}
            }
        }
        let (verdict, reasons) = verdict(&sources);

        let mut out = json!({
            "indicator": indicator,
            "type": type_name,
            "scope": "public",
            "verdict": verdict,
            "reasons": reasons,
        });
        for (k, v) in sources {
            out[k.as_str()] = v;
        }
        if !errors.is_empty() {
            out["errors"] = json!(errors);
        } else {
            // Failed lookups (rate limit, timeout) aren't cached, so the next try can succeed.
            self.store(&indicator, &out);
        }
        out
    }

    fn cached(&self, key: &str) -> Option<Value> {
        let ttl = Duration::from_secs(self.cfg.cache_hours * 3600);
        let cache = self.cache.lock().ok()?;
        cache.get(key).filter(|(at, _)| at.elapsed() < ttl).map(|(_, v)| v.clone())
    }

    fn store(&self, key: &str, value: &Value) {
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len() >= MAX_CACHE_ENTRIES {
                cache.clear();
            }
            cache.insert(key.to_string(), (Instant::now(), value.clone()));
        }
    }

    fn is_internal_domain(&self, d: &str) -> bool {
        const PRIVATE_TLDS: &[&str] =
            &["local", "lan", "internal", "intranet", "corp", "home", "localdomain", "arpa", "test", "localhost", "invalid"];
        let tld = d.rsplit('.').next().unwrap_or("");
        PRIVATE_TLDS.contains(&tld)
            || self.cfg.internal_domains.iter().any(|suffix| {
                let suffix = suffix.trim().trim_start_matches('.').to_ascii_lowercase();
                !suffix.is_empty() && (d == suffix || d.ends_with(&format!(".{suffix}")))
            })
    }

    /// GET returning the JSON body, or None on 404 (not in that source's database).
    async fn get_json(&self, req: reqwest::RequestBuilder) -> anyhow::Result<Option<Value>> {
        let resp = req.send().await?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            let text: String = resp.text().await.unwrap_or_default().chars().take(200).collect();
            anyhow::bail!("HTTP {status}: {text}");
        }
        Ok(Some(resp.json().await?))
    }

    // ---------- RDAP (WHOIS) ----------

    async fn rdap(&self, kind: &Kind) -> Option<anyhow::Result<Value>> {
        if !self.cfg.rdap {
            return None;
        }
        Some(match kind {
            Kind::Ip(ip) => self.rdap_ip(*ip).await,
            Kind::Domain(d) => self.rdap_domain(d).await,
        })
    }

    async fn rdap_ip(&self, ip: IpAddr) -> anyhow::Result<Value> {
        let req = self
            .client
            .get(format!("https://rdap.org/ip/{ip}"))
            .header("accept", "application/rdap+json");
        let Some(v) = self.get_json(req).await? else {
            return Ok(json!({ "found": false }));
        };
        let cidr = v.pointer("/cidr0_cidrs/0").and_then(|c| {
            let prefix = c.get("v4prefix").or_else(|| c.get("v6prefix"))?.as_str()?;
            Some(format!("{prefix}/{}", c.get("length")?.as_u64()?))
        });
        let org = find_entity(&v, "registrant").and_then(|e| vcard(e, "fn"));
        let abuse = find_entity(&v, "abuse").and_then(|e| vcard(e, "email"));
        Ok(compact(json!({
            "network": cidr.or_else(|| {
                Some(format!("{} - {}", v.get("startAddress")?.as_str()?, v.get("endAddress")?.as_str()?))
            }),
            "name": v.get("name"),
            "organisation": org,
            "country": v.get("country"),
            "abuse_contact": abuse,
        })))
    }

    async fn rdap_domain(&self, domain: &str) -> anyhow::Result<Value> {
        // RDAP knows registered domains, not hostnames: www.a.example.com -> a.example.com ->
        // example.com until one is found.
        let mut name = domain.to_string();
        let v = loop {
            let req = self
                .client
                .get(format!("https://rdap.org/domain/{name}"))
                .header("accept", "application/rdap+json");
            if let Some(v) = self.get_json(req).await? {
                break v;
            }
            match name.split_once('.') {
                Some((_, rest)) if rest.contains('.') => name = rest.to_string(),
                _ => return Ok(json!({ "found": false })),
            }
        };
        let event = |action: &str| {
            v.get("events")?
                .as_array()?
                .iter()
                .find(|e| e.get("eventAction").and_then(Value::as_str) == Some(action))?
                .get("eventDate")?
                .as_str()
                .map(|d| d.chars().take(10).collect::<String>())
        };
        let registered = event("registration");
        let age_days = registered.as_deref().and_then(days_since);
        let nameservers: Vec<&str> = v
            .get("nameservers")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|n| n.get("ldhName").and_then(Value::as_str)).take(4).collect())
            .unwrap_or_default();
        Ok(compact(json!({
            "registered_domain": name,
            "registrar": find_entity(&v, "registrar").and_then(|e| vcard(e, "fn")),
            "registrant": find_entity(&v, "registrant").and_then(|e| vcard(e, "fn")),
            "registered": registered,
            "age_days": age_days,
            "expires": event("expiration"),
            "status": v.get("status"),
            "nameservers": nameservers,
        })))
    }

    // ---------- AbuseIPDB ----------

    async fn abuseipdb(&self, kind: &Kind) -> Option<anyhow::Result<Value>> {
        let key = self.cfg.abuseipdb_api_key.as_deref()?;
        let Kind::Ip(ip) = kind else { return None };
        Some(self.abuseipdb_check(key, *ip).await)
    }

    async fn abuseipdb_check(&self, key: &str, ip: IpAddr) -> anyhow::Result<Value> {
        let req = self
            .client
            .get("https://api.abuseipdb.com/api/v2/check")
            .query(&[("ipAddress", ip.to_string()), ("maxAgeInDays", "90".into()), ("verbose", "true".into())])
            .header("Key", key)
            .header("accept", "application/json");
        let v = self.get_json(req).await?.unwrap_or_default();
        let d = v.get("data").cloned().unwrap_or_default();
        let reports = d.get("reports").and_then(Value::as_array).cloned().unwrap_or_default();

        let mut counts: HashMap<u64, usize> = HashMap::new();
        for r in &reports {
            for c in r.get("categories").and_then(Value::as_array).into_iter().flatten() {
                if let Some(c) = c.as_u64() {
                    *counts.entry(c).or_default() += 1;
                }
            }
        }
        let mut counts: Vec<(u64, usize)> = counts.into_iter().collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let categories: Vec<String> =
            counts.iter().take(6).map(|(c, n)| format!("{} ({n})", abuse_category(*c))).collect();
        let recent: Vec<String> = reports
            .iter()
            .filter_map(|r| {
                let comment = r.get("comment")?.as_str()?.trim();
                if comment.is_empty() {
                    return None;
                }
                let date: String = r.get("reportedAt")?.as_str()?.chars().take(10).collect();
                Some(format!("{date}: {}", comment.chars().take(160).collect::<String>()))
            })
            .take(3)
            .collect();

        Ok(compact(json!({
            "confidence_score": d.get("abuseConfidenceScore"),
            "total_reports": d.get("totalReports"),
            "distinct_reporters": d.get("numDistinctUsers"),
            "last_reported": d.get("lastReportedAt").and_then(Value::as_str).map(|s| s.chars().take(10).collect::<String>()),
            "report_categories": categories,
            "recent_reports": recent,
            "usage_type": d.get("usageType"),
            "isp": d.get("isp"),
            "domain": d.get("domain"),
            "country": d.get("countryCode"),
            "is_tor": d.get("isTor").filter(|t| t.as_bool() == Some(true)),
            "whitelisted": d.get("isWhitelisted").filter(|t| t.as_bool() == Some(true)),
        })))
    }

    // ---------- VirusTotal ----------

    async fn virustotal(&self, kind: &Kind) -> Option<anyhow::Result<Value>> {
        let key = self.cfg.virustotal_api_key.as_deref()?;
        Some(self.virustotal_check(key, kind).await)
    }

    async fn virustotal_check(&self, key: &str, kind: &Kind) -> anyhow::Result<Value> {
        {
            let mut calls = self.vt_calls.lock().map_err(|_| anyhow::anyhow!("lock poisoned"))?;
            while calls.front().is_some_and(|t| t.elapsed() > Duration::from_secs(60)) {
                calls.pop_front();
            }
            if calls.len() >= self.cfg.virustotal_per_minute.max(1) {
                anyhow::bail!(
                    "skipped: {} lookups per minute already used ([intel] virustotal_per_minute); retry in a minute",
                    self.cfg.virustotal_per_minute
                );
            }
            calls.push_back(Instant::now());
        }
        let url = match kind {
            Kind::Ip(ip) => format!("https://www.virustotal.com/api/v3/ip_addresses/{ip}"),
            Kind::Domain(d) => format!("https://www.virustotal.com/api/v3/domains/{d}"),
        };
        let Some(v) = self.get_json(self.client.get(url).header("x-apikey", key)).await? else {
            return Ok(json!({ "found": false }));
        };
        let a = v.pointer("/data/attributes").cloned().unwrap_or_default();
        let stat = |k: &str| a.pointer(&format!("/last_analysis_stats/{k}")).and_then(Value::as_u64).unwrap_or(0);
        let flagged: Vec<String> = a
            .get("last_analysis_results")
            .and_then(Value::as_object)
            .map(|engines| {
                engines
                    .iter()
                    .filter(|(_, r)| matches!(r.get("category").and_then(Value::as_str), Some("malicious" | "suspicious")))
                    .map(|(engine, r)| format!("{engine}: {}", r.get("result").and_then(Value::as_str).unwrap_or("flagged")))
                    .take(8)
                    .collect()
            })
            .unwrap_or_default();
        let mut categories: Vec<&str> = a
            .get("categories")
            .and_then(Value::as_object)
            .map(|c| c.values().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        categories.sort_unstable();
        categories.dedup();
        categories.truncate(6);
        let date = |k: &str| a.get(k).and_then(Value::as_i64).map(unix_date);

        Ok(compact(json!({
            "malicious": stat("malicious"),
            "suspicious": stat("suspicious"),
            "harmless": stat("harmless"),
            "undetected": stat("undetected"),
            "flagged_by": flagged,
            "reputation": a.get("reputation"),
            "community_votes": a.get("total_votes"),
            "tags": a.get("tags"),
            "categories": categories,
            "as_owner": a.get("as_owner"),
            "asn": a.get("asn"),
            "country": a.get("country"),
            "network": a.get("network"),
            "registrar": a.get("registrar"),
            "created": date("creation_date"),
            "last_analysis": date("last_analysis_date"),
        })))
    }

    // ---------- AlienVault OTX ----------

    async fn otx(&self, kind: &Kind) -> Option<anyhow::Result<Value>> {
        let key = self.cfg.otx_api_key.as_deref()?;
        Some(self.otx_check(key, kind).await)
    }

    async fn otx_check(&self, key: &str, kind: &Kind) -> anyhow::Result<Value> {
        let (section, value) = match kind {
            Kind::Ip(IpAddr::V4(ip)) => ("IPv4", ip.to_string()),
            Kind::Ip(IpAddr::V6(ip)) => ("IPv6", ip.to_string()),
            Kind::Domain(d) if d.matches('.').count() == 1 => ("domain", d.clone()),
            Kind::Domain(d) => ("hostname", d.clone()),
        };
        let req = self
            .client
            .get(format!("https://otx.alienvault.com/api/v1/indicators/{section}/{value}/general"))
            .header("X-OTX-API-KEY", key);
        let Some(v) = self.get_json(req).await? else {
            return Ok(json!({ "found": false }));
        };
        let names = |p: &Value, field: &str, key: &str| -> Vec<String> {
            p.get(field)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| x.get(key).and_then(Value::as_str).map(str::to_string)).take(5).collect())
                .unwrap_or_default()
        };
        let pulses: Vec<Value> = v
            .pointer("/pulse_info/pulses")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .take(5)
                    .map(|p| {
                        let tags: Vec<&str> = p
                            .get("tags")
                            .and_then(Value::as_array)
                            .map(|t| t.iter().filter_map(Value::as_str).take(5).collect())
                            .unwrap_or_default();
                        compact(json!({
                            "name": p.get("name"),
                            "created": p.get("created").and_then(Value::as_str).map(|s| s.chars().take(10).collect::<String>()),
                            "adversary": p.get("adversary").filter(|a| a.as_str().is_some_and(|s| !s.is_empty())),
                            "malware_families": names(p, "malware_families", "display_name"),
                            "attack_techniques": names(p, "attack_ids", "display_name"),
                            "tags": tags,
                        }))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // OTX's own false-positive hints, e.g. "Whitelisted: known DNS server".
        let validation: Vec<String> = v
            .get("validation")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.get("message").or_else(|| x.get("name")).and_then(Value::as_str).map(str::to_string))
                    .take(3)
                    .collect()
            })
            .unwrap_or_default();
        Ok(compact(json!({
            "pulse_count": v.pointer("/pulse_info/count").and_then(Value::as_u64).unwrap_or(0),
            "pulses": pulses,
            "known_benign": validation,
        })))
    }
}

/// Heuristic verdict and the reasons behind it, from whatever sources answered.
fn verdict(sources: &Map<String, Value>) -> (&'static str, Vec<String>) {
    let mut level: usize = 0; // 0 clean, 1 suspicious, 2 malicious
    let mut answered = false;
    let mut reasons = Vec::new();
    let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);

    if let Some(a) = sources.get("abuseipdb") {
        answered = true;
        let score = num(a, "confidence_score");
        let reports = num(a, "total_reports");
        if score >= 75 {
            level = 2;
        } else if score >= 25 {
            level = level.max(1);
        }
        if reports > 0 {
            reasons.push(format!("AbuseIPDB: confidence {score}%, {reports} report(s) in 90 days"));
        }
        if a.get("whitelisted").is_some() {
            reasons.push("AbuseIPDB: whitelisted".into());
        }
    }
    if let Some(vt) = sources.get("virustotal").filter(|v| v.get("found") != Some(&json!(false))) {
        answered = true;
        let malicious = num(vt, "malicious");
        let suspicious = num(vt, "suspicious");
        if malicious >= 5 {
            level = 2;
        } else if malicious >= 1 || suspicious >= 2 {
            level = level.max(1);
        }
        if malicious + suspicious > 0 {
            reasons.push(format!("VirusTotal: {malicious} malicious, {suspicious} suspicious detection(s)"));
        }
    }
    if let Some(otx) = sources.get("otx") {
        answered = true;
        let pulses = num(otx, "pulse_count");
        let benign = otx.get("known_benign").is_some();
        if pulses > 0 {
            if !benign {
                level = level.max(1);
            }
            reasons.push(format!("OTX: named in {pulses} threat report(s)"));
        }
        if benign {
            reasons.push("OTX: listed as known benign".into());
        }
    }
    if let Some(age) = sources.get("whois").and_then(|w| w.get("age_days")).and_then(Value::as_i64) {
        if age < 30 {
            level = level.max(1);
            reasons.push(format!("domain registered {age} day(s) ago"));
        }
    }
    let verdict = if !answered {
        reasons.push("no reputation source answered (keys in [intel])".into());
        "unknown"
    } else {
        ["clean", "suspicious", "malicious"][level]
    };
    (verdict, reasons)
}

/// Removes null, empty-string and empty-array fields, to keep results short.
fn compact(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(_, v)| !matches!(v, Value::Null) && v != &json!("") && v != &json!([]))
                .collect(),
        ),
        other => other,
    }
}

/// Accepts "1.2.3.4", "1.2.3.4:443", "https://example.com/path", defanged "example[.]com".
fn normalize(raw: &str) -> Option<(String, Kind)> {
    let mut s = raw
        .trim()
        .trim_matches(|c: char| "\"'`<>()[],;".contains(c))
        .replace("[.]", ".")
        .replace("(.)", ".");
    for scheme in ["https://", "http://", "hxxps://", "hxxp://"] {
        if s.to_ascii_lowercase().starts_with(scheme) {
            s = s[scheme.len()..].to_string();
        }
    }
    let s = s.split('/').next().unwrap_or("").trim_end_matches('.').to_string();
    if let Ok(ip) = s.parse::<IpAddr>() {
        return Some((ip.to_string(), Kind::Ip(ip)));
    }
    if let Ok(sock) = s.parse::<std::net::SocketAddr>() {
        return Some((sock.ip().to_string(), Kind::Ip(sock.ip())));
    }
    let host = match s.rsplit_once(':') {
        Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => s.as_str(),
    }
    .to_ascii_lowercase();
    valid_domain(&host).then(|| (host.clone(), Kind::Domain(host)))
}

fn valid_domain(d: &str) -> bool {
    if d.len() > 253 || !d.contains('.') {
        return false;
    }
    let labels: Vec<&str> = d.split('.').collect();
    let tld = labels[labels.len() - 1];
    labels.iter().all(|l| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    }) && (tld.starts_with("xn--") || (tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())))
}

/// Anything that isn't a globally routable unicast address.
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || o[0] == 0
                || o[0] >= 240
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10, carrier-grade NAT
                || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18.0.0/15, benchmarking
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_ip(&IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link local
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
        }
    }
}

/// Recursively finds the first RDAP entity with `role`.
fn find_entity<'a>(v: &'a Value, role: &str) -> Option<&'a Value> {
    for e in v.get("entities")?.as_array()? {
        let has_role = e
            .get("roles")
            .and_then(Value::as_array)
            .is_some_and(|r| r.iter().any(|x| x.as_str() == Some(role)));
        if has_role {
            return Some(e);
        }
        if let Some(found) = find_entity(e, role) {
            return Some(found);
        }
    }
    None
}

/// A field of an RDAP entity's vCard: ["vcard", [["fn", {}, "text", "Name"], ...]].
fn vcard(entity: &Value, field: &str) -> Option<String> {
    entity
        .pointer("/vcardArray/1")?
        .as_array()?
        .iter()
        .find(|p| p.get(0).and_then(Value::as_str) == Some(field))?
        .get(3)?
        .as_str()
        .map(str::to_string)
}

/// Unix seconds -> "YYYY-MM-DD".
fn unix_date(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days from "YYYY-MM-DD..." to today.
fn days_since(date: &str) -> Option<i64> {
    let mut parts = date.get(..10)?.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    let today = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64 / 86_400;
    Some(today - days_from_civil(y, m, d))
}

/// (year, month, day) -> days since 1970-01-01. Howard Hinnant's algorithm, the inverse of
/// `civil_from_days`.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// AbuseIPDB's report categories (https://www.abuseipdb.com/categories).
fn abuse_category(id: u64) -> String {
    let name = match id {
        1 => "DNS Compromise",
        2 => "DNS Poisoning",
        3 => "Fraud Orders",
        4 => "DDoS Attack",
        5 => "FTP Brute-Force",
        6 => "Ping of Death",
        7 => "Phishing",
        8 => "Fraud VoIP",
        9 => "Open Proxy",
        10 => "Web Spam",
        11 => "Email Spam",
        12 => "Blog Spam",
        13 => "VPN IP",
        14 => "Port Scan",
        15 => "Hacking",
        16 => "SQL Injection",
        17 => "Spoofing",
        18 => "Brute-Force",
        19 => "Bad Web Bot",
        20 => "Exploited Host",
        21 => "Web App Attack",
        22 => "SSH",
        23 => "IoT Targeted",
        _ => return format!("category {id}"),
    };
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        assert!(matches!(normalize("8.8.8.8:53"), Some((s, Kind::Ip(_))) if s == "8.8.8.8"));
        assert!(matches!(normalize("hxxps://Evil[.]example.com/x"), Some((s, Kind::Domain(_))) if s == "evil.example.com"));
        assert!(normalize("config.toml1").is_none());
        assert!(normalize("hello").is_none());
    }

    #[test]
    fn private_ranges() {
        for ip in ["10.1.2.3", "172.16.0.1", "192.168.1.1", "100.64.0.1", "127.0.0.1", "fd00::1", "fe80::1", "::ffff:10.0.0.1"] {
            assert!(is_private_ip(&ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700::1111"] {
            assert!(!is_private_ip(&ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 9, 29), 20_725);
        assert_eq!(unix_date(0), "1970-01-01");
    }
}
