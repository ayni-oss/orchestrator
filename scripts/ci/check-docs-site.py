#!/usr/bin/env python3
"""Confirm that the published site contains only the human-facing source pages."""
from pathlib import Path

root = Path(__file__).resolve().parents[2]
source = root / 'docs/site'
output = root / 'docs/.vitepress/dist'
expected = {p.relative_to(source).with_suffix('.html') for p in source.rglob('*.md')}
actual = {p.relative_to(output) for p in output.rglob('*.html')}
missing = expected - actual
unexpected = actual - expected - {Path('404.html')}
if missing or unexpected:
    raise SystemExit(f'Site audience boundary failed: missing={sorted(map(str, missing))}, unexpected={sorted(map(str, unexpected))}')
if (output / 'CNAME').read_text().strip() != 'orchestrator.ayni.build':
    raise SystemExit('Site CNAME must be orchestrator.ayni.build')
home = (output / 'index.html').read_text()
for required in ['ayni-brand-mark', '<svg', 'favicon.svg', 'Durable workflow foundations', 'PostgreSQL authority']:
    if required not in home:
        raise SystemExit(f'Homepage branding/content is missing: {required}')
if not (output / 'favicon.svg').is_file():
    raise SystemExit('The favicon must be included in published assets')
print(f'Human-facing site boundary is valid: {len(expected)} pages; no technical pages published.')
