#!/usr/bin/env python3
"""Runtime smoke of one native candidate: version -> serve -> status -> reuse -> stop.

Starts the already-built many-ai-cli exactly as the product's own isolation does
(--trial-root/--trial-port, see rust/src/cli.rs) inside a fresh temporary directory,
with HOME-like variables redirected there as well. Nothing is written outside that
directory, the default port 47777 is never used, and the Hub is always stopped and
the directory removed afterwards. The Hub token is never printed or recorded.
"""
from __future__ import annotations

import argparse
import dataclasses
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
from typing import Callable

SCHEMA_VERSION = 1
TARGET_MODES = {
    "x86_64-unknown-linux-gnu": "linux",
    "x86_64-pc-windows-msvc": "windows-native",
    "x86_64-apple-darwin": "darwin",
    "aarch64-apple-darwin": "darwin",
}
DEFAULT_PORT = 47777
LOOPBACK = "127.0.0.1"
PROFILE_VARIABLES = ("CODEX_HOME", "CLAUDE_CONFIG_DIR", "GROK_HOME", "COPILOT_CONFIG_DIR",
                     "CURSOR_CONFIG_DIR", "OPENCODE_CONFIG_DIR", "COMMAND_CODE_HOME")
CHECK_IDS = ("prepare", "version", "status-stopped-before", "serve-start", "ledger",
             "status-running", "api-info", "reuse-existing-hub", "stop", "stopped-after",
             "binary-unchanged")
LIMITS = (
    "Synthetic isolated runtime only: no user Hub, home, data, shortcuts, browser or GUI.",
    "Fixed routes only (status command, GET / and GET /api/info); page JavaScript and screens are not exercised.",
    "No provider, subscription or quota call is made; network egress is neither blocked nor measured.",
    "Install, update, rollback, setup and tray are not exercised.",
)

# Seconds. The first launch of a fresh binary can be slow (for example the first-run
# checks of macOS), so the first commands get more room than later ones.
TOTAL_SECONDS = 240.0
VERSION_SECONDS = 30.0
STATUS_SECONDS = 20.0
START_SECONDS = 60.0
POLL_SECONDS = 0.5
REUSE_SECONDS = 20.0
STOP_SECONDS = 30.0
EXIT_SECONDS = 20.0
INFO_SECONDS = 5.0
CLEANUP_STOP_SECONDS = 15.0
CLEANUP_SIGNAL_SECONDS = 5.0
TAIL_CHARS = 1200
READ_CAP = 1024 * 1024


class SmokeFailure(Exception):
    """A failed check. `code` and `detail` must never contain the Hub token."""

    def __init__(self, code: str, detail: dict | None = None):
        super().__init__(code)
        self.code = code
        self.detail = detail or {}


@dataclasses.dataclass(frozen=True)
class CommandResult:
    code: int | None
    stdout: bytes = b""
    stderr: bytes = b""
    timed_out: bool = False


@dataclasses.dataclass(frozen=True)
class SmokeConfig:
    target: str
    binary: Path
    binary_sha256: str
    expected_version: str
    expected_commit: str
    expected_build_time: str
    temp_parent: Path | None = None
    total_seconds: float = TOTAL_SECONDS


# ---------------------------------------------------------------- pure helpers

_TOKEN_QUERY = re.compile(r"(?i)(token=)[^\s&\"'<>]+")
_BEARER = re.compile(r"(?i)(bearer\s+)\S+")
_LONG_HEX = re.compile(r"[0-9a-fA-F]{32,}")


def redact(text: str, secrets: tuple[str, ...] = ()) -> str:
    """Remove the known token and anything shaped like a token or long hex secret."""
    for secret in secrets:
        if secret:
            text = text.replace(secret, "<redacted>")
    text = _TOKEN_QUERY.sub(r"\1<redacted>", text)
    text = _BEARER.sub(r"\1<redacted>", text)
    return _LONG_HEX.sub("<redacted>", text)


def tail_text(data: bytes, secrets: tuple[str, ...] = (), limit: int = TAIL_CHARS) -> str:
    """Redact the whole buffer first, then keep its end (a cut token can't survive)."""
    return redact(data[:READ_CAP].decode("utf-8", "replace"), secrets).strip()[-limit:]


_LEVEL = re.compile(r"\blevel=(\w+)")
_MESSAGE = re.compile(r'\bmsg=("(?:[^"\\]|\\.)*"|\S+)')


