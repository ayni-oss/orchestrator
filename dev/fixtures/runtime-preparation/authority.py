"""Disposable component qualification authority; never deployed by the product chart.

Inputs are synthetic and memory-only. This deliberately does not emulate PostgreSQL
admission, real GitHub acquisition, GHCR publication, or supervisor recovery.
"""
import copy
import json
import os
from http.server import BaseHTTPRequestHandler, HTTPServer

CLAIM = None
REPORT = None
TOKEN = open('/authority/token', encoding='ascii').read().strip()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        global CLAIM, REPORT
        if self.headers.get('Authorization') != 'Bearer ' + TOKEN:
            self.send_error(403)
            return
        size = int(self.headers.get('Content-Length', '0'))
        if size > 131072:
            self.send_error(413)
            return
        body = json.loads(self.rfile.read(size)) if size else {}
        if self.path == '/configure':
            CLAIM, REPORT = body, None
            self.reply({'accepted': True})
            return
        if self.path == '/result':
            self.reply(REPORT)
            return
        if CLAIM is None:
            self.send_error(409)
            return
        stage = CLAIM['attempt']['stage']
        expected = '/v1/runtime-worker/{}/{}/{}'.format(
            CLAIM['operation']['id'], stage, CLAIM['attempt']['generation'])
        if self.headers.get('x-runtime-command-digest') != CLAIM['attempt']['command_digest']:
            self.send_error(403)
            return
        if self.path == expected + '/credential' and stage == 'discover':
            # Deliberately invalid synthetic credentials, never an operator token.
            self.reply({'kind': 'registry', 'username': 'runtime-qualification-invalid',
                'password': 'runtime-qualification-invalid'})
            return
        if self.path != expected:
            self.send_error(403)
            return
        pod_uid = self.headers.get('x-runtime-pod-uid', '')
        if body.get('action') == 'input':
            claim = copy.deepcopy(CLAIM)
            claim['attempt']['pod_uid'] = pod_uid
            self.reply({'claim': claim, 'accepted': True})
        elif body.get('action') == 'report':
            binding = body.get('report', {}).get('binding', {})
            if binding.get('pod_uid') != pod_uid or binding.get('stage') != stage:
                self.send_error(409)
                return
            REPORT = body['report']
            self.reply({'claim': None, 'accepted': True})
        else:
            self.send_error(403)

    def reply(self, value):
        payload = json.dumps(value, separators=(',', ':')).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Cache-Control', 'no-store')
        self.send_header('Content-Length', str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


HTTPServer(('0.0.0.0', 8080), Handler).serve_forever()
