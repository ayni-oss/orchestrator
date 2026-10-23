#!/usr/bin/env python3
"""Actual Git transport and retained GitHub response-loss fixture regression."""
import http.client
import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parent
SOURCE = ROOT.parent / "task-quality"


def port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


class PublicationFixtureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="publication-fixture-")
        self.directory = Path(self.temporary.name)
        self.http_port, self.git_port = port(), port()
        self.process = None
        self.start()

    def tearDown(self):
        self.stop()
        self.temporary.cleanup()

    def start(self):
        self.process = subprocess.Popen([
            sys.executable, str(ROOT / "server.py"), "--directory", str(self.directory / "remote"),
            "--source-pack", str(SOURCE / "source.pack"), "--source-metadata", str(SOURCE / "source.json"),
            "--http-port", str(self.http_port), "--git-port", str(self.git_port),
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(100):
            try:
                self.request("/healthz")
                return
            except OSError:
                time.sleep(.02)
        self.fail("fixture did not start")

    def stop(self):
        if self.process:
            self.process.terminate()
            self.process.wait(timeout=5)
            self.process = None

    def request(self, path, body=None):
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.http_port}{path}",
            data=None if body is None else json.dumps(body).encode(),
            headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(request, timeout=3) as response:
            return json.load(response)

    def publish_branch(self):
        checkout = self.directory / "checkout"
        subprocess.run(["git", "clone", f"git://127.0.0.1:{self.git_port}/fixture/repo.git", str(checkout)],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(["git", "-C", str(checkout), "-c", "user.name=Publication Fixture",
                        "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false",
                        "-c", "core.hooksPath=/dev/null", "commit", "--allow-empty", "-m", "test: draft fixture"],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(["git", "-C", str(checkout), "push", "origin", "HEAD:refs/heads/task/fixture"],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"]).decode().strip()

    def test_lost_create_response_recovers_exact_pr_after_restart_and_closure(self):
        commit = self.publish_branch()
        self.request("/fixture/control", {"drop_create_response": 1})
        body = {"title": "feat(tasks): publish fixture", "body": "<!-- publication:fixture -->",
                "head": "task/fixture", "base": "main", "draft": True,
                "maintainer_can_modify": False}
        with self.assertRaises((OSError, http.client.HTTPException)):
            self.request("/repos/fixture/repo/pulls", body)
        self.stop()
        self.start()
        pulls = self.request("/repos/fixture/repo/pulls?state=all&head=fixture:task/fixture&base=main")
        self.assertEqual(len(pulls), 1)
        self.assertEqual(pulls[0]["head"]["sha"], commit)
        self.request("/fixture/pulls/1", {"state": "closed", "merged": True})
        recovered = self.request("/repos/fixture/repo/pulls?state=all&head=fixture:task/fixture&base=main")
        self.assertEqual(recovered[0]["number"], 1)
        self.assertTrue(recovered[0]["merged"])
        self.assertEqual(self.request("/fixture/state")["create_calls"], 1)

    def test_collision_control_preserves_existing_branch_and_main(self):
        original = self.request("/fixture/branch", {"branch": "task/collision", "value": "base"})
        with self.assertRaises(urllib.error.HTTPError) as rejected:
            self.request("/fixture/branch", {"branch": "task/collision", "value": "base"})
        self.assertEqual(rejected.exception.code, 409)
        rejected.exception.close()
        reference = self.request("/repos/fixture/repo/git/ref/heads/task/collision")
        self.assertEqual(reference["object"]["sha"], original["sha"])
        with self.assertRaises(urllib.error.HTTPError) as rejected:
            self.request("/fixture/branch", {"branch": "main", "value": "base"})
        self.assertEqual(rejected.exception.code, 400)
        rejected.exception.close()

    def test_create_requires_published_head_and_draft_authority(self):
        body = {"title": "test: bounded fixture", "body": "fixture", "head": "task/missing",
                "base": "main", "draft": True, "maintainer_can_modify": False}
        with self.assertRaises(urllib.error.HTTPError) as rejected:
            self.request("/repos/fixture/repo/pulls", body)
        self.assertEqual(rejected.exception.code, 422)
        rejected.exception.close()
        self.assertEqual(self.request("/fixture/state")["pulls"], [])


if __name__ == "__main__":
    unittest.main()
