#!/usr/bin/env python3
"""Inspect stopped synthetic SQLite fixtures without rewriting retained evidence."""
import json
import sqlite3
import sys
from pathlib import Path


def connect(path):
    # Exported baselines are VACUUM snapshots; observed copies are made only
    # after the stopped writer's WAL was explicitly checkpointed and drained.
    return sqlite3.connect(Path(path).resolve().as_uri() + '?mode=ro&immutable=1', uri=True)


def varint(payload, offset):
    result = shift = 0
    while True:
        value = payload[offset]
        offset += 1
        result |= (value & 127) << shift
        if value < 128:
            return result, offset
        shift += 7
        assert shift < 70, 'invalid protobuf varint'


def fields(payload, wanted):
    """Read exact protobuf field bytes for immutable embedded plan comparison."""
    offset = 0
    found = []
    while offset < len(payload):
        tag, offset = varint(payload, offset)
        wire = tag & 7
        if wire == 0:
            _, offset = varint(payload, offset)
        elif wire == 2:
            length, offset = varint(payload, offset)
            value = payload[offset:offset + length]
            offset += length
            if tag >> 3 == wanted:
                found.append(value)
        elif wire in (1, 5):
            offset += 8 if wire == 1 else 4
        else:
            raise AssertionError(f'unsupported protobuf wire type {wire}')
    return found


baseline, observed = map(connect, sys.argv[1:3])
assert observed.execute("SELECT version FROM clean_schema WHERE component='control_plane'").fetchone() == (25,), 'context schema migration missing'
for table in ('development_tasks', 'development_attempts', 'durable_actions', 'project_conversation_actions', 'project_agent_requests', 'project_outcome_summaries'):
    expected = baseline.execute(f'SELECT * FROM {table} ORDER BY 1,2').fetchall()
    actual = observed.execute(f'SELECT * FROM {table} ORDER BY 1,2').fetchall()
    assert actual == expected, f'{table}: context retrieval/replay changed frozen work or receipts'
for identity, payload in baseline.execute('SELECT id,payload FROM project_conversations'):
    actual = observed.execute('SELECT payload FROM project_conversations WHERE id=?', (identity,)).fetchone()[0]
    assert fields(payload, 10) == fields(actual, 10), 'retained plan content or decisions changed'
    assert len(fields(actual, 5)) <= 16, 'conversation projection is not bounded'
    for message in fields(payload, 5):
        message_id = fields(message, 1)[0].decode()
        content = fields(message, 3)[0].decode()
        archived = observed.execute('SELECT content FROM project_context_records WHERE conversation_id=? AND id=?', (identity, f'message:{message_id}')).fetchone()
        assert archived == (content,), 'original message lost during projection compaction'
        original = observed.execute('SELECT original_payload FROM project_context_records WHERE conversation_id=? AND id=?', (identity, f'message:{message_id}')).fetchone()
        assert original == (message,), 'original message timestamp, run identity or other fields changed'
    records = observed.execute('SELECT sequence,id,kind,content FROM project_context_records WHERE conversation_id=? ORDER BY sequence', (identity,)).fetchall()
    assert len(records) > 80 and len({r[1] for r in records}) == len(records)
    assert any(r[2] == 'worker_outcome' for r in records), 'safe accepted worker outcome missing'
    assert any(r[2] == 'approval' for r in records), 'approval context missing'
    assert any('amber migration boundary' in r[3] for r in records), 'old decision missing'
    safe_keys = ('provider_credentials', 'secret_name', 'secret_uid', 'credential_revision', 'environment_variables')
    for _, _, kind, content in records:
        if kind != 'message':
            assert not any(f'"{key}"' in content for key in safe_keys), 'credential metadata reached context'
    frozen = observed.execute("SELECT payload FROM project_agent_requests WHERE run_id='context-final-run'").fetchone()
    request = json.loads(frozen[0])
    assert any('amber migration boundary' in r['content'] for r in request['conversation_context']['records']), 'older context absent from frozen run'
if baseline.execute("SELECT 1 FROM sqlite_master WHERE name='project_context_records'").fetchone():
    for table in ('project_context_records', 'project_context_summaries'):
        assert baseline.execute(f'SELECT * FROM {table} ORDER BY 1,2').fetchall() == observed.execute(f'SELECT * FROM {table} ORDER BY 1,2').fetchall(), f'{table}: read/replay/restart modified original context'
assert observed.execute('PRAGMA integrity_check').fetchone() == ('ok',)
print('Schema 25, original history, safe outcomes, frozen requests, immutable plans/tasks and exact action receipts verified.')
