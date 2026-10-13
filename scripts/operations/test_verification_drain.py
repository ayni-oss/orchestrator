"""Persisted drain and publication-recovery compatibility guarantees."""
import importlib.util
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('verification-drain.py')
spec = importlib.util.spec_from_file_location('drain', SCRIPT)
drain = importlib.util.module_from_spec(spec)
spec.loader.exec_module(drain)
CONFIG = {'endpoint': 'http://control-plane/events/verification', 'source': 'ayni://orchestrator/supervisors/test'}
BINDING = {'taskId': 'task-attempt', 'attemptId': 'attempt', 'generation': 'gen', 'executionId': 'job', 'snapshotDigest': 'a' * 64}


def fixture(version=18):
    db = sqlite3.connect(':memory:')
    db.executescript('''
        CREATE TABLE clean_schema(component TEXT,version INTEGER);
        CREATE TABLE reconciliation_cursors(name TEXT PRIMARY KEY,value TEXT);
        CREATE TABLE development_tasks(id TEXT,active_attempt_id TEXT);
        CREATE TABLE development_attempts(id TEXT,development_task_id TEXT,state INTEGER);
        CREATE TABLE coding_executions(attempt_id TEXT,admission TEXT,stopped INTEGER,terminal INTEGER);
        CREATE TABLE coding_inbox(attempt_id TEXT,decision TEXT);
        CREATE TABLE verification_inbox(decision TEXT);
        CREATE TABLE runtime_operation_inbox(decision TEXT);
    ''')
    db.execute("INSERT INTO clean_schema VALUES('control_plane',?)", (version,))
    for name in ('runtime_profile_revisions', 'runtime_artifacts', 'resource_events',
                 'page_snapshots', 'durable_actions', 'verification_journal',
                 'verification_outbox', 'lifecycle_journal', 'lifecycle_outbox'):
        db.execute(f'CREATE TABLE {name}(payload BLOB)')
    return db


def cursor(db, name, value):
    db.execute('INSERT INTO reconciliation_cursors VALUES(?,?)', (name, json.dumps(value)))


def attempt(db, name='attempt', state=4, complete=True, admission=None):
    db.execute('INSERT INTO development_tasks VALUES(?,?)', ('task-' + name, name))
    db.execute('INSERT INTO development_attempts VALUES(?,?,?)', (name, 'task-' + name, state))
    db.execute('INSERT INTO coding_executions VALUES(?,?,1,1)', (name, admission))
    if complete:
        cursor(db, 'coding-done:' + name, {})


