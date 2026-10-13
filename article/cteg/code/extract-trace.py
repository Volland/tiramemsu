#!/usr/bin/env python3
"""Turn a Claude Code session transcript into a sanitized CTEG trace.

Usage: python3 extract-trace.py SESSION.jsonl [SUBAGENT.jsonl ...] > trace.json

Keeps only structure: record ids, parent ids, timestamps, an event type, the
tool name and a short label. Prompts, outputs, file contents and paths are
dropped. The transcript's own `parentUuid` is the single causal parent.
"""
import json, sys
from datetime import datetime

def ms(ts):
    return int(datetime.fromisoformat(ts.replace("Z", "+00:00")).timestamp() * 1000)

def label_of(tool, inp):
    if tool == "Bash":
        return inp.get("description", "shell command")
    if tool in ("WebSearch",):
        return inp.get("query", "")[:70]
    if tool == "WebFetch":
        return inp.get("url", "")
    if tool == "Agent":
        return inp.get("description", "subagent")
    if tool in ("Read", "Write", "Edit"):
        return inp.get("file_path", "").split("/")[-1]
    if tool == "ToolSearch":
        return inp.get("query", "")
    return tool

def events(path, session, agent):
    out = []
    for line in open(path):
        d = json.loads(line)
        if "uuid" not in d or not d.get("timestamp"):
            continue
        kind, tool, label, extra = None, None, None, {}
        msg = d.get("message") or {}
        content = msg.get("content")
        if d["type"] == "attachment":
            a = d.get("attachment", {})
            if a.get("type") == "queued_command" and "Subagent hand-back" in json.dumps(a):
                kind, label = "SubagentReturn", "subagent report handed back"
            else:
                kind, label = "HarnessContext", a.get("type", "attachment")
        elif d["type"] == "user":
            blocks = content if isinstance(content, list) else [{"type": "text", "text": content}]
            if any(b.get("type") == "tool_result" for b in blocks):
                b = next(b for b in blocks if b.get("type") == "tool_result")
                kind, label = "ToolResult", "result"
                extra["toolUseId"] = b["tool_use_id"]
                extra["isError"] = bool(b.get("is_error"))
            elif d.get("parentUuid") is None:
                kind, label = "Prompt", "task prompt"
            elif "task-notification" in json.dumps(blocks):
                kind, label = "Notification", "background task finished"
            else:
                kind, label = "Prompt", "user message"
        elif d["type"] == "assistant":
            b = (content or [{}])[0]
            if b.get("type") == "thinking":
                kind, label = "Thinking", "reasoning"
            elif b.get("type") == "text":
                kind, label = "Message", b["text"][:60].split("\n")[0]
            elif b.get("type") == "tool_use":
                tool = b["name"]
                kind = "SubagentCall" if tool == "Agent" else "ToolCall"
                label = label_of(tool, b.get("input", {}))
                extra["toolUseId"] = b["id"]
        if kind is None:
            continue
        out.append(dict(id=d["uuid"], parent=d.get("parentUuid"), t=ms(d["timestamp"]),
                        type=kind, tool=tool, label=label, session=session, agent=agent, **extra))
    return out

main = events(sys.argv[1], "main", "main")
subs = []
for p in sys.argv[2:]:
    agent_id = p.split("agent-")[-1].split(".")[0]
    subs.append(dict(agent=agent_id[:8], events=events(p, "sub-" + agent_id[:8], "fact-checker")))
json.dump(dict(main=main, subagents=subs), sys.stdout, indent=1)
