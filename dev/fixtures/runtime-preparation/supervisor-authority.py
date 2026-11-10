"""Synthetic cleanup authority for the disposable supervisor qualification only.

Memory-only state exercises the real supervisor protocol; it does not implement
control-plane admission or substitute for the PostgreSQL authority regressions.
"""
import copy
import hashlib
import hmac
import json
import os
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

FROZEN_FIELDS = set('repository_id inspection_id source_commit source_tree source_connection_id source_connection_revision source_credential_revision installation_id provider_repository_id repository_full_name registry_connection_id registry_connection_revision registry_credential_revision registry_destination discovery_tag reusable_image publication_tag signer_id signer_revision signer_active_revision signer_key_id signer_public_key signer_secret_name signer_secret_uid policy lock policy_digest lock_digest lock_fingerprint sources profile_json'.split())
OPERATION_FIELDS = set('id idempotency_key repository_id inspection_id registry_connection_id signer_id platform state stage safe_reason generation allow_build allow_replace max_builds builds_started deadline_unix_ms accepted_at_unix_ms updated_at_unix_ms frozen_input_digest frozen evidence'.split())
ATTEMPT_FIELDS = set('id operation_id stage generation command_digest pod_uid state deadline_unix_ms accepted_at_unix_ms report_digest'.split())
BINDING_FIELDS = set('operation_id stage generation pod_uid command_digest'.split())
EVIDENCE_FIELDS = set('source_digest unsigned_manifest_digest certification_layer_digest candidate_image artifact_digest artifact_reference protected_content_root certificate_digest expected_manifest_digest image_digest content_verified managed_compatible managed_check_digest cleanup_complete details'.split())


def capability(token, binding):
    mac = hmac.new(token.encode('ascii'), b'ayni.runtime-preparation.worker.v1\0', hashlib.sha256)
    for value in (binding['operation_id'], binding['stage'], str(binding['generation']), binding['command_digest']):
        encoded = value.encode('utf-8')
        mac.update(len(encoded).to_bytes(8, 'big'))
        mac.update(encoded)
    return mac.hexdigest()


def validate_claim(claim):
    if set(claim) != {'operation', 'attempt'}:
        raise ValueError('claim_shape')
    op, attempt = claim['operation'], claim['attempt']
    if set(op) != OPERATION_FIELDS or set(attempt) != ATTEMPT_FIELDS:
        raise ValueError('record_shape')
    if set(op['frozen']) != FROZEN_FIELDS or set(op['evidence']) != EVIDENCE_FIELDS:
        raise ValueError('frozen_shape')
    if op['stage'] != 'cleanup' or attempt['stage'] != 'cleanup' or attempt['pod_uid']:
        raise ValueError('cleanup_only')
    if op['id'] != attempt['operation_id'] or op['generation'] != attempt['generation']:
        raise ValueError('claim_identity')
    return copy.deepcopy(claim)


