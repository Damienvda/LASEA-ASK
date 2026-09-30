"""
title: LASEASK · Réputation
author: LASEA
version: 2.2.0
description: Button under each answer: reputation of the public IPs and domains it mentions (RDAP, AbuseIPDB, VirusTotal, OTX, through LASEASK's intel lookups and cache). Replaces the LASEASK console's hover cards. Private IPs and internal domains are never sent out.
"""

# Install: Admin Panel > Functions > + > paste > Save, switch it on and set it Global. The table
# is added folded under the answer and is not sent back to the AI with the next question.

import asyncio
import re
from typing import Optional

import aiohttp
from pydantic import BaseModel, Field

IPV4 = r"(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)"
DOMAIN = r"(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+(?:xn--[a-z0-9-]+|[a-z]{2,24})"
INDICATOR_RE = re.compile(rf"(?<![\w.:-])({IPV4}|{DOMAIN})(?![\w-]|\.[\w-])", re.IGNORECASE)
# File names and code that look like domains.
NOT_TLDS = set(
    "md js py sh rs ps cs db gz rb ts hs vb so xz bz toml json yaml yml txt log conf cfg ini exe dll "
    "pdf doc docx xls xlsx csv html htm zip png jpg jpeg gif svg eml msi bat cmd".split()
)
FOLDED_RE = re.compile(r"<details.*?</details>", re.IGNORECASE | re.DOTALL)
VERDICT = {
    "malicious": "🔴 malveillant",
    "suspicious": "🟠 suspect",
    "clean": "🟢 sain",
    "unknown": "⚪ inconnu",
    "private": "🔒 privé",
}


def indicators(text: str, limit: int) -> list[str]:
    found: list[str] = []
    for match in INDICATOR_RE.finditer(FOLDED_RE.sub(" ", text)):
        value = match.group(1).lower().rstrip(".")
        if not re.fullmatch(IPV4, value) and value.rsplit(".", 1)[-1] in NOT_TLDS:
            continue
        if value not in found:
            found.append(value)
        if len(found) >= limit:
            break
    return found


def cell(value) -> str:
    return str(value).replace("|", "\\|").replace("\n", " ") if value not in (None, "") else "—"


def row(data: dict) -> str:
    ind = data.get("indicator", "?")
    if data.get("error"):
        return f"| `{ind}` | — | {cell(data['error'])} | | | | | |"
    if data.get("scope") == "private":
        return f"| `{ind}` | {VERDICT['private']} | non interrogé (adresse privée / domaine interne) | | | | | |"
    w = data.get("whois") or {}
    vt = data.get("virustotal") or {}
    ab = data.get("abuseipdb") or {}
    otx = data.get("otx") or {}
    owner = w.get("organisation") or w.get("registrant") or vt.get("as_owner") or ab.get("isp") or w.get("name")
    country = w.get("country") or vt.get("country") or ab.get("country")
    abuse = f"{ab.get('confidence_score', 0)} % · {ab.get('total_reports', 0)} signal." if ab else None
    if vt:
        total = sum(vt.get(k, 0) or 0 for k in ("malicious", "suspicious", "harmless", "undetected"))
        vt_txt = "absent" if vt.get("found") is False else f"{vt.get('malicious', 0)} malv. / {vt.get('suspicious', 0)} susp. sur {total}"
    else:
        vt_txt = None
    otx_txt = ("absent" if otx.get("found") is False else f"{otx.get('pulse_count', 0)} pulse(s)") if otx else None
    why = "; ".join(data.get("reasons") or [])
    verdict = VERDICT.get(data.get("verdict", "unknown"), data.get("verdict"))
    return f"| `{ind}` | {verdict} | {cell(owner)} | {cell(country)} | {cell(abuse)} | {cell(vt_txt)} | {cell(otx_txt)} | {cell(why)} |"


class Action:
    class Valves(BaseModel):
        laseask_url: str = Field(
            default="http://laseask:8787",
            description="LASEASK backend, as seen from the Open WebUI container.",
        )
        max_indicators: int = Field(default=10, description="Indicators looked up per click.")

    def __init__(self):
        self.valves = self.Valves()

    async def _lookup(self, session: aiohttp.ClientSession, value: str) -> dict:
        try:
            async with session.get(f"{self.valves.laseask_url.rstrip('/')}/api/intel", params={"q": value}) as r:
                data = await r.json(content_type=None)
                if r.status != 200:
                    return {"indicator": value, "error": data.get("error", f"HTTP {r.status}")}
                return data
        except Exception as e:  # network error, timeout
            return {"indicator": value, "error": str(e)}

    async def action(
        self,
        body: dict,
        __user__: Optional[dict] = None,
        __event_emitter__=None,
        __event_call__=None,
    ) -> None:
        messages = body.get("messages") or []
        text = messages[-1].get("content", "") if messages else ""
        if not isinstance(text, str):
            text = ""
        wanted = indicators(text, self.valves.max_indicators)

        async def status(description: str, done: bool) -> None:
            if __event_emitter__:
                await __event_emitter__({"type": "status", "data": {"description": description, "done": done}})

        if not wanted:
            await status("Aucune IP ni domaine dans cette réponse", True)
            return

        await status(f"Réputation de {len(wanted)} indicateur(s)…", False)
        timeout = aiohttp.ClientTimeout(total=60)
        async with aiohttp.ClientSession(timeout=timeout) as session:
            results = await asyncio.gather(*(self._lookup(session, v) for v in wanted))

        flagged = sum(1 for d in results if d.get("verdict") in ("malicious", "suspicious"))
        lines = [
            "",
            "",
            "<details>",
            f"<summary>🛡 Réputation : {len(results)} indicateur(s), {flagged} suspect(s) ou malveillant(s)</summary>",
            "",
            "| Indicateur | Verdict | Propriétaire | Pays | AbuseIPDB | VirusTotal | OTX | Pourquoi |",
            "|---|---|---|---|---|---|---|---|",
            *(row(d) for d in results),
            "",
            "Sources : RDAP, AbuseIPDB, VirusTotal, OTX (via LASEASK). Les IP privées et domaines internes ne sont jamais envoyés.",
            "",
            "</details>",
        ]
        if __event_emitter__:
            await __event_emitter__({"type": "message", "data": {"content": "\n".join(lines)}})
        await status("Réputation ajoutée sous la réponse", True)
