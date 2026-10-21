#!/usr/bin/env python3
"""Verify deployed session isolation without reading any Secret data."""
import argparse
import json
import subprocess


def kubectl(context, namespace, *arguments):
    result = subprocess.run(
        ["kubectl", "--context", context, "--namespace", namespace, *arguments],
        text=True, capture_output=True, check=False,
    )
    if result.returncode and not (arguments[:2] == ("auth", "can-i") and result.returncode == 1):
        result.check_returncode()
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--context", default="docker-desktop", choices=["docker-desktop"])
    parser.add_argument("--namespace", required=True, help="Application namespace")
    parser.add_argument("--minimum-conversations", type=int, default=2)
    arguments = parser.parse_args()
    namespace = arguments.namespace + "-sessions"
    owner = json.loads(kubectl(arguments.context, namespace, "get", "namespace", namespace, "-o", "json"))
    assert owner["metadata"]["labels"]["build.ayni/parent-namespace"] == arguments.namespace
    assert owner["metadata"]["annotations"]["helm.sh/resource-policy"] == "keep"
    pods = json.loads(kubectl(arguments.context, namespace, "get", "pods", "-l", "app.kubernetes.io/name=ayni-conversation-session", "-o", "json"))["items"]
    conversations = {}
    volumes = set()
    for pod in pods:
        spec = pod["spec"]
        assert spec["automountServiceAccountToken"] is False
        assert spec["securityContext"]["runAsNonRoot"] is True
        assert len(spec["containers"]) == 1
        container = spec["containers"][0]
        assert container["args"] == ["session-worker"]
        assert container["securityContext"]["readOnlyRootFilesystem"] is True
        assert container["securityContext"]["allowPrivilegeEscalation"] is False
        env = {item["name"]: item.get("value") for item in container["env"]}
        assert not any("BROKER" in name or "DATABASE" in name for name in env)
        conversation = env["ORCHESTRATOR_SESSION_CONVERSATION_ID"]
        scope = env["ORCHESTRATOR_SESSION_ID"]
        generation = env["ORCHESTRATOR_SESSION_GENERATION"]
        labels = pod["metadata"]["labels"]
        assert labels["build.ayni/session-id"] == scope
        assert labels["build.ayni/session-generation"] == generation
        assert conversations.setdefault(conversation, scope) == scope
        mounts = spec["volumes"]
        assert len(mounts) in (2, 3)
        pvc = next(item["persistentVolumeClaim"]["claimName"] for item in mounts if "persistentVolumeClaim" in item)
        token = next(item["secret"]["secretName"] for item in mounts if item["name"] == "token")
        assert pvc == token == pod["metadata"]["name"]
        provider = next((item for item in mounts if item["name"] == "provider"), None)
        if provider:
            assert provider["secret"]["secretName"] == "provider-openai-" + env["ORCHESTRATOR_SESSION_WORKSPACE_ID"].replace("-", "")
            assert provider["secret"]["items"] == [{"key": "api-key", "path": "api-key"}]
            assert provider["secret"]["optional"] is True
            assert env["ORCHESTRATOR_OPENAI_API_KEY_PATH"] == "/var/run/provider/api-key"
            assert not any(item.get("valueFrom") for item in container["env"])
            mount = next(item for item in container["volumeMounts"] if item["name"] == "provider")
            assert mount["mountPath"] == "/var/run/provider" and mount["readOnly"] is True
        else:
            assert "ORCHESTRATOR_OPENAI_API_KEY_PATH" not in env
        assert pvc not in volumes
        volumes.add(pvc)
        assert all("projected" not in item and "hostPath" not in item for item in mounts)
    assert len(conversations) >= arguments.minimum_conversations, conversations
    assert len(set(conversations.values())) == len(conversations)
    identity = "system:serviceaccount:" + arguments.namespace + ":conversation-session-supervisor"
    denied = kubectl(arguments.context, arguments.namespace, "auth", "can-i", "get", "secrets", "--as", identity).strip()
    assert denied == "no", denied
    allowed = kubectl(arguments.context, namespace, "auth", "can-i", "get", "secrets", "--as", identity).strip()
    assert allowed == "yes", allowed
    print(json.dumps({"result": "pass", "conversations": len(conversations), "session_pods": len(pods), "separate_pvcs": len(volumes), "application_secret_access": "denied"}))


if __name__ == "__main__":
    main()