def summarize_hub_log(data: bytes, secrets: tuple[str, ...] = ()) -> dict:
    """The Hub mirrors its log lines to stderr. Keep counts per level and WARN/ERROR messages."""
    levels: dict[str, int] = {}
    problems: list[str] = []
    for line in data[:READ_CAP].decode("utf-8", "replace").splitlines():
        found = _LEVEL.search(line)
        if not found:
            continue
        level = found.group(1).upper()
        levels[level] = levels.get(level, 0) + 1
        message = _MESSAGE.search(line)
        if level in ("WARN", "WARNING", "ERROR") and message and len(problems) < 10:
            problems.append(redact(message.group(1), secrets)[:120])
    return {"lines": len(data.splitlines()), "levels": levels, "problems": problems}


def expected_runtime_mode(target: str) -> str:
    try:
        return TARGET_MODES[target]
    except KeyError:
        raise ValueError(f"unknown target: {target}") from None


def parse_status(stdout: bytes) -> dict:
    """Classify `status` output without keeping the URL (it carries the token)."""
    lines = [line.strip() for line in stdout.decode("utf-8", "replace").splitlines() if line.strip()]
    if lines == ["stopped"]:
        return {"kind": "stopped"}
    if lines and lines[0].startswith("running "):
        try:
            parts = urllib.parse.urlsplit(lines[0][len("running "):].strip())
            scheme, host, port = parts.scheme, parts.hostname, parts.port
        except ValueError:
            scheme = host = port = None
        return {"kind": "running", "scheme": scheme, "host": host, "port": port,
                "stale_warning": any("WARNING" in line for line in lines[1:]),
                "extra_lines": len(lines) - 1}
    return {"kind": "other", "lines": len(lines)}


def read_token(config_path: Path) -> str | None:
    """Read the token the application itself wrote. Returned only to the caller."""
    try:
        data = config_path.read_bytes()
    except OSError:
        return None
    if len(data) > 8 * READ_CAP:
        return None
    match = re.search(r"(?m)^token:[ \t]*[\"']?([A-Za-z0-9_-]{16,256})[\"']?[ \t]*\r?$",
                      data.decode("utf-8", "replace"))
    return match.group(1) if match else None


def read_ledger(path: Path) -> dict:
    try:
        data = path.read_bytes()
    except OSError as error:
        raise SmokeFailure("ledger_unreadable", {"error": type(error).__name__}) from None
    if len(data) > READ_CAP:
        raise SmokeFailure("ledger_too_large")
    try:
        value = json.loads(data)
    except ValueError:
        raise SmokeFailure("ledger_invalid") from None
    if not isinstance(value, dict):
        raise SmokeFailure("ledger_invalid")
    return value


def compare_info(info: dict, expected: dict[str, str]) -> list[str]:
    bad = [key for key, wanted in expected.items()
           if not isinstance(info.get(key), str) or info[key] != wanted]
    if info.get("binary_stale") is not False:
        bad.append("binary_stale")
    if info.get("web_dist_fresh") is not True:
        bad.append("web_dist_fresh")
    return bad


def digest(path: Path) -> str:
    if path.is_symlink() or not path.is_file():
        raise ValueError("binary must be a regular non-symlink file")
    value = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            value.update(block)
    return value.hexdigest()


