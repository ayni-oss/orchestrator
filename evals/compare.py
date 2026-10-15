#!/usr/bin/env python3
"""Compare retained evaluation reports without promoting fixture scores to model quality."""
import argparse
import json
from pathlib import Path


def cases(report):
    rows = {(f'fixture:{c["id"]}'): c for c in report["deterministic"]}
    for mode in ["scripted", "provider"]:
        for case in report.get(mode, {}).get("cases", []):
            rows[f'{mode}:{case["scenario"]["id"]}'] = case
    return rows


def measurements(case):
    if case is None:
        return None
    request = case.get("request", {})
    return {"status": case["status"], "latency_ms": case["latency_ms"],
            "model": request.get("model"),
            "profile_revision": (request.get("profile") or {}).get("revision"),
            "usage": (case.get("result") or {}).get("token_usage"),
            "input": case.get("scenario", {}).get("input")}


def compare(before, after):
    old, new = cases(before), cases(after)
    return {"before_revision": before["revision"], "after_revision": after["revision"],
            "changed_sources": sorted(k for k in before["inputs"].keys() | after["inputs"].keys()
                                      if before["inputs"].get(k) != after["inputs"].get(k)),
            "cases": [{"id": key, "before": measurements(old.get(key)),
                       "after": measurements(new.get(key))} for key in sorted(old.keys() | new.keys())]}


def load(path):
    report = json.loads(path.read_text())
    scripted = path.with_name("scripted.json")
    if scripted.is_file():
        report["scripted"] = json.loads(scripted.read_text())
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    args = parser.parse_args()
    print(json.dumps(compare(load(args.before), load(args.after)), indent=2))


if __name__ == "__main__":
    main()
