#!/usr/bin/env python3
"""Compare stopped SQLite fixture state without altering retained databases."""
import sqlite3
import sys
from pathlib import Path


def connect(path):
    # VACUUM baselines and explicitly checkpointed stopped-state copies have no WAL.
    return sqlite3.connect(Path(path).resolve().as_uri() + '?mode=ro&immutable=1', uri=True)


baseline, observed = map(connect, sys.argv[1:3])
assert observed.execute("SELECT version FROM clean_schema WHERE component='control_plane'").fetchone() == (25,), 'plan schema upgrade missing'
for table in ('development_tasks', 'development_attempts', 'durable_actions'):
    expected = baseline.execute(f'SELECT * FROM {table} ORDER BY 1,2').fetchall()
    actual = observed.execute(f'SELECT * FROM {table} ORDER BY 1,2').fetchall()
    assert actual == expected, f'{table}: plan review/replay changed admitted work or receipts'
expected = baseline.execute('SELECT action_key,digest,conversation_id FROM project_conversation_actions ORDER BY action_key').fetchall()
actual = observed.execute('SELECT action_key,digest,conversation_id FROM project_conversation_actions ORDER BY action_key').fetchall()
for receipt in expected:
    assert receipt in actual, 'historical conversation receipt changed'
assert observed.execute('PRAGMA integrity_check').fetchone() == ('ok',), 'SQLite integrity check failed'
print('Schema 25, immutable admitted task/attempt rows and retained action receipts verified.')