def isolated_environment(root: Path, *, windows: bool | None = None,
                         host: dict[str, str] | None = None) -> dict[str, str]:
    """Child environment built from scratch: nothing from the runner is inherited.

    Every path-valued entry points inside `root` (the system directories on Windows
    excepted), so a code path that ignored the trial root would still write only there.
    """
    windows = (os.name == "nt") if windows is None else windows
    host = os.environ if host is None else host
    home, state = root / "home", root / "state"
    vendor = state / "vendor-home"
    env = {
        "HOME": str(home), "USERPROFILE": str(home),
        "USERNAME": "synthetic", "USER": "synthetic", "LOGNAME": "synthetic",
        "XDG_CONFIG_HOME": str(vendor / "config"), "XDG_CACHE_HOME": str(vendor / "cache"),
        "TMPDIR": str(state / "tmp"), "TEMP": str(state / "tmp"), "TMP": str(state / "tmp"),
        # Skips the local-address probe used to label the Hub in /api/info.
        "MANY_AI_CLI_HOST_LABEL": "synthetic-isolated",
    }
    env.update({name: str(vendor / name.lower()) for name in PROFILE_VARIABLES})
    if windows:
        windir = host.get("SystemRoot") or host.get("WINDIR") or r"C:\Windows"
        system32 = str(Path(windir) / "System32")
        drive, tail = os.path.splitdrive(str(home))
        env.update({
            "SystemRoot": windir, "WINDIR": windir,
            "SystemDrive": os.path.splitdrive(windir)[0] or "C:",
            "COMSPEC": str(Path(system32) / "cmd.exe"),
            "PATH": os.pathsep.join((system32, windir)), "PATHEXT": ".COM;.EXE;.BAT;.CMD",
            "HOMEDRIVE": drive, "HOMEPATH": tail,
            "APPDATA": str(vendor / "appdata"), "LOCALAPPDATA": str(vendor / "localappdata"),
        })
    else:
        env.update({"PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "SHELL": "/bin/sh"})
    return env


def remove_tree(path: Path, attempts: int = 3, pause: float = 0.5) -> bool:
    def force(function, target, *_):
        try:
            os.chmod(target, stat.S_IREAD | stat.S_IWRITE | stat.S_IEXEC)
            function(target)
        except OSError:
            pass
    options = {"onexc": force} if sys.version_info >= (3, 12) else {"onerror": force}
    for attempt in range(attempts):
        if not path.exists() and not path.is_symlink():
            return True
        shutil.rmtree(path, **options)
        if not path.exists():
            return True
        if attempt + 1 < attempts:
            time.sleep(pause)
    return not path.exists()


def inventory(path: Path) -> dict:
    files = directories = 0
    for _, children, names in os.walk(path, followlinks=False):
        directories += len(children)
        files += len(names)
    return {"files": files, "directories": directories}


# ----------------------------------------------------------- real-world adapters

def _process_options() -> dict:
    if os.name == "nt":
        return {"creationflags": getattr(subprocess, "CREATE_NO_WINDOW", 0)}
    return {"start_new_session": True}  # own process group: one signal reaches descendants


def _signal_group(process: subprocess.Popen, number: int) -> None:
    try:
        os.killpg(process.pid, number)
    except (ProcessLookupError, PermissionError):
        pass


class ProcessHandle:
    """A spawned process that is reaped the moment it exits.

    On Unix the product's own `stop` decides the Hub is gone with kill(pid, 0), which is
    still true for an exited child nobody has waited for. A reaper thread keeps `stop`
    from waiting on a zombie of ours.
    """

    def __init__(self, process: subprocess.Popen, stderr_file):
        self._process = process
        self._stderr_file = stderr_file
        self._done = threading.Event()
        self._code: int | None = None
        self.pid = process.pid
        threading.Thread(target=self._reap, daemon=True).start()

    def _reap(self) -> None:
        self._code = self._process.wait()
        self._done.set()

    def poll(self) -> int | None:
        return self._code if self._done.is_set() else None

    def wait(self, timeout: float) -> int | None:
        return self._code if self._done.wait(timeout) else None

    def terminate(self) -> None:
        if os.name == "nt":
            self._process.terminate()
        else:
            _signal_group(self._process, signal.SIGTERM)

    def kill(self) -> None:
        if os.name == "nt":
            self._process.kill()
        else:
            _signal_group(self._process, signal.SIGKILL)

    def close(self) -> None:
        self._stderr_file.close()


class SubprocessExecutor:
    def run(self, argv: list[str], env: dict[str, str], cwd: Path, timeout: float) -> CommandResult:
        process = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   **_process_options())
        try:
            out, err = process.communicate(timeout=timeout)
            return CommandResult(process.returncode, out, err)
        except subprocess.TimeoutExpired:
            if os.name == "nt":
                process.kill()
            else:
                _signal_group(process, signal.SIGKILL)
            try:
                out, err = process.communicate(timeout=5)
            except (subprocess.TimeoutExpired, OSError, ValueError):
                out = err = b""
            return CommandResult(None, out, err, True)

    def spawn(self, argv: list[str], env: dict[str, str], cwd: Path, stderr_path: Path) -> ProcessHandle:
        stderr_file = stderr_path.open("wb")
        try:
            process = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                       stdout=subprocess.DEVNULL, stderr=stderr_file,
                                       **_process_options())
        except BaseException:
            stderr_file.close()
            raise
        return ProcessHandle(process, stderr_file)


