#!/usr/bin/env python3
"""Reproduce synthetic observations using hash-pinned, untouched Go sources."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
manifest = json.loads((HERE / 'source-sha256.json').read_text())
with tempfile.TemporaryDirectory(prefix='many-launcher-oracle-') as temporary:
    destination = Path(temporary)
    for relative, entry in manifest['files'].items():
        source = ROOT / relative
        data = source.read_bytes()
        if hashlib.sha256(data).hexdigest() != entry['sha256']:
            raise SystemExit(f'Go oracle source changed: {relative}')
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    (destination / 'internal/launcher/oracle_test.go').write_bytes((HERE / 'oracle_test.go.txt').read_bytes())
    env = dict(os.environ)
    env['GOFLAGS'] = '-buildvcs=false'
    result = subprocess.run(['go', 'test', '-count=1', '-run', '^TestRustLauncherOracle$', '-v', './internal/launcher'], cwd=destination, env=env, capture_output=True, text=True, check=False)
    if result.returncode:
        raise SystemExit(result.stdout + result.stderr)
    prefix = 'RUST_LAUNCHER_ORACLE='
    lines = [line[len(prefix):] for line in result.stdout.split('\n') if line.startswith(prefix)]
    if len(lines) != 1:
        raise SystemExit('Go fixture did not return exactly one corpus')
    rows = json.loads(lines[0])
    (HERE / 'go-oracle.json').write_text(json.dumps(rows, ensure_ascii=False, indent=2) + '\n')
    print(f'actual Go launcher oracle: {len(rows)} cases; source {manifest["oracle"]}')