class State:
    def __init__(self, token):
        self.token = token
        self.claim = None
        self.released = False
        self.observed_absent = ''
        self.report = None
        self.diagnostics = []
        self.stopped = False
        self.bindings = []
        self.polls = 0
        self.blocked_reports = 0
        self.rejected_stops = 0
        self.lock = threading.Lock()

    def binding(self):
        attempt = self.claim['attempt']
        return {key: attempt[key] for key in BINDING_FIELDS}

    def exact(self, binding, allow_unbound=False):
        if set(binding) != BINDING_FIELDS:
            return False
        expected = self.binding()
        if allow_unbound:
            expected['pod_uid'] = binding['pod_uid']
        return binding == expected and bool(binding['pod_uid'])

    def result(self):
        return {'configured': self.claim is not None, 'pod_uid': self.claim['attempt']['pod_uid'] if self.claim else '',
            'polls': self.polls, 'bindings': self.bindings, 'blocked_reports': self.blocked_reports,
            'report_accepted': self.report is not None, 'externally_observed_absent_uid': self.observed_absent,
            'stopped': self.stopped, 'rejected_premature_stops': self.rejected_stops,
            'diagnostics': self.diagnostics}

    def master(self, path, body):
        if path == '/configure' and self.claim is None:
            self.claim = validate_claim(body)
            return 200, {'accepted': True}
        if path == '/result':
            return 200, self.result()
        if self.claim is None:
            return 409, {}
        if path == '/v1/runtime-supervisor/diagnostics':
            binding = self.binding()
            if (body.get('kind'), body.get('execution_id'), body.get('generation'), body.get('stage'), body.get('pod_uid')) != (
                'runtime', binding['operation_id'], binding['generation'], binding['stage'], binding['pod_uid']
            ) or body.get('event') != 'retiring' or not self.report:
                return 409, {}
            if body not in self.diagnostics:
                self.diagnostics.append(copy.deepcopy(body))
            return 200, None
        if path == '/release':
            self.released = True
            return 200, {'accepted': True}
        if path == '/observe-absent':
            if body != {'pod_uid': self.claim['attempt']['pod_uid']} or not self.report:
                return 409, {}
            self.observed_absent = body['pod_uid']
            return 200, {'accepted': True}
        if path != '/v1/runtime-supervisor':
            return 403, {}
        action = body.get('action')
        if action == 'poll' and set(body) == {'action'}:
            self.polls += 1
            return 200, None if self.stopped else copy.deepcopy(self.claim)
        binding = body.get('binding', {})
        if set(body) != {'action', 'binding'}:
            return 403, {}
        if action == 'bind' and self.exact(binding, allow_unbound=True) and not self.report:
            previous = self.claim['attempt']['pod_uid']
            if previous and previous != binding['pod_uid']:
                return 409, {}
            self.claim['attempt']['pod_uid'] = binding['pod_uid']
            self.claim['attempt']['state'] = 'running'
            self.claim['operation']['state'] = 'running'
            self.bindings.append(binding['pod_uid'])
            return 200, None
        if action == 'stopped' and self.exact(binding):
            if not self.report or self.observed_absent != binding['pod_uid']:
                self.rejected_stops += 1
                return 409, {}
            self.stopped = True
            return 200, None
        return 409, {}

    def worker(self, path, body, authorization, uid, digest):
        if self.claim is None or self.stopped:
            return 409, {}
        binding = self.binding()
        expected = '/v1/runtime-worker/{operation_id}/{stage}/{generation}'.format(**binding)
        if path != expected or not binding['pod_uid'] or uid != binding['pod_uid'] or digest != binding['command_digest']:
            return 403, {}
        if not hmac.compare_digest(authorization, 'Bearer ' + capability(self.token, binding)):
            return 403, {}
        report = body.get('report', {})
        if set(body) != {'action', 'report'} or body['action'] != 'report' or set(report) != {'binding', 'outcome', 'safe_reason', 'evidence'}:
            return 403, {}
        evidence = report['evidence']
        if not self.exact(report['binding']) or report['outcome'] != 'completed' or report['safe_reason'] or set(evidence) != EVIDENCE_FIELDS:
            return 409, {}
        expected_evidence = {key: {} if key == 'details' else key == 'cleanup_complete' if key in ('cleanup_complete', 'content_verified', 'managed_compatible') else '' for key in EVIDENCE_FIELDS}
        if evidence != expected_evidence:
            return 409, {}
        if not self.released:
            self.blocked_reports += 1
            return 503, {}
        if self.report is not None and report != self.report:
            return 409, {}
        self.report = copy.deepcopy(report)
        self.claim['operation']['state'] = 'stopping'
        return 200, {'claim': None, 'accepted': True}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        try:
            size = int(self.headers.get('Content-Length', '0'))
            if not 0 <= size <= 131072:
                return self.reply(413, {})
            body = json.loads(self.rfile.read(size)) if size else {}
            state = self.server.state
            authorization = self.headers.get('Authorization', '')
            with state.lock:
                if hmac.compare_digest(authorization, 'Bearer ' + state.token):
                    status, value = state.master(self.path, body)
                else:
                    status, value = state.worker(self.path, body, authorization,
                        self.headers.get('x-runtime-pod-uid', ''), self.headers.get('x-runtime-command-digest', ''))
            self.reply(status, value)
        except (ValueError, TypeError, KeyError):
            self.reply(400, {})

    def reply(self, status, value):
        payload = json.dumps(value, separators=(',', ':')).encode('utf-8')
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Cache-Control', 'no-store')
        self.send_header('Content-Length', str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


if __name__ == '__main__':
    token = open(os.environ.get('AUTHORITY_TOKEN_PATH', '/authority/token'), encoding='ascii').read().strip()
    if len(token) != 64:
        raise SystemExit('synthetic_authority_token_invalid')
    server = ThreadingHTTPServer(('0.0.0.0', 8080), Handler)
    server.state = State(token)
    server.serve_forever()