class Clock:
    def now(self) -> float:
        return time.monotonic()

    def sleep(self, seconds: float) -> None:
        time.sleep(seconds)


def pick_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.bind((LOOPBACK, 0))
        return probe.getsockname()[1]


def port_is_open(port: int) -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.settimeout(1.0)
        return probe.connect_ex((LOOPBACK, port)) == 0


def fetch_info_http(port: int, token: str, timeout: float = INFO_SECONDS) -> dict:
    """GET /api/info on the numeric loopback address: no proxy, DNS, redirect or cookies."""
    connection = http.client.HTTPConnection(LOOPBACK, port, timeout=timeout)
    try:
        connection.request("GET", "/api/info",
                           headers={"Authorization": "Bearer " + token, "Connection": "close"})
        response = connection.getresponse()
        if response.status != 200:
            raise SmokeFailure("info_http_status", {"status": response.status})
        body = response.read(READ_CAP + 1)
    finally:
        connection.close()
    if len(body) > READ_CAP:
        raise SmokeFailure("info_too_large")
    try:
        value = json.loads(body)
    except ValueError:
        raise SmokeFailure("info_invalid_json") from None
    if not isinstance(value, dict):
        raise SmokeFailure("info_invalid_json")
    return value


# ------------------------------------------------------------------------ smoke