class DrainTests(unittest.TestCase):
    def test_supported_schema_snapshots_never_authorize_removal(self):
        for version in range(13, 19):
            with self.subTest(version=version), fixture(version) as db:
                result = drain.report(db)
                self.assertEqual(result['schema'], version)
                self.assertFalse(result['compatibility_reader_removal_safe'])
                self.assertTrue(result['historical_reader_and_evidence_audit_required'])

    def test_trust_only_builder_and_removed_repository_are_reachable(self):
        with fixture() as db:
            cursor(db, 'managed:trust', {'trust': {'state': 'open', 'result': {'private': 'omitted'}}})
            cursor(db, 'managed:builder', {'status': {'builder-job': 'job', 'private': 'omitted'}})
            cursor(db, 'repository-removal:cleanup', {'pending': {'name': 'job', 'failed': True}})
            result = drain.report(db)
            self.assertEqual(result['legacy_reader_dependencies'], 3)
            self.assertEqual(result['runtime_operation_legacy_dependencies'], 3)
            self.assertEqual(result['trust_continuations'], 1)
            self.assertNotIn('private', json.dumps(result))
            self.assertNotIn('omitted', json.dumps(result))

    def test_shadow_and_event_authorities_remain_frozen(self):
        with fixture() as db:
            for mode in ('events', 'shadow', 'retained'):
                authority = {'mode': mode}
                if mode != 'retained':
                    authority['reporting'] = CONFIG
                cursor(db, 'managed:' + mode, {'preparation': {
                    'generation': mode, 'verification_authority': authority,
                    'operation_reporting': CONFIG}})
            result = drain.report(db)
            self.assertEqual(result['legacy_reader_dependencies'], 2)
            self.assertEqual(result['runtime_operation_legacy_dependencies'], 0)
            self.assertNotIn('omitted', json.dumps(result))

    def test_completed_legacy_attempt_keeps_publication_and_history_dependencies(self):
        with fixture() as db:
            attempt(db)
            result = drain.report(db)
            self.assertEqual(result['coding_legacy_dependencies'], 0)
            self.assertEqual(result['coding_legacy_publication_recovery_candidates'], 1)
            self.assertEqual(result['coding_legacy_historical_reader_dependencies'], 1)

    def test_legacy_failure_history_survives_without_active_generation(self):
        with fixture() as db:
            cursor(db, 'managed:historical', {'failures': 3})
            cursor(db, 'managed:migrated', {'accounting': {'legacy_failures': 2}, 'failures': 0})
            cursor(db, 'managed:precise', {'accounting': {'legacy_failures': 0}, 'failures': 4})
            result = drain.report(db)
            self.assertEqual(result['legacy_failure_counter_cursors'], 2)
            self.assertEqual(result['legacy_reader_dependencies'], 0)

    def test_pending_receipt_keeps_completed_event_attempt_reachable(self):
        with fixture() as db:
            attempt(db, admission=json.dumps({'configuration': CONFIG, 'binding': BINDING}))
            db.execute("INSERT INTO coding_inbox VALUES('attempt','pending')")
            result = drain.report(db)
            self.assertTrue(result['coding_attempts'][0]['reachable'])
            self.assertEqual(result['coding_legacy_dependencies'], 0)

    def test_missing_coding_execution_is_conservatively_legacy(self):
        with fixture() as db:
            attempt(db, state=2)
            db.execute('DELETE FROM coding_executions')
            self.assertEqual(drain.report(db)['coding_legacy_dependencies'], 1)

    def test_cancelled_attempt_has_history_but_no_publication_candidate(self):
        with fixture() as db:
            attempt(db, state=6)
            result = drain.report(db)
            self.assertEqual(result['coding_legacy_publication_recovery_candidates'], 0)
            self.assertEqual(result['coding_legacy_historical_reader_dependencies'], 1)

    def test_malformed_or_unknown_state_fails_closed(self):
        bad_cursors = ([], {'pending': []}, {'trust': 'invalid'},
                       {'preparation': {'verification_authority': {'mode': 'future'}}},
                       {'preparation': {'verification_authority': {'mode': 'events'}}},
                       {'preparation': {'verification_authority': {'mode': 'events', 'reporting': {}}}},
                       {'preparation': {'operation_reporting': {}}},
                       {'pending': {'phase': 'future'}}, {'pending': {'failed': 'false'}})
        for value in bad_cursors:
            with self.subTest(value=value), fixture() as db:
                cursor(db, 'managed:bad', value)
                with self.assertRaises(drain.DrainError):
                    drain.report(db)
        with fixture(19) as db, self.assertRaises(drain.DrainError):
            drain.report(db)
        with fixture() as db:
            attempt(db, state=99)
            with self.assertRaises(drain.DrainError):
                drain.report(db)

    def test_orphaned_coding_state_fails_closed(self):
        for table in ('coding_executions', 'coding_inbox', 'development_attempts'):
            with self.subTest(table=table), fixture() as db:
                if table == 'coding_executions':
                    db.execute("INSERT INTO coding_executions VALUES('absent',NULL,0,0)")
                elif table == 'coding_inbox':
                    db.execute("INSERT INTO coding_inbox VALUES('absent','pending')")
                else:
                    db.execute("INSERT INTO development_attempts VALUES('absent','missing-task',2)")
                with self.assertRaises(drain.DrainError):
                    drain.report(db)

    def test_duplicate_nonfinite_and_truncated_json_fail_closed(self):
        for raw in ('{"pending": {}, "pending": null}', '{"pending": NaN}', '{'):
            with self.subTest(raw=raw), fixture() as db:
                db.execute('INSERT INTO reconciliation_cursors VALUES(?,?)', ('managed:bad', raw))
                with self.assertRaises(drain.DrainError):
                    drain.report(db)

    def test_frozen_nested_identifiers_cannot_leak_in_report(self):
        with fixture() as db:
            cursor(db, 'managed:bad', {'preparation': {'generation': {'private': 'omitted'}}})
            with self.assertRaises(drain.DrainError):
                drain.report(db)

    def test_invalid_admission_cannot_establish_event_authority(self):
        for raw in ('[]', '{}', '{"configuration": [], "binding": {}}', json.dumps({'configuration': CONFIG, 'binding': {}})):
            with self.subTest(raw=raw), fixture() as db:
                attempt(db, admission=raw)
                with self.assertRaises(drain.DrainError):
                    drain.report(db)

    def test_mismatched_binding_and_invalid_execution_flags_fail_closed(self):
        with fixture() as db:
            attempt(db, admission=json.dumps({'configuration': CONFIG, 'binding': {**BINDING, 'attemptId': 'other'}}))
            with self.assertRaises(drain.DrainError):
                drain.report(db)
        with fixture() as db:
            attempt(db)
            db.execute('UPDATE coding_executions SET stopped=1.5')
            with self.assertRaises(drain.DrainError):
                drain.report(db)

    def test_bound_and_missing_tables_fail_closed(self):
        with fixture() as db:
            db.executemany('INSERT INTO reconciliation_cursors VALUES(?,?)',
                           [('managed:' + str(i), '{}') for i in range(drain.BOUND + 1)])
            with self.assertRaises(drain.DrainError):
                drain.report(db)
        with fixture() as db:
            db.execute('DROP TABLE development_attempts')
            with self.assertRaises(sqlite3.Error):
                drain.report(db)

    def test_cli_reads_without_changes_and_never_creates_missing_database(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'store.sqlite'
            with fixture() as db, sqlite3.connect(path) as dest:
                db.commit()
                db.backup(dest)
            before = path.read_bytes()
            result = subprocess.run([sys.executable, str(SCRIPT), str(path)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(before, path.read_bytes())
            self.assertFalse(json.loads(result.stdout)['compatibility_reader_removal_safe'])
            absent = Path(directory) / 'absent.sqlite'
            result = subprocess.run([sys.executable, str(SCRIPT), str(absent)], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, '')
            self.assertFalse(absent.exists())


if __name__ == '__main__':
    unittest.main()
