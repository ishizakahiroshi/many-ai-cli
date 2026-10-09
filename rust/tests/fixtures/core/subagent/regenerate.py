#!/usr/bin/env python3
"""Regenerate using unmodified Go Hub readers via a test-only virtual file overlay.
Use the task-approved Go binary/cache settings. Only synthetic fixtures are read.
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile

here = Path(__file__).resolve().parent
root = here.parents[4]
with tempfile.TemporaryDirectory(prefix="subagent-oracle-") as temporary:
    temporary = Path(temporary)
    overlay = temporary / "overlay.json"
    overlay.write_text(json.dumps({"Replace": {
        str(root / "internal/hub/rust_subagent_oracle_test.go"):
        str(here / "oracle_test.go.txt")
    }}))
    environment = dict(os.environ)
    environment.update(HOME=str(temporary), RUST_SUBAGENT_INPUT=str(here / "go-input.json"),
                       RUST_SUBAGENT_OUTPUT=str(here / "go-golden.json"))
    subprocess.run(["go", "test", "-count=1", "-overlay", str(overlay),
                    "./internal/hub", "-run", "^TestRustSubagentGolden$"],
                   cwd=root, env=environment, check=True)
