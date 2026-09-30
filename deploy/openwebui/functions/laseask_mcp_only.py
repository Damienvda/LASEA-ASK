"""
title: LASEASK · MCP uniquement
author: LASEA
version: 3.0.0
description: Button in the message bar: the AI must consult a tool before answering and base its answer on tool results, never on its own knowledge alone.
"""

# Install: Admin Panel > Functions > + > paste > Save, switch it on and set it Global. It appears
# as a button in the message bar and only acts while switched on.

from typing import Optional

from pydantic import BaseModel, Field

RULE = (
    "MCP-only mode is on: call at least one tool before answering, and base the answer on tool "
    "results, never on your own knowledge alone. If no tool can answer, say so."
)


class Filter:
    class Valves(BaseModel):
        # After "LASEASK · Investigation", so the rule comes last in the system prompt.
        priority: int = Field(default=10)

    def __init__(self):
        self.valves = self.Valves()
        self.toggle = True
        self.icon = (
            "data:image/svg+xml;utf8,"
            "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='currentColor' "
            "stroke-width='2' stroke-linecap='round' stroke-linejoin='round'><path d='M14.7 6.3a1 1 0 0 0 0 1.4l1.6 "
            "1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 "
            "7.94-7.94l-3.76 3.76z'/></svg>"
        )

    async def inlet(self, body: dict, __user__: Optional[dict] = None) -> dict:
        messages = body.setdefault("messages", [])
        first = messages[0] if messages else None
        if first and first.get("role") == "system" and isinstance(first.get("content"), str):
            first["content"] = f"{first['content']}\n\n{RULE}"
        else:
            messages.insert(0, {"role": "system", "content": RULE})
        return body
