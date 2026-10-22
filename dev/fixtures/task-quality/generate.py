#!/usr/bin/env python3
"""Freeze the real managed-runtime fixture with deterministic Git identities."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

root = Path(__file__).resolve().parent
with tempfile.TemporaryDirectory(prefix="ayni-task-quality-") as directory:
    checkout = Path(directory)
    for source in sorted(root.joinpath("source").rglob("*")):
        if source.is_file() and ".ayni" not in source.relative_to(root / "source").parts:
            destination = checkout / source.relative_to(root / "source")
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)

    def git(*arguments, input=None):
        return subprocess.check_output(
            ["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false",
             "-C", str(checkout), *arguments], input=input, stderr=subprocess.DEVNULL,
            env={**os.environ, "GIT_AUTHOR_DATE": "2000-01-01T00:00:00Z",
                 "GIT_COMMITTER_DATE": "2000-01-01T00:00:00Z"})

    git("init", "--initial-branch=main")
    git("add", ".")
    git("-c", "user.name=Task Fixture", "-c", "user.email=fixture@example.invalid",
        "commit", "-m", "test: freeze managed task quality fixture")
    objects = git("rev-list", "--objects", "--no-object-names", "HEAD")
    pack = git("pack-objects", "--stdout", input=objects)
    root.joinpath("source.pack").write_bytes(pack)
    metadata = {
        "source_commit": git("rev-parse", "HEAD").decode().strip(),
        "lock_digest": "sha256:" + hashlib.sha256(checkout.joinpath(".ayni.lock").read_bytes()).hexdigest(),
        "source_pack_digest": "sha256:" + hashlib.sha256(pack).hexdigest(),
    }
    root.joinpath("source.json").write_text(json.dumps(metadata, indent=2) + "\n")
