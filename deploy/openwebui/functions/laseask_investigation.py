"""
title: LASEASK · Investigation
author: LASEA
version: 3.0.0
description: Adds LASEASK's investigation prompt to the chats (current date, the rules that make the AI investigate thoroughly and answer briefly from tool results only, and the guide for each tool server, e.g. the FortiAnalyzer field quirks). The prompt comes from the LASEASK tool service, so it always matches its config.toml.
"""

# Install: Admin Panel > Functions > + > paste > Save, switch it on and set it Global (or attach
# it to specific models in Workspace > Models > the model > Filters).

import time
from typing import Optional

import aiohttp
from pydantic import BaseModel, Field


class Filter:
    class Valves(BaseModel):
        priority: int = Field(default=0, description="Order among filters (lower runs first).")
        laseask_url: str = Field(
            default="http://laseask:8787",
            description="LASEASK tool service, as seen from the Open WebUI container.",
        )
        model_ids: str = Field(
            default="",
            description="Only these model ids, comma-separated. Empty = every model.",
        )
        only_with_tools: bool = Field(
            default=False,
            description="Only when the request lists enabled tools (tool_ids). Test it before relying on it: whether Open WebUI passes tool_ids to filters depends on its version.",
        )

    def __init__(self):
        self.valves = self.Valves()
        self._prompt: Optional[str] = None
        self._fetched_at = 0.0

    async def _get_prompt(self) -> Optional[str]:
        # Refreshed every 5 minutes: it carries the current date and time.
        if self._prompt is not None and time.monotonic() - self._fetched_at < 300:
            return self._prompt
        try:
            timeout = aiohttp.ClientTimeout(total=5)
            async with aiohttp.ClientSession(timeout=timeout) as session:
                async with session.get(f"{self.valves.laseask_url.rstrip('/')}/api/prompt") as r:
                    r.raise_for_status()
                    self._prompt = (await r.json()).get("prompt")
                    self._fetched_at = time.monotonic()
        except Exception:
            pass  # keep the last prompt we had, if any
        return self._prompt

    def _applies(self, body: dict) -> bool:
        wanted = [m.strip() for m in self.valves.model_ids.split(",") if m.strip()]
        if wanted and body.get("model") not in wanted:
            return False
        if self.valves.only_with_tools:
            tool_ids = body.get("tool_ids") or (body.get("metadata") or {}).get("tool_ids")
            if not tool_ids:
                return False
        return True

    async def inlet(self, body: dict, __user__: Optional[dict] = None, __event_emitter__=None) -> dict:
        if not self._applies(body):
            return body
        prompt = await self._get_prompt()
        if not prompt:
            if __event_emitter__:
                await __event_emitter__(
                    {"type": "status", "data": {"description": "LASEASK : prompt d'investigation indisponible", "done": True}}
                )
            return body
        messages = body.setdefault("messages", [])
        first = messages[0] if messages else None
        if first and first.get("role") == "system" and isinstance(first.get("content"), str):
            # Keep the model's own system prompt (Workspace > Models) and Open WebUI's context.
            first["content"] = f"{prompt}\n\n{first['content']}"
        else:
            messages.insert(0, {"role": "system", "content": prompt})
        return body
