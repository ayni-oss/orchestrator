#!/usr/bin/env python3
"""Inventory a read-only SQLite snapshot; this never authorizes evidence deletion."""
import argparse
import json
import sqlite3
from pathlib import Path

BOUND = 10000
MAX_JSON_BYTES = 4 * 1024 * 1024


class DrainError(Exception):
    """Incomplete inventory: operators must not infer zero dependencies."""


def bounded(connection, query):
    rows = connection.execute(query + f" LIMIT {BOUND + 1}").fetchall()
    if len(rows) > BOUND:
        raise DrainError("drain report bound exceeded; do not infer zero dependencies")
    return rows


def object_value(raw):
    if not isinstance(raw, str) or len(raw.encode()) > MAX_JSON_BYTES:
        raise DrainError("unsupported cursor size or encoding")
    try:
        value = json.loads(raw, object_pairs_hook=unique_keys,
                           parse_constant=lambda _: reject_json_constant())
    except (ValueError, RecursionError) as error:
        raise DrainError("malformed retained JSON; do not infer zero dependencies") from error
    if not isinstance(value, dict):
        raise DrainError("retained JSON must be an object")
    return value


def unique_keys(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise DrainError("duplicate retained JSON key")
        value[key] = item
    return value


def reject_json_constant():
    raise DrainError("non-finite retained JSON value")


def optional_string(value, key):
    item = value.get(key)
    if item is not None and (not isinstance(item, str) or not item or len(item) > 1024):
        raise DrainError("unsupported retained identifier")
    return item


def reporting_contract(configuration):
    if configuration is None or any(optional_string(configuration, key) is None
                                    for key in ("endpoint", "source")):
        raise DrainError("incomplete reporting contract")


def optional_object(value, key):
    item = value.get(key)
    if item is not None and not isinstance(item, dict):
        raise DrainError("unsupported retained object; do not infer zero dependencies")
    return item


def receipts(connection, table):
    # Fixed table names only; no payloads, credentials or diagnostic bytes are printed.
    return dict(connection.execute(f"SELECT decision,COUNT(*) FROM {table} GROUP BY decision"))


def report(connection):
    version_row = connection.execute(
        "SELECT version FROM clean_schema WHERE component='control_plane'"
    ).fetchone()
    if version_row is None or version_row[0] not in range(13, 19):
        raise DrainError("unsupported database schema; use a matching drain tool")
    version = version_row[0]
    generations = []
    legacy_failure_counters = 0
    runtime_rows = bounded(connection, "SELECT name,value FROM reconciliation_cursors "
                           "WHERE name LIKE 'managed:%' OR name LIKE 'repository-removal:%' ORDER BY name")
    for name, raw in runtime_rows:
        cursor = object_value(raw)
        preparation = optional_object(cursor, "preparation")
        pending = optional_object(cursor, "pending")
        trust = optional_object(cursor, "trust")
        builder = optional_object(cursor, "builder") or {}
        status = optional_object(cursor, "status") or {}
        accounting = optional_object(cursor, "accounting")
        legacy_failures = (accounting or {}).get("legacy_failures", 0) if accounting is not None else cursor.get("failures", 0)
        if type(legacy_failures) is not int or legacy_failures < 0:
            raise DrainError("unsupported retained failure counter")
        legacy_failure_counters += legacy_failures > 0
        builder_pending = builder.get("job") is not None or status.get("builder-job") is not None
        if preparation is None and pending is None and trust is None and not builder_pending:
            continue
        authority = optional_object(preparation or {}, "verification_authority") or {"mode": "retained"}
        mode = authority.get("mode")
        if mode not in ("retained", "shadow", "events"):
            raise DrainError("unsupported frozen authority; do not infer zero dependencies")
        operation_reporting = optional_object(preparation or {}, "operation_reporting")
        if operation_reporting is not None:
            reporting_contract(operation_reporting)
        if mode != "retained":
            reporting_contract(optional_object(authority, "reporting"))
        phase = (pending or {}).get("phase")
        if phase is not None and phase not in (
            "discovering", "discovering_started", "building", "building_started", "publishing",
            "publication_readback", "publication_readback_started", "verifying", "activating",
        ):
            raise DrainError("unsupported execution phase")
        failed = (pending or {}).get("failed", False)
        if not isinstance(failed, bool):
            raise DrainError("unsupported execution failure flag")
        # Omit frozen requests, registry/certificate refs and trust diagnostics.
        generations.append({
            "profile": name.split(":", 1)[1], "cursor_kind": name.split(":", 1)[0],
            "generation": optional_string(preparation or {}, "generation"), "authority": mode,
            "operation_authority": "events" if operation_reporting is not None else "legacy",
            "execution": optional_string(pending or {}, "name"), "phase": phase, "failed": failed,
            "trust_continuation": trust is not None, "builder_continuation": builder_pending,
            "retained_verification": (pending or {}).get("verification") is not None,
            "legacy_phase_reader": pending is not None and phase is None,
            "legacy_builder_reader": "builder" not in cursor and builder_pending,
        })

    # Attempt payloads are Protobuf: retain their readers and conservatively count
    # publication recovery from indexed state rather than reinterpret frozen bytes.
    attempts = bounded(connection, "SELECT a.id,a.state,t.active_attempt_id,"
                       "EXISTS(SELECT 1 FROM reconciliation_cursors c WHERE c.name='coding-done:'||a.id) "
                       "FROM development_attempts a JOIN development_tasks t ON t.id=a.development_task_id ORDER BY a.id")
    if connection.execute("SELECT COUNT(*) FROM development_attempts").fetchone()[0] != len(attempts):
        raise DrainError("orphaned or unbounded attempts; do not infer zero dependencies")
    admissions = {}
    if version >= 17:
        for attempt, raw, stopped, terminal in bounded(connection,
                "SELECT attempt_id,admission,stopped,terminal FROM coding_executions ORDER BY attempt_id"):
            if type(stopped) is not int or type(terminal) is not int or stopped not in (0, 1) or terminal not in (0, 1):
                raise DrainError("unsupported coding execution flags")
            if raw is not None:
                admission = object_value(raw)
                reporting_contract(optional_object(admission, "configuration"))
                binding = optional_object(admission, "binding")
                if binding is None:
                    raise DrainError("incomplete coding reporting admission")
                # pbjson writes camelCase and also accepts snake_case aliases.
                for camel, snake in (("taskId", "task_id"), ("attemptId", "attempt_id"),
                                     ("generation", "generation"), ("executionId", "execution_id"),
                                     ("snapshotDigest", "snapshot_digest")):
                    key = camel if camel in binding else snake
                    if optional_string(binding, key) is None:
                        raise DrainError("incomplete coding execution binding")
                    if camel == "attemptId" and binding[key] != attempt:
                        raise DrainError("coding admission identity mismatch")
            admissions[attempt] = "events" if raw is not None else "legacy"
    known_attempts = {row[0] for row in attempts}
    if set(admissions) - known_attempts:
        raise DrainError("orphaned coding executions; do not infer zero dependencies")
    pending_coding = set()
    if version >= 17:
        pending_coding = {row[0] for row in bounded(connection,
            "SELECT DISTINCT attempt_id FROM coding_inbox WHERE decision='pending' ORDER BY attempt_id")}
    if pending_coding - known_attempts:
        raise DrainError("orphaned coding receipts; do not infer zero dependencies")
    coding = []
    for attempt, state, active, complete in attempts:
        if type(state) is not int or state not in range(1, 9):
            raise DrainError("unsupported coding attempt state")
        # Cleanup markers do not rule out a future publication-only retry.
        coding.append({"attempt": attempt, "authority": admissions.get(attempt, "legacy"),
                       "cleanup_complete": bool(complete),
                       "reachable": not complete or state in (1, 2, 3) or attempt in pending_coding,
                       "publication_recovery_candidate": attempt == active and state in (4, 5),
                       "pending_receipt": attempt in pending_coding})

    history_tables = ("runtime_profile_revisions", "runtime_artifacts", "development_attempts",
                      "resource_events", "page_snapshots", "durable_actions", "verification_journal",
                      "verification_outbox", "reconciliation_cursors")
    history = {table: connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
               for table in history_tables}
    if version >= 18:
        history.update({table: connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
                        for table in ("lifecycle_journal", "lifecycle_outbox")})
    return {
        "schema": version, "reachable_generations": generations,
        "legacy_reader_dependencies": sum(g["authority"] != "events" for g in generations),
        "runtime_operation_legacy_dependencies": sum(g["operation_authority"] != "events" for g in generations),
        "trust_continuations": sum(g["trust_continuation"] for g in generations),
        "legacy_failure_counter_cursors": legacy_failure_counters,
        "coding_attempts": coding,
        "coding_legacy_dependencies": sum(c["authority"] == "legacy" and c["reachable"] for c in coding),
        "coding_legacy_publication_recovery_candidates": sum(c["authority"] == "legacy" and c["publication_recovery_candidate"] for c in coding),
        "coding_legacy_historical_reader_dependencies": sum(c["authority"] == "legacy" for c in coding),
        "receipts": receipts(connection, "verification_inbox"),
        "operation_receipts": receipts(connection, "runtime_operation_inbox") if version >= 16 else {},
        "coding_receipts": receipts(connection, "coding_inbox") if version >= 17 else {},
        "retained_history_rows": history,
        "historical_reader_and_evidence_audit_required": True,
        "compatibility_reader_removal_safe": False,
        "limitations": [
            "SQLite snapshot only: Kubernetes, retained PVC files and provider side effects require separate inspection.",
            "Publication recovery candidates are conservative: frozen Protobuf result, commit, deadlines and provider readback require the versioned application reader.",
            "Counts do not validate historical payloads, evidence availability or reporting configuration; zero counts never authorize retained-data deletion.",
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("database", type=Path, help="operator-owned database or quiesced backup")
    args = parser.parse_args()
    try:
        with sqlite3.connect(args.database.resolve().as_uri() + "?mode=ro", uri=True) as connection:
            connection.execute("PRAGMA query_only=ON")
            connection.execute("BEGIN")
            result = report(connection)
    except (sqlite3.Error, DrainError, TypeError, UnicodeError) as error:
        # Never include retained JSON or SQL names in errors.
        raise SystemExit("drain inventory incomplete; do not infer zero dependencies: " +
                         (str(error) if isinstance(error, DrainError) else type(error).__name__)) from None
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
