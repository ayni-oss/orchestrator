#!/usr/bin/env python3
"""Regenerate the workflow catalog fingerprint from repository SQL on pinned PostgreSQL.

Run with --write after changing migration SQL; the default prints the expected
fingerprint and fails when the source constant differs. Uses a disposable local
container with no host port, no mounted data and fixture-only trust authentication.
"""

import argparse
import hashlib
import pathlib
import re
import subprocess
import time
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[2]
SOURCE = ROOT / "backend/servers/control-plane/crates/workflow-postgres/src/migrations.rs"
IMAGE = "postgres@sha256:f3bd19c606e442c3d7bdfa8002e03fe260a1023351e0ea4598032022b68dd6e3"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true")
    options = parser.parse_args()
    source = SOURCE.read_text()
    query = re.search(r'const fn schema_manifest_query\(\) -> &\x27static str \{\s*r"(.*?)"\s*\}', source, re.S)
    if query is None:
        raise SystemExit("cannot locate canonical schema manifest query")
    name = "ayni-workflow-schema-" + uuid.uuid4().hex[:12]
    try:
        subprocess.run(["docker", "run", "--detach", "--rm", "--name", name,
                        "--env", "POSTGRES_HOST_AUTH_METHOD=trust", IMAGE], check=True,
                       stdout=subprocess.DEVNULL)
        for _ in range(60):
            ready = subprocess.run(["docker", "exec", name, "pg_isready", "-U", "postgres"],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if ready.returncode == 0:
                break
            time.sleep(1)
        else:
            raise SystemExit("disposable PostgreSQL failed readiness")
        sql = """SET search_path TO public, pg_catalog, pg_temp;
CREATE TABLE public.workflow_schema_migrations (
version bigint PRIMARY KEY CHECK(version > 0), description text NOT NULL,
checksum text NOT NULL, applied_at timestamptz NOT NULL DEFAULT clock_timestamp());
"""
        for migration in sorted((SOURCE.parent.parent / "migrations").glob("*.sql")):
            sql += migration.read_text() + "\n"
        command = ["docker", "exec", "-i", name, "psql", "-XAt", "-v", "ON_ERROR_STOP=1", "-U", "postgres"]
        subprocess.run(command, input=sql.encode(), check=True, stdout=subprocess.DEVNULL)
        manifest = subprocess.check_output(command, input=query.group(1).encode()).removesuffix(b"\n")
        fingerprint = "sha256:" + hashlib.sha256(manifest).hexdigest()
        pattern = r'(const SCHEMA_MANIFEST_CHECKSUM: &str =\s*")[^"]+(";)'
        current = re.search(pattern, source)
        if current is None:
            raise SystemExit("cannot locate schema manifest constant")
        print(fingerprint)
        if options.write:
            SOURCE.write_text(re.sub(pattern, lambda match: match.group(1) + fingerprint + match.group(2), source))
        elif fingerprint not in current.group(0):
            raise SystemExit("workflow schema manifest differs; rerun with --write")
    finally:
        subprocess.run(["docker", "rm", "--force", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)


if __name__ == "__main__":
    main()
