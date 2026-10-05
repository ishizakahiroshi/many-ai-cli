#!/usr/bin/env python3
"""Extract pinned Go helpers; run only inside fresh temporary synthetic roots.

Usage: source /path/to/pinned-env.sh; python3 extract_oracle.py /path/to/repo
Writes normalized JSON to stdout. Redirect receipts/failures outside fixtures.
No Git mutation runs against the supplied source repository.
"""
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

BASELINE = "21d0bc7935a2c4696fb89ccff2e324157a528c2d"
repo = Path(sys.argv[1]).resolve()
fixture = Path(__file__).resolve().parent

def source(path):
    return subprocess.run(["git", "-C", str(repo), "show", f"{BASELINE}:{path}"], check=True, capture_output=True, text=True).stdout

normal = source("internal/hub/normal_worktree.go").replace("package hub", "package main", 1)
orch = source("internal/hub/orchestration.go")
safe = re.search(r"func safeToken\(value string\) string \{.*?\n\}", orch, re.S).group(0)
pattern = re.search(r"var safeOrchestrationToken = .*", orch).group(0)
with tempfile.TemporaryDirectory(prefix="normal-worktree-oracle-") as tmp:
    root = Path(tmp)
    home = root / "home"
    home.mkdir()
    (root / "hooks").mkdir()
    empty = root / "empty-git-config"
    empty.write_text("")
    env = os.environ.copy()
    for key in list(env):
        if key.startswith("GIT_"):
            del env[key]
    env.update(HOME=str(home), USERPROFILE=str(home), XDG_CONFIG_HOME=str(home),
               GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=str(empty), GIT_CONFIG_COUNT="0",
               GIT_TERMINAL_PROMPT="0", GIT_AUTHOR_NAME="Synthetic Fixture", GIT_AUTHOR_EMAIL="fixture@example.invalid",
               GIT_COMMITTER_NAME="Synthetic Fixture", GIT_COMMITTER_EMAIL="fixture@example.invalid",
               NORMAL_WORKTREE_FIXTURE_ROOT=str(root), GO111MODULE="off", GOPROXY="off", GOSUMDB="off", GOTOOLCHAIN="local")
    (root / "normal.go").write_text(normal)
    (root / "token.go").write_text('package main\nimport("regexp";"strings")\n'+pattern+'\n'+safe+'\n')
    shutil.copyfile(fixture / "observe.go", root / "observe.go")
    subprocess.run([env.get("MANY_AI_GO_BINARY", "go"), "run", "normal.go", "token.go", "observe.go"], cwd=root, env=env, check=True)
