#!/usr/bin/env bash
# Emit a "Proof" markdown section for a PR body, from the Testigo packet that
# witnessed this branch's work.
#
# Detection: commits on the current branch (vs the base, default `main`) carry
# `Testigo-Case:` trailers when they were made from agent-console with the
# ledger active. The matching packet must have been exported first (Proof tab
# → case → review → sign & export) into <repo>/proofpacks/.
#
# Usage: scripts/pr-proof.sh [base-branch]
#   Prints the markdown section to stdout; exits 0 with no output when the
#   branch carries no case trailer (proof is opt-in by construction: no
#   witnessed work, no section). Exits 2 when a trailer exists but no packet
#   was exported — so CI/skills can remind instead of silently shipping less.
set -euo pipefail

base="${1:-main}"
repo_root="$(git rev-parse --show-toplevel)"

case_id="$(git log "$base"..HEAD --format=%B | sed -n 's/^Testigo-Case: //p' | sort -u | head -1)"
if [ -z "$case_id" ]; then
  exit 0
fi

stem="$(printf '%s' "$case_id" | tr ':/\\' '---')"
packet="$repo_root/proofpacks/$stem.proofpack.json"
if [ ! -f "$packet" ]; then
  echo "Testigo-Case trailer found ($case_id) but no packet at proofpacks/$stem.proofpack.json." >&2
  echo "Export it first: Proof tab → $case_id → review → sign & export." >&2
  exit 2
fi

python3 - "$packet" "$case_id" <<'PY'
import json, base64, hashlib, sys

path, case_id = sys.argv[1], sys.argv[2]
pk = json.load(open(path))
st = json.loads(base64.b64decode(pk["envelope"]["payload"]))
pred = st["predicate"]
events = pred["events"]
full = [json.loads(e["line"]) for e in events if "line" in e]
kinds = {}
for v in full:
    kinds[v["kind"]] = kinds.get(v["kind"], 0) + 1
keyid = pk["envelope"]["signatures"][0]["keyid"]

print("## Proof (Testigo)")
print()
print(f"This branch's work was witnessed by [Testigo](https://github.com/cyl-castillo/testigo).")
print()
print("| | |")
print("|---|---|")
print(f"| case | `{case_id}` |")
print(f"| events | {len(events)} ({', '.join(f'{k} ×{n}' for k, n in sorted(kinds.items()))}) |")
print(f"| redactions | {pred.get('redactionCount', 0)} |")
print(f"| subject digest | `{st['subject'][0]['digest']['sha256']}` |")
print(f"| signer key id | `{keyid}` |")
print(f"| generator | `{pred.get('generator', '?')}` |")
print()
print("<details><summary>proof packet (drop into the <a href=\"https://github.com/cyl-castillo/testigo/blob/main/verifier/testigo-verifier.html\">standalone verifier</a> — save as .json)</summary>")
print()
print("```json")
print(json.dumps(pk, indent=1, ensure_ascii=False))
print("```")
print("</details>")
PY
