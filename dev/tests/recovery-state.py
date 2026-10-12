"""Stopped synthetic fixture state: invariant checks and explicitly scoped fault setup."""
import json
import sqlite3
import sys

before = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
after = sqlite3.connect(sys.argv[2])
phase = sys.argv[3]
# No delivery can create another action, execution or accepted projection.
for table in ("runtime_profiles", "resource_events", "verification_journal", "runtime_artifacts", "durable_actions"):
    assert before.execute(f"SELECT * FROM {table}").fetchall() == after.execute(f"SELECT * FROM {table}").fetchall(), table
assert before.execute("SELECT source,event_id,payload FROM verification_outbox").fetchall() == after.execute("SELECT source,event_id,payload FROM verification_outbox").fetchall()
assert after.execute("SELECT stored_at IS NOT NULL,broker_sequence FROM verification_outbox").fetchone() == (1, 2)
name, value = after.execute("SELECT name,value FROM reconciliation_cursors WHERE name LIKE 'managed:%'").fetchone()
cursor = json.loads(value)
original = json.loads(before.execute("SELECT value FROM reconciliation_cursors WHERE name=?", (name,)).fetchone()[0])
expected = original.copy()
expected['next_at'] = 0 if phase == 'lost' else 9876543210
assert cursor == expected, (phase, cursor)
if phase == 'lost':
    assert after.execute("SELECT outcome,COUNT(*) FROM verification_consumer GROUP BY outcome ORDER BY outcome").fetchall() == [('rejected', 1), ('woken', 1)]
    # Simulate later reconciler backoff, after the consumer's committed cleanup wake.
    after.execute("UPDATE reconciliation_cursors SET value=json_set(value,'$.next_at',9876543210) WHERE name=?", (name,))
elif phase == 'conflicted':
    assert after.execute("SELECT outcome,COUNT(*) FROM verification_consumer GROUP BY outcome ORDER BY outcome").fetchall() == [('rejected', 2), ('woken', 1)]
elif phase == 'fill':
    after.execute("""WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<99997)
        INSERT INTO verification_consumer(delivery_key,digest,outcome,processed_at)
        SELECT printf('%064d',i),zeroblob(32),'rejected',unixepoch() FROM n""")
elif phase == 'release':
    # Only synthetic zero-digest diagnostics created above are removed.
    after.execute("DELETE FROM verification_consumer WHERE digest=zeroblob(32) AND journal_id IS NULL")
elif phase == 'blocked':
    assert after.execute("SELECT COUNT(*) FROM verification_consumer").fetchone() == (100000,)
elif phase == 'drained':
    assert after.execute("SELECT outcome,COUNT(*) FROM verification_consumer GROUP BY outcome ORDER BY outcome").fetchall() == [('rejected', 12), ('woken', 1)]
else:
    assert after.execute("SELECT COUNT(*) FROM verification_consumer").fetchone() == (2,)
after.commit()
print(f'Accepted state, action count, execution identity and failure budgets preserved: {phase}')
