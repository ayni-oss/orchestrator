#!/usr/bin/env python3
"""Credential-free GitHub protocol fixture backed by an actual bare Git remote."""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

MAX_BODY = 64 * 1024
REPOSITORY = "fixture/repo"
BRANCH = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$")


def checked_branch(value):
    if not isinstance(value, str) or not BRANCH.fullmatch(value) or ".." in value or "//" in value:
        raise ValueError("invalid fixture branch")
    return value


class Store:
    def __init__(self, directory, pack, metadata):
        self.directory = Path(directory)
        self.directory.mkdir(parents=True, exist_ok=True)
        self.repository = self.directory / "fixture/repo.git"
        self.path = self.directory / "github.json"
        self.lock = threading.RLock()
        self.state = (json.loads(self.path.read_text()) if self.path.exists() else {
            "pulls": [], "create_calls": 0, "list_calls": 0,
            "drop_create_response": 0, "hold_create": False,
        })
        if not self.repository.exists():
            self.repository.parent.mkdir(parents=True, exist_ok=True)
            subprocess.run(["git", "init", "--bare", str(self.repository)], check=True,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            self.git("unpack-objects", "-r", input=Path(pack).read_bytes())
            commit = json.loads(Path(metadata).read_text())["source_commit"]
            self.git("update-ref", "refs/heads/main", commit)
            self.git("symbolic-ref", "HEAD", "refs/heads/main")
            self.git("config", "receive.denyNonFastForwards", "true")
            self.git("config", "receive.denyDeletes", "true")
        self.persist()

    def git(self, *arguments, input=None):
        return subprocess.check_output(["git", "--git-dir", str(self.repository), *arguments],
                                       input=input, stderr=subprocess.DEVNULL).decode().strip()

    def persist(self):
        temporary = self.path.with_suffix(".pending")
        with temporary.open("w") as output:
            json.dump(self.state, output, sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        temporary.replace(self.path)
        descriptor = os.open(self.directory, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)

    def snapshot(self):
        value = json.loads(json.dumps(self.state))
        value["references"] = self.git("for-each-ref", "--format=%(refname) %(objectname)",
                                        "refs/heads").splitlines()
        return value


class Handler(BaseHTTPRequestHandler):
    server_version = "PublicationFixture/1"

    def log_message(self, _format, *_arguments):
        # No request headers, credentials, source contents or PR bodies in logs.
        pass

    def respond(self, code, value):
        body = json.dumps(value, sort_keys=True).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def read_body(self):
        length = int(self.headers.get("Content-Length", "0"))
        if length < 1 or length > MAX_BODY:
            raise ValueError("fixture request bound")
        value = json.loads(self.rfile.read(length))
        if not isinstance(value, dict):
            raise ValueError("fixture object required")
        return value

    def do_GET(self):
        parsed = urlsplit(self.path)
        store = self.server.store
        with store.lock:
            if parsed.path == "/healthz":
                return self.respond(200, {"ready": True})
            if parsed.path == "/fixture/state":
                return self.respond(200, store.snapshot())
            if parsed.path == f"/repos/{REPOSITORY}/pulls":
                query = parse_qs(parsed.query)
                head = query.get("head", [""])[0].removeprefix("fixture:")
                base = query.get("base", [""])[0]
                state = query.get("state", ["open"])[0]
                store.state["list_calls"] += 1
                store.persist()
                pulls = [pull for pull in store.state["pulls"]
                         if (not head or pull["head"]["ref"] == head)
                         and (not base or pull["base"]["ref"] == base)
                         and (state == "all" or pull["state"] == state)]
                return self.respond(200, pulls)
            match = re.fullmatch(r"/repos/fixture/repo/pulls/(\d+)", parsed.path)
            if match:
                pull = next((p for p in store.state["pulls"]
                             if p["number"] == int(match.group(1))), None)
                return self.respond(200 if pull else 404, pull or {"message": "not found"})
            prefix = f"/repos/{REPOSITORY}/git/ref/heads/"
            if parsed.path.startswith(prefix):
                try:
                    branch = checked_branch(parsed.path.removeprefix(prefix))
                    sha = store.git("rev-parse", "--verify", f"refs/heads/{branch}^{{commit}}")
                except (ValueError, subprocess.CalledProcessError):
                    return self.respond(404, {"message": "reference not found"})
                return self.respond(200, {"ref": f"refs/heads/{branch}",
                                         "object": {"type": "commit", "sha": sha}})
            self.respond(404, {"message": "unsupported fixture operation"})

    def do_POST(self):
        try:
            body = self.read_body()
            self.post(body)
        except (ValueError, KeyError, TypeError, json.JSONDecodeError):
            self.respond(400, {"message": "invalid fixture request"})

    def post(self, body):
        path = urlsplit(self.path).path
        store = self.server.store
        with store.lock:
            if path == "/fixture/control":
                for name in ("drop_create_response", "hold_create"):
                    if name in body:
                        if name == "drop_create_response":
                            if type(body[name]) is not int or not 0 <= body[name] <= 16:
                                raise ValueError("drop bound")
                        elif type(body[name]) is not bool:
                            raise ValueError("hold boolean")
                        store.state[name] = body[name]
                store.persist()
                return self.respond(200, {"configured": True})
            if path == "/fixture/branch":
                branch = checked_branch(body["branch"])
                if branch == "main" or body.get("value") != "base":
                    raise ValueError("fixture collision branch required")
                commit = store.git("rev-parse", "refs/heads/main")
                try:
                    store.git("update-ref", f"refs/heads/{branch}", commit, "0" * 40)
                except subprocess.CalledProcessError:
                    return self.respond(409, {"message": "branch already exists"})
                return self.respond(201, {"branch": branch, "sha": commit})
            match = re.fullmatch(r"/fixture/pulls/(\d+)", path)
            if match:
                pull = next((p for p in store.state["pulls"]
                             if p["number"] == int(match.group(1))), None)
                if not pull:
                    return self.respond(404, {"message": "not found"})
                if body.get("state") != "closed" or type(body.get("merged", False)) is not bool:
                    raise ValueError("fixture supports closure only")
                pull["state"] = "closed"
                pull["merged"] = body.get("merged", False)
                pull["draft"] = not pull["merged"]
                store.persist()
                return self.respond(200, pull)
            if path != f"/repos/{REPOSITORY}/pulls":
                return self.respond(404, {"message": "unsupported fixture operation"})
            store.state["create_calls"] += 1
            store.persist()
            if store.state["hold_create"]:
                return self.respond(503, {"message": "fixture create held"})
            head, base = checked_branch(body["head"]), checked_branch(body["base"])
            if body.get("draft") is not True or body.get("maintainer_can_modify") is not False:
                raise ValueError("draft without maintainer modification required")
            if not all(isinstance(body.get(name), str) and body[name]
                       for name in ("title", "body")):
                raise ValueError("title and body required")
            if any(p["head"]["ref"] == head and p["base"]["ref"] == base and p["state"] == "open"
                   for p in store.state["pulls"]):
                return self.respond(422, {"message": "pull request already exists"})
            try:
                commit = store.git("rev-parse", "--verify", f"refs/heads/{head}^{{commit}}")
                base_commit = store.git("rev-parse", "--verify", f"refs/heads/{base}^{{commit}}")
            except subprocess.CalledProcessError:
                return self.respond(422, {"message": "published references missing"})
            number = len(store.state["pulls"]) + 1
            repository = {"full_name": REPOSITORY}
            pull = {"number": number, "state": "open", "draft": True, "merged": False,
                    "html_url": f"https://github.com/{REPOSITORY}/pull/{number}",
                    "title": body["title"], "body": body["body"],
                    "head": {"sha": commit, "ref": head, "repo": repository},
                    "base": {"sha": base_commit, "ref": base, "repo": repository}}
            store.state["pulls"].append(pull)
            drop = store.state["drop_create_response"] > 0
            if drop:
                store.state["drop_create_response"] -= 1
            store.persist()
            if drop:
                self.close_connection = True
                try:
                    self.connection.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                return
            self.respond(201, pull)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", default="/var/lib/publication-fixture")
    parser.add_argument("--source-pack", default="/opt/fixture/source.pack")
    parser.add_argument("--source-metadata", default="/opt/fixture/source.json")
    parser.add_argument("--http-port", type=int, default=8080)
    parser.add_argument("--git-port", type=int, default=9418)
    args = parser.parse_args()
    store = Store(args.directory, args.source_pack, args.source_metadata)
    daemon = subprocess.Popen(["git", "daemon", "--reuseaddr", "--export-all", "--enable=receive-pack",
                               "--listen=0.0.0.0", f"--port={args.git_port}",
                               f"--base-path={store.directory}", str(store.directory)],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    server = ThreadingHTTPServer(("0.0.0.0", args.http_port), Handler)
    server.store = store
    signal.signal(signal.SIGTERM, lambda _signum, _frame: (_ for _ in ()).throw(SystemExit(0)))
    try:
        server.serve_forever()
    finally:
        server.server_close()
        daemon.terminate()
        daemon.wait(timeout=5)


if __name__ == "__main__":
    main()
