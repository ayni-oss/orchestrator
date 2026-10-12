#!/usr/bin/env python3
"""Read a consistent, read-only SQLite snapshot; never delete compatibility evidence."""
import argparse
import json
import sqlite3
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("database", type=Path, help="operator-owned database or quiesced backup")
args = parser.parse_args()
connection = sqlite3.connect(args.database.resolve().as_uri() + "?mode=ro", uri=True)
connection.execute("BEGIN")
version = connection.execute("SELECT version FROM clean_schema WHERE component='control_plane'").fetchone()[0]
if version not in (13, 14):
    raise SystemExit("unsupported database schema; use a matching drain tool")
rows = connection.execute("SELECT name,value FROM reconciliation_cursors WHERE name LIKE 'managed:%' ORDER BY name LIMIT 10001").fetchall()
if len(rows) > 10000:
    raise SystemExit("drain report bound exceeded; do not infer zero dependencies")
generations = []
for name, raw in rows:
    cursor = json.loads(raw)
    preparation = cursor.get("preparation")
    pending = cursor.get("pending")
    if preparation is None and pending is None:
        continue
    authority = (preparation or {}).get("verification_authority", {"mode": "retained"})
    mode = authority["mode"]
    if mode not in ("retained", "shadow", "events"):
        raise SystemExit("unsupported frozen authority; do not infer zero dependencies")
    generations.append({"profile": name.removeprefix("managed:"), "generation": (preparation or {}).get("generation"), "authority": mode, "execution": (pending or {}).get("name"), "phase": (pending or {}).get("phase"), "failed": (pending or {}).get("failed", False)})
receipts = dict(connection.execute("SELECT decision,COUNT(*) FROM verification_inbox GROUP BY decision"))
print(json.dumps({"schema": version, "reachable_generations": generations, "legacy_reader_dependencies": sum(g["authority"] != "events" for g in generations), "receipts": receipts, "historical_reader_and_evidence_audit_required": True}, indent=2))
