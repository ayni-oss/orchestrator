#!/usr/bin/env python3
"""Exercise the chart launcher with public dummy credentials on pinned NATS.

No deployed Secrets or operator environment values are accessed. The protocol
checks exact authentication, including strings the NATS config parser normally
misinterprets as numbers, variables or delimiters.
"""

import json
import pathlib
import socket
import subprocess
import tempfile
import time
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[2]
IMAGE = "nats:2.15.0-alpine@sha256:ac8f88a6494bffc2c2a5289a0ca61cb28a9145c11ba5677cf24265d07f46d8d4"
LAUNCHER = ROOT / "dev/charts/ayni-orchestrator-local/files/jetstream-start.sh"
VARIABLES = ["WORKFLOW_ADMIN_PASSWORD", "WORKFLOW_APPLICATION_PASSWORD",
             "WORKFLOW_EXECUTOR_PASSWORD", "WORKFLOW_ACCEPTANCE_PASSWORD"]
CASES = {
    "digit-leading-hex": "9e1234567890abcdef0123456789abcdef",
    "integer": "1234567890",
    "boolean": "true",
    "quotes-and-backslash": 'fixture-"quoted"-\\path',
    "variable-looking": "$WORKFLOW_OTHER_PASSWORD",
    "whitespace-and-controls": " public fixture\twith\nnewlines\r\x01\x7f\n",
    "unicode": "public-fixture-åñ界",
}


def authenticate(port, password):
    with socket.create_connection(("127.0.0.1", port), timeout=2) as connection:
        connection.settimeout(2)
        greeting = connection.recv(8192)
        if not greeting.startswith(b"INFO "):
            raise RuntimeError("NATS greeting missing")
        packet = json.dumps({"user": "fixture", "pass": password, "verbose": True})
        connection.sendall(b"CONNECT " + packet.encode() + b"\r\nPING\r\n")
        response = b""
        while b"PONG" not in response and b"-ERR" not in response:
            chunk = connection.recv(8192)
            if not chunk:
                return False
            response += chunk
        return b"PONG" in response and b"-ERR" not in response


def main():
    with tempfile.TemporaryDirectory(prefix="ayni-nats-passwords-") as temporary:
        fixture = pathlib.Path(temporary)
        (fixture / "start.sh").write_text(LAUNCHER.read_text())
        (fixture / "nats.conf").write_text(
            'port: 4222\nauthorization { users: [{ user: "fixture", '
            'password: $WORKFLOW_ADMIN_PASSWORD }] }\n'
        )
        for label, password in CASES.items():
            container = "ayni-nats-password-" + uuid.uuid4().hex[:12]
            command = ["docker", "run", "--detach", "--name", container,
                       "--entrypoint", "/bin/sh", "--publish", "127.0.0.1::4222",
                       "--mount", f"type=bind,source={temporary},target=/fixture,readonly"]
            for variable in VARIABLES:
                command += ["--env", variable + "=" + password]
            command += [IMAGE, "/fixture/start.sh", "--config=/fixture/nats.conf"]
            try:
                subprocess.run(command, check=True, stdout=subprocess.DEVNULL)
                port = int(subprocess.check_output(
                    ["docker", "port", container, "4222/tcp"], text=True
                ).strip().rsplit(":", 1)[1])
                for attempt in range(50):
                    try:
                        accepted = authenticate(port, password)
                        break
                    except (ConnectionError, OSError):
                        if attempt == 49:
                            raise RuntimeError(f"NATS fixture did not become ready: {label}") from None
                        time.sleep(0.1)
                if not accepted or authenticate(port, "wrong-public-fixture-password"):
                    raise RuntimeError(f"NATS authentication did not preserve input: {label}")
                print(f"nats-password-{label}=pass")
            except Exception:
                # This container has only the public dummy values defined above.
                result = subprocess.run(["docker", "logs", container], capture_output=True, text=True)
                print(result.stdout + result.stderr)
                raise
            finally:
                subprocess.run(["docker", "rm", "--force", container], check=False,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


if __name__ == "__main__":
    main()
