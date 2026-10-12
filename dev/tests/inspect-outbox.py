"""Inspect only synthetic fixture databases, after their writer has stopped."""
import sqlite3
import sys

before = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
after = sqlite3.connect(f"file:{sys.argv[2]}?mode=ro", uri=True)
assert after.execute("SELECT version FROM clean_schema").fetchone() == (15,)
for table in ("verification_journal", "runtime_artifacts"):
    assert before.execute(f"SELECT * FROM {table}").fetchall() == after.execute(f"SELECT * FROM {table}").fetchall(), table
original = before.execute("SELECT source,event_id,payload FROM verification_outbox").fetchone()
assert after.execute("SELECT source,event_id,payload FROM verification_outbox").fetchone() == original
for connection in (before, after):
    assert connection.execute("SELECT active_revision_id FROM runtime_profiles").fetchall() == before.execute("SELECT active_revision_id FROM runtime_profiles").fetchall()
row = after.execute("SELECT attempts,stored_at,broker_sequence,last_error FROM verification_outbox").fetchone()
assert row[0] >= 2, row
if sys.argv[3] == "pending":
    assert row[1:] == (None, None, "broker_unavailable"), row
else:
    assert row[1] is not None and row[2:] == (2, ""), row
print(f"Accepted projection and immutable outbox preserved: {sys.argv[3]}")
