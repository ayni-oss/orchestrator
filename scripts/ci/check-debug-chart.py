#!/usr/bin/env python3
"""Render both debug modes and verify the collection and configuration boundary."""
import os
import pathlib
import subprocess
root=pathlib.Path(__file__).resolve().parents[2]
def render(enabled):
    return subprocess.check_output(['helm','template','debug',str(root/'deploy/charts/ayni-orchestrator'),'--namespace','test','--set',f'debug.enabled={str(enabled).lower()}','--set-string','images.controlPlane=local/test:one','--set-string','images.conversationSupervisor=local/test:one'],env=dict(os.environ,HELM_NO_PLUGINS='1'),text=True)
for enabled in (False,True):
    text=render(enabled)
    config=next(doc for doc in text.split('\n---') if 'kind: ConfigMap\n' in doc and '  name: control-plane-config\n' in doc)
    metadata,data=config.split('\ndata:\n',1)
    assert 'ORCHESTRATOR_' not in metadata
    assert f'ORCHESTRATOR_DEBUG: "{str(enabled).lower()}"' in data
    assert ('kind: StatefulSet\nmetadata:\n  name: log-collector\n' in text)==enabled
    if enabled:
        assert 'resources: [pods/log]' in text
        assert 'volumeClaimTemplates:' in text
        assert 'hostPath:' not in text
        assert 'kind: ClusterRole\n' not in text
        assert 'ORCHESTRATOR_TELEMETRY_TOKEN_PATH:' in data
print('Debug collection chart boundaries passed')
