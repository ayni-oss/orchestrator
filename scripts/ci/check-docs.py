#!/usr/bin/env python3
"""Validate repository-local Markdown link targets without network access."""
from pathlib import Path
import re
import subprocess
from urllib.parse import unquote

root = Path(__file__).resolve().parents[2]
files = subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard'], cwd=root, text=True).splitlines()
errors = []
for filename in sorted(set(files)):
    source = root / filename
    if source.suffix != '.md' or not source.is_file():
        continue
    text = re.sub(r'```.*?```', '', source.read_text(), flags=re.S)
    for match in re.finditer(r'\]\((<[^>]+>|[^\s)]+)(?:\s+"[^"]*")?\)', text):
        target = match.group(1).strip('<>')
        if re.match(r'[a-zA-Z][a-zA-Z0-9+.-]*:', target) or target.startswith('#'):
            continue
        filename = unquote(target.split('#', 1)[0])
        destination = (root / 'docs' / filename.lstrip('/')) if filename.startswith('/') else source.parent / filename
        if not destination.exists():
            errors.append(f'{source.relative_to(root)}: missing {target}')
if errors:
    raise SystemExit('\n'.join(errors))
print('Repository Markdown link targets are valid.')