class Smoke:
    def __init__(self, config: SmokeConfig, executor, clock, *, fetch_info=fetch_info_http,
                 pick_port=pick_free_port, port_open=port_is_open,
                 log: Callable[[str], None] = lambda message: None):
        self.config = config
        self.executor = executor
        self.clock = clock
        self.fetch_info = fetch_info
        self.pick_port = pick_port
        self.port_open = port_open
        self.log = log
        self.checks: list[dict] = []
        self.failure: dict | None = None
        self.hub = None
        self.root: Path | None = None
        self.exe: Path | None = None
        self.port = 0
        self.env: dict[str, str] = {}
        self._token: str | None = None
        self._started = 0.0

    # --- plumbing ---------------------------------------------------------
    def _secrets(self) -> tuple[str, ...]:
        if self._token is None and self.root is not None:
            self._token = read_token(self.root / "state" / "config.yaml")
        return (self._token,) if self._token else ()

    def _clean(self, text: str) -> str:
        return redact(text, self._secrets())

    def _tail(self, data: bytes) -> str:
        return tail_text(data, self._secrets())

    def _remaining(self) -> float:
        return self._started + self.config.total_seconds - self.clock.now()

    def _argv(self, *tail: str) -> list[str]:
        return [str(self.exe), "--trial-root", str(self.root / "state"),
                "--trial-port", str(self.port), *tail]

    def _run(self, tail: tuple[str, ...], limit: float) -> CommandResult:
        remaining = self._remaining()
        if remaining <= 0:
            raise SmokeFailure("deadline_exceeded", {"limit_seconds": self.config.total_seconds})
        try:
            return self.executor.run(self._argv(*tail), self.env, self.root / "state" / "work",
                                     min(limit, remaining))
        except OSError as error:
            raise SmokeFailure("exec_failed", {"error": type(error).__name__}) from None

    def _require_ok(self, result: CommandResult, what: str) -> None:
        if result.timed_out or result.code != 0:
            raise SmokeFailure(what + ("_timed_out" if result.timed_out else "_exit_nonzero"),
                               {"exit_code": result.code, "timed_out": result.timed_out,
                                "stderr_tail": self._tail(result.stderr)})

    def _status(self, limit: float = STATUS_SECONDS) -> tuple[CommandResult, dict]:
        result = self._run(("status",), limit)
        return result, (parse_status(result.stdout) if result.code == 0 else {"kind": "error"})

    def _serve_tail(self) -> str:
        path = self.root / "serve.stderr.log" if self.root else None
        try:
            return self._tail(path.read_bytes()[:READ_CAP]) if path and path.is_file() else ""
        except OSError:
            return ""

    def _check(self, name: str, function: Callable[[], dict | None]) -> None:
        if self.failure is not None:
            return
        started = self.clock.now()
        try:
            detail, status = function() or {}, "passed"
        except SmokeFailure as error:
            detail, status = {"code": error.code, **error.detail}, "failed"
            self.failure = {"check": name, "code": error.code}
        except Exception as error:  # a harness defect must still stop and clean up
            detail, status = {"code": "internal_error", "error": type(error).__name__}, "failed"
            self.failure = {"check": name, "code": "internal_error"}
        seconds = round(self.clock.now() - started, 3)
        self.checks.append({"id": name, "status": status, "seconds": seconds, "detail": detail})
        self.log(f"[smoke] {name}: {status} ({seconds}s)"
                 + (f" code={self.failure['code']}" if status == "failed" else ""))

    # --- checks -----------------------------------------------------------
    def _prepare(self) -> dict:
        config = self.config
        if digest(config.binary) != config.binary_sha256:
            raise SmokeFailure("binary_hash_mismatch")
        self.root = Path(os.path.realpath(tempfile.mkdtemp(prefix="many-ai-cli-smoke-",
                                                           dir=config.temp_parent)))
        home, state = self.root / "home", self.root / "state"
        (self.root / "bin").mkdir()
        home.mkdir()
        state.mkdir()
        if os.name != "nt":
            os.chmod(state, 0o700)
        vendor = state / "vendor-home"
        for path in (state / "work", state / "tmp", vendor,
                     *(vendor / name for name in ("appdata", "localappdata", "config", "cache")),
                     *(vendor / name.lower() for name in PROFILE_VARIABLES)):
            path.mkdir()
        self.exe = self.root / "bin" / config.binary.name
        shutil.copy2(config.binary, self.exe)
        if os.name != "nt":
            os.chmod(self.exe, 0o755)
        if digest(self.exe) != config.binary_sha256:
            raise SmokeFailure("binary_copy_mismatch")
        self.port = self.pick_port()
        if self.port == DEFAULT_PORT or not 0 < self.port < 65536:
            raise SmokeFailure("port_refused")
        self.env = isolated_environment(self.root)
        return {"port": self.port}

    def _version(self) -> dict:
        result = self._run(("version",), VERSION_SECONDS)
        self._require_ok(result, "version")
        observed = result.stdout.decode("utf-8", "replace").strip()
        if observed != self.config.expected_version:
            raise SmokeFailure("version_mismatch", {"observed": self._clean(observed)[:200],
                                                    "expected": self.config.expected_version})
        return {"version": observed}

    def _status_stopped(self) -> dict:
        result, parsed = self._status()
        self._require_ok(result, "status")
        if parsed["kind"] != "stopped":
            raise SmokeFailure("unexpected_status", {"kind": parsed["kind"]})
        return {"status": "stopped"}

    def _serve_start(self) -> dict:
        try:
            self.hub = self.executor.spawn(self._argv("serve", "--open=false", "--strict-port"),
                                           self.env, self.root / "state" / "work",
                                           self.root / "serve.stderr.log")
        except OSError as error:
            raise SmokeFailure("exec_failed", {"error": type(error).__name__}) from None
        started = self.clock.now()
        deadline = started + min(START_SECONDS, max(self._remaining(), 0.0))
        polls = timeouts = 0
        last = None
        while True:
            code = self.hub.poll()
            if code is not None:
                raise SmokeFailure("serve_exited_early", {"exit_code": code, "status_polls": polls,
                                                          "stderr_tail": self._serve_tail()})
            result, parsed = self._status()
            polls += 1
            timeouts += bool(result.timed_out)
            last = result
            if parsed["kind"] == "running":
                return {"status_polls": polls, "ready_after_seconds": round(self.clock.now() - started, 3)}
            if self.clock.now() >= deadline:
                raise SmokeFailure("serve_not_ready", {
                    "status_polls": polls, "status_timeouts": timeouts,
                    "last_status_exit": last.code, "last_status_stderr": self._tail(last.stderr),
                    "stderr_tail": self._serve_tail()})
            self.clock.sleep(POLL_SECONDS)

    def _ledger(self) -> dict:
        value = read_ledger(self.root / "state" / "hub-runtime.json")
        if value.get("version") != 1 or value.get("pid") != self.hub.pid or value.get("port") != self.port:
            raise SmokeFailure("ledger_mismatch", {
                "ledger_version": value.get("version"), "pid_matches": value.get("pid") == self.hub.pid,
                "port_matches": value.get("port") == self.port})
        return {"pid_matches_spawned_process": True, "port_matches": True}

    def _status_running(self) -> dict:
        result, parsed = self._status()
        self._require_ok(result, "status")
        if parsed["kind"] != "running":
            raise SmokeFailure("unexpected_status", {"kind": parsed["kind"]})
        if (parsed["scheme"], parsed["host"], parsed["port"]) != ("http", LOOPBACK, self.port):
            raise SmokeFailure("status_endpoint_mismatch", {
                "scheme": parsed["scheme"], "host": parsed["host"], "port_matches": parsed["port"] == self.port})
        if parsed["stale_warning"]:
            raise SmokeFailure("stale_binary_warning")
        return {"status": "running", "endpoint": f"http://{LOOPBACK}:<trial port>/"}

    def _api_info(self) -> dict:
        token = read_token(self.root / "state" / "config.yaml")
        if token is None:
            raise SmokeFailure("token_unreadable")
        self._token = token
        try:
            info = self.fetch_info(self.port, token)
        except SmokeFailure:
            raise
        except (OSError, ValueError) as error:
            raise SmokeFailure("info_request_failed", {"error": type(error).__name__}) from None
        expected = {"version": self.config.expected_version, "git_commit": self.config.expected_commit,
                    "build_time": self.config.expected_build_time,
                    "binary_sha256": self.config.binary_sha256,
                    "runtime_mode": expected_runtime_mode(self.config.target)}
        bad = compare_info(info, expected)
        if bad:
            raise SmokeFailure("info_mismatch", {
                "fields": bad,
                "observed": {key: self._clean(str(info.get(key)))[:80] for key in (*expected, "binary_stale", "web_dist_fresh")}})
        return {"fields_verified": [*expected, "binary_stale", "web_dist_fresh"],
                "web_src_hash": self._clean(str(info.get("web_src_hash")))[:16],
                "active_sessions": info.get("active_sessions")}

    def _reuse(self) -> dict:
        ledger = self.root / "state" / "hub-runtime.json"
        before = read_ledger(ledger).get("pid")
        result = self._run((), REUSE_SECONDS)
        self._require_ok(result, "reuse")
        after = read_ledger(ledger).get("pid")
        if before != after or self.hub.poll() is not None:
            raise SmokeFailure("hub_changed_by_default_command",
                               {"pid_unchanged": before == after, "hub_alive": self.hub.poll() is None})
        _, parsed = self._status()
        if parsed["kind"] != "running":
            raise SmokeFailure("hub_not_running_after_reuse", {"kind": parsed["kind"]})
        return {"pid_unchanged": True}

    def _stop(self) -> dict:
        result = self._run(("stop",), STOP_SECONDS)
        self._require_ok(result, "stop")
        code = self.hub.wait(min(EXIT_SECONDS, max(self._remaining(), 1.0)))
        if code is None:
            raise SmokeFailure("serve_still_running_after_stop")
        if code != 0:
            raise SmokeFailure("serve_exit_nonzero", {"exit_code": code, "stderr_tail": self._serve_tail()})
        return {"serve_exit_code": code}

    def _stopped_after(self) -> dict:
        result, parsed = self._status()
        self._require_ok(result, "status")
        if parsed["kind"] != "stopped":
            raise SmokeFailure("unexpected_status", {"kind": parsed["kind"]})
        if self.port_open(self.port):
            raise SmokeFailure("port_still_listening")
        if (self.root / "state" / "hub-runtime.json").exists():
            raise SmokeFailure("ledger_remains")
        return {"status": "stopped", "port_closed": True, "ledger_removed": True}

    def _binary_unchanged(self) -> dict:
        if digest(self.exe) != self.config.binary_sha256:
            raise SmokeFailure("binary_changed")
        return {"sha256_unchanged": True}

    # --- run / cleanup ------------------------------------------------------
    def run(self) -> dict:
        self._started = self.clock.now()
        cleanup: dict = {}
        isolation: dict = {}
        serve: dict = {}
        try:
            for name, function in (
                    ("prepare", self._prepare), ("version", self._version),
                    ("status-stopped-before", self._status_stopped), ("serve-start", self._serve_start),
                    ("ledger", self._ledger), ("status-running", self._status_running),
                    ("api-info", self._api_info), ("reuse-existing-hub", self._reuse),
                    ("stop", self._stop), ("stopped-after", self._stopped_after),
                    ("binary-unchanged", self._binary_unchanged)):
                self._check(name, function)
        except BaseException as error:  # interrupted (for example SIGTERM): clean up, then record
            self.failure = self.failure or {"check": "run", "code": "interrupted",
                                            "error": type(error).__name__}
        finally:
            cleanup = self._cleanup(serve, isolation)
        passed = self.failure is None and cleanup["hub_stopped"]
        if self.failure is None and not cleanup["hub_stopped"]:
            self.failure = {"check": "cleanup", "code": "hub_still_running"}
        done = {check["id"] for check in self.checks}
        checks = [*self.checks, *({"id": name, "status": "not-run"} for name in CHECK_IDS if name not in done)]
        return {
            "schema_version": SCHEMA_VERSION,
            "status": "passed" if passed else "failed",
            "target": self.config.target,
            "platform": sys.platform,
            "method": ("trial runtime (--trial-root/--trial-port) in a fresh temporary directory with HOME, "
                       "USERPROFILE, APPDATA, XDG_* and TMP redirected into it; serve --open=false "
                       "--strict-port; status; default command against the running Hub; stop; status"),
            "binary": {"name": self.config.binary.name, "sha256": self.config.binary_sha256},
            "expected": {"version": self.config.expected_version, "git_commit": self.config.expected_commit,
                         "build_time": self.config.expected_build_time,
                         "runtime_mode": expected_runtime_mode(self.config.target)},
            "port": self.port or None,
            "checks": checks,
            "failure": self.failure,
            "serve": serve,
            "isolation": isolation,
            "cleanup": cleanup,
            "total_seconds": round(self.clock.now() - self._started, 3),
            "limits": list(LIMITS),
        }

    def _cleanup(self, serve: dict, isolation: dict) -> dict:
        result = {"hub_started": self.hub is not None, "graceful_stop_attempted": False,
                  "forced_termination": False, "hub_stopped": True, "root_removed": None}
        hub = self.hub
        try:
            if hub is not None and hub.poll() is None:
                result["graceful_stop_attempted"] = True
                try:
                    self.executor.run(self._argv("stop"), self.env, self.root / "state" / "work",
                                      CLEANUP_STOP_SECONDS)
                except BaseException:  # a second interrupt must not skip the forced stop below
                    pass
                if hub.wait(CLEANUP_SIGNAL_SECONDS) is None:
                    result["forced_termination"] = True
                    hub.terminate()
                    if hub.wait(CLEANUP_SIGNAL_SECONDS) is None:
                        hub.kill()
                        hub.wait(CLEANUP_SIGNAL_SECONDS)
            if hub is not None:
                result["hub_stopped"] = hub.poll() is not None
                serve.update({"pid": hub.pid, "exit_code": hub.poll()})
                try:
                    data = (self.root / "serve.stderr.log").read_bytes()[:READ_CAP]
                except OSError:
                    data = b""
                if data:
                    serve["stderr"] = summarize_hub_log(data, self._secrets())
                    # Text only when something failed: it is the diagnosis, and a passing
                    # receipt (also shipped with release bundles) needs no log excerpts.
                    if self.failure is not None or not result["hub_stopped"]:
                        serve["stderr_tail"] = self._tail(data)
                hub.close()
        except Exception as error:
            result["cleanup_error"] = type(error).__name__
            result["hub_stopped"] = hub is None or hub.poll() is not None
        if self.root is not None:
            try:
                isolation["home_after"] = inventory(self.root / "home")
            except OSError:
                pass
            result["root_removed"] = remove_tree(self.root) if result["hub_stopped"] else False
        return result


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=sorted(TARGET_MODES), required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--binary-sha256", required=True)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--expected-commit", required=True)
    parser.add_argument("--expected-build-time", required=True)
    parser.add_argument("--result", type=Path, required=True)
    return parser


def _interrupt(*_) -> None:
    raise KeyboardInterrupt


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if os.name != "nt":
        # A terminate request from the caller must still run the cleanup.
        signal.signal(signal.SIGTERM, _interrupt)
    config = SmokeConfig(target=args.target, binary=args.binary.resolve(),
                         binary_sha256=args.binary_sha256, expected_version=args.expected_version,
                         expected_commit=args.expected_commit,
                         expected_build_time=args.expected_build_time)
    record = Smoke(config, SubprocessExecutor(), Clock(), log=lambda m: print(m, flush=True)).run()
    args.result.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    print(f"[smoke] {record['status']} ({record['total_seconds']}s)"
          + (f" failed at {record['failure']['check']}: {record['failure']['code']}" if record["failure"] else ""),
          flush=True)
    return 0 if record["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
