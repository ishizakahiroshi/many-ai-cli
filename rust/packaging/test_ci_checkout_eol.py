"""Real Git checkout byte contracts; no product builds, network or user config."""
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = (
    ("rust-migration.yml", "native"),
    ("release.yml", "native"),
    ("rust-windows-check.yml", "windows-check"),
)
TEXT_PATHS = ("rust/Cargo.lock", "rust/assets/launcher/ui/index.html", "web/src/app.ts")
PAYLOAD = b"first line\nsecond line\n"


def checkout_preflight(filename, job):
    """Read the actual job's run block immediately preceding its first checkout."""
    source = (ROOT / ".github/workflows" / filename).read_text(encoding="utf-8")
    job_source = source.split(f"\n  {job}:\n", 1)[1]
    job_source = re.split(r"\n  [A-Za-z0-9_-]+:\n", job_source, maxsplit=1)[0]
    before_checkout = job_source.split("      - uses: actions/checkout@", 1)[0]
    match = re.search(r"^        run: (.+)\n", before_checkout, re.M)
    if match is None:
        raise AssertionError(f"{filename}: source-byte preflight missing before checkout")
    shell = re.findall(r"^        shell: (\w+)$", before_checkout, re.M)[-1]
    if match.group(1) == "|":
        block = re.match(r"(?:          [^\n]*\n)+", before_checkout[match.end():])
        script = "\n".join(line[10:] for line in block.group(0).splitlines()) + "\n"
    else:
        script = match.group(1) + "\n"
    return shell, script


class CheckoutEolTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="ci-checkout-eol-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        self.env.update({"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_SYSTEM": os.devnull,
                         "GIT_CONFIG_GLOBAL": str(self.root / "global.gitconfig"),
                         "GITHUB_OUTPUT": str(self.root / "github-output")})
        self.git("init", "--quiet")
        self.git("config", "core.hooksPath", str(self.root / "no-hooks"))
        self.git("config", "--global", "core.autocrlf", "false")
        self.git("config", "--global", "core.eol", "lf")
        (self.repo / ".gitattributes").write_bytes(
            b"* text=auto\n*.ps1 text eol=crlf\n*.rs text eol=lf\n")
        for relative in (*TEXT_PATHS, "script.ps1", "source.rs"):
            path = self.repo / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(PAYLOAD)
        self.git("add", "--all")
        self.git("-c", "user.name=Synthetic Test", "-c", "user.email=fixture@example.com",
                 "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "synthetic checkout fixture")
        for relative in TEXT_PATHS:
            self.assertEqual(self.git("show", f"HEAD:{relative}"), PAYLOAD)

    def git(self, *arguments):
        result = subprocess.run(["git", *arguments], cwd=self.repo, env=self.env,
                                capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        return result.stdout

    def checkout(self):
        # Force real checkout conversion again instead of trusting existing stat data.
        for relative in (*TEXT_PATHS, "script.ps1", "source.rs"):
            (self.repo / relative).unlink()
        self.git("checkout-index", "--all", "--force")

    def test_autocrlf_false_alone_retains_the_crlf_counterexample(self):
        self.git("config", "--global", "core.eol", "crlf")
        self.git("config", "--global", "core.autocrlf", "false")
        self.checkout()
        for relative in TEXT_PATHS:
            self.assertEqual((self.repo / relative).read_bytes(), PAYLOAD.replace(b"\n", b"\r\n"))
            self.assertNotEqual((self.repo / relative).read_bytes(), self.git("show", f"HEAD:{relative}"))

    def test_actual_workflow_preflights_override_crlf_and_preserve_explicit_attributes(self):
        for filename, job in WORKFLOWS:
            with self.subTest(workflow=filename):
                self.git("config", "--global", "core.eol", "crlf")
                self.git("config", "--global", "core.autocrlf", "true")
                shell, script = checkout_preflight(filename, job)
                if shell == "bash":
                    executable = r"C:\Program Files\Git\bin\bash.exe" if os.name == "nt" else "/bin/bash"
                    command = [executable, "--noprofile", "--norc", "-eo", "pipefail", "-c", script]
                    result = subprocess.run(command, cwd=self.repo, env=self.env, capture_output=True, timeout=15)
                    self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
                elif os.name == "nt":
                    result = subprocess.run(["pwsh", "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script],
                                            cwd=self.repo, env=self.env, capture_output=True, timeout=15)
                    self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
                else:
                    # The Windows-only shell is exercised on Windows. Other native
                    # runners still verify its actual Git arguments and checkout bytes.
                    for line in script.splitlines():
                        if line.startswith("git "):
                            self.git(*shlex.split(line)[1:])
                self.checkout()
                for relative in TEXT_PATHS:
                    self.assertEqual((self.repo / relative).read_bytes(), self.git("show", f"HEAD:{relative}"))
                self.assertEqual(self.git("config", "--global", "core.autocrlf").strip(), b"false")
                self.assertEqual(self.git("config", "--global", "core.eol").strip(), b"lf")
                self.assertEqual((self.repo / "script.ps1").read_bytes(), PAYLOAD.replace(b"\n", b"\r\n"))
                self.assertEqual((self.repo / "source.rs").read_bytes(), PAYLOAD)

    def test_windows_preflight_propagates_each_git_failure(self):
        shell, script = checkout_preflight("rust-windows-check.yml", "windows-check")
        self.assertEqual(shell, "pwsh")
        commands = [line for line in script.splitlines() if line.startswith("git ")]
        self.assertEqual(len(commands), 2)
        for command in commands:
            self.assertIn(command + "\nif ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }", script)
        if os.name == "nt":
            # Inject a real native Git config failure at either command, without
            # changing the workflow block or reading/writing user configuration.
            failure = subprocess.run(["git", "config", "--file", str(self.root), "core.eol", "lf"],
                                     cwd=self.repo, env=self.env, capture_output=True, timeout=10)
            self.assertNotEqual(failure.returncode, 0)
            for position in (1, 2):
                with self.subTest(failed_git_command=position):
                    calls = self.root / "git-calls"
                    calls.write_bytes(b"")
                    failed_env = self.env | {"FIXTURE_GIT": shutil.which("git"),
                                             "FIXTURE_CONFIG": str(self.root),
                                             "FIXTURE_CALLS": str(calls), "FIXTURE_FAIL_AT": str(position)}
                    injection = """
$global:fixtureCalls = 0
function git {
    $global:fixtureCalls++
    Add-Content -LiteralPath $env:FIXTURE_CALLS -Value $global:fixtureCalls
    if ($global:fixtureCalls -eq [int]$env:FIXTURE_FAIL_AT) {
        & $env:FIXTURE_GIT config --file $env:FIXTURE_CONFIG core.eol lf
    } else {
        & $env:FIXTURE_GIT @args
    }
}
"""
                    result = subprocess.run(["pwsh", "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", injection + script],
                                            cwd=self.repo, env=failed_env, capture_output=True, timeout=15)
                    self.assertEqual(result.returncode, failure.returncode)
                    self.assertEqual(len(calls.read_text(encoding="utf-8-sig").splitlines()), position)


if __name__ == "__main__":
    unittest.main()
