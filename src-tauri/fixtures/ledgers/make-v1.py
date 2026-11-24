#!/usr/bin/env python3
"""Writes v1.jsonl: a ledger in line format 1, the one every build up to
v0.98 wrote (no schemaVersion member).

This is an independent implementation of the line format on purpose: it does
not call the app. If the app ever stops agreeing with these bytes, the tests
that read the committed file fail. The content is made up; nothing here was
copied from an install.

    python3 make-v1.py > v1.jsonl
"""
import hashlib
import json
import sys

ORDER = ["seq", "ts", "caseId", "turnId", "kind", "termId", "sessionId", "actor", "payload"]


def line(seq, prev, **f):
    rec = {"seq": seq, **f}
    out = {k: rec[k] for k in ORDER if rec.get(k) is not None}
    out["prevHash"] = prev
    out["hash"] = ""
    # serde_json: compact, non-ASCII kept as is, map keys of the payload sorted.
    out["payload"] = json.loads(json.dumps(out["payload"], sort_keys=True))
    body = json.dumps(out, separators=(",", ":"), ensure_ascii=False)
    digest = hashlib.sha256(body.encode("utf-8")).hexdigest()
    return body[: -len('""}')] + '"' + digest + '"}', digest


CASE, TERM, SESS, TURN = "jira:DEMO-7", "t1", "s-0001", "t1:1"
EVENTS = [
    dict(ts=1000, caseId=CASE, kind="case_link", termId=TERM, actor="system",
         payload={"caseId": CASE}),
    dict(ts=2000, caseId=CASE, turnId=TURN, kind="prompt", termId=TERM, sessionId=SESS,
         actor="human", payload={"cwd": None, "prompt": "añadí el test que falta ✅", "skill": None}),
    dict(ts=3000, caseId=CASE, turnId=TURN, kind="approval_request", termId=TERM,
         sessionId=SESS, actor="agent",
         payload={"id": "a1", "tool": "Bash", "input": {"command": "npm test"}}),
    dict(ts=3500, caseId=CASE, turnId=TURN, kind="approval_decision", termId=TERM,
         sessionId=SESS, actor="human", payload={"id": "a1", "decision": "allow"}),
    dict(ts=4000, caseId=CASE, turnId=TURN, kind="tool_result", termId=TERM, sessionId=SESS,
         actor="agent", payload={"tool": "Bash", "command": "npm test", "exitCode": 0}),
    dict(ts=4500, caseId="term:t2", kind="prompt", termId="t2", actor="human",
         payload={"cwd": None, "prompt": "otra terminal", "skill": None}),
    dict(ts=5000, caseId=CASE, turnId=TURN, kind="turn_end", termId=TERM, sessionId=SESS,
         actor="agent", payload={"summary": "tests pass"}),
]

prev = "genesis"
for i, ev in enumerate(EVENTS):
    text, prev = line(i, prev, **ev)
    sys.stdout.write(text + "\n")
