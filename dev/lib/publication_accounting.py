"""Account for this acceptance run without discarding retained fixture history."""
import json
from pathlib import Path
import sys


def verify(baseline, current, creates, pulls, identity=None):
    if current['create_calls'] - baseline['create_calls'] != creates:
        raise ValueError('unexpected publication create request delta')
    if len(current['pulls']) - len(baseline['pulls']) != pulls:
        raise ValueError('unexpected retained publication delta')
    retained = {pull['number']: pull for pull in current['pulls']}
    if len(retained) != len(current['pulls']):
        raise ValueError('duplicate retained publication number')
    for previous in baseline['pulls']:
        if retained.get(previous['number']) != previous:
            raise ValueError('previously retained publication changed')
    if not set(baseline['references']).issubset(current['references']):
        raise ValueError('previously retained Git reference changed')
    if identity is not None:
        matching = [pull for pull in current['pulls']
                    if pull['number'] == identity['pr_number']
                    and pull['head']['ref'] == identity['branch']]
        if len(matching) != 1:
            raise ValueError('current publication identity missing')
        pull = matching[0]
        if pull['number'] in {old['number'] for old in baseline['pulls']}:
            raise ValueError('current publication reused retained identity')
        if pull['head']['sha'] != identity['commit_sha'] or not pull['draft'] or pull['state'] != 'open':
            raise ValueError('current publication differs from selected draft')
        if f"refs/heads/{identity['branch']} {identity['commit_sha']}" not in current['references']:
            raise ValueError('current publication Git reference missing')
    return {'create_requests': creates, 'retained_prs': pulls,
            'baseline_create_requests': baseline['create_calls'],
            'baseline_retained_prs': len(baseline['pulls']),
            'total_create_requests': current['create_calls'],
            'total_retained_prs': len(current['pulls'])}


if __name__ == '__main__':
    baseline, current = [json.loads(Path(path).read_text()) for path in sys.argv[1:3]]
    identity = json.loads(Path(sys.argv[5]).read_text()) if len(sys.argv) > 5 else None
    verify(baseline, current, int(sys.argv[3]), int(sys.argv[4]), identity)
