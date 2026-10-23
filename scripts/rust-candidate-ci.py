#!/usr/bin/env python3
"""Clean native build-only receipts for #3. Never publishes/releases or installs candidates."""
from __future__ import annotations
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "x86_64-unknown-linux-gnu": "linux-x64",
    "x86_64-pc-windows-msvc": "windows-x64",
    "x86_64-apple-darwin": "macos-intel",
    "aarch64-apple-darwin": "macos-apple-silicon",
}
WINDOWS_RUNTIME_NAMES = ('vcomp140.dll', 'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')
PACKAGING_INPUTS = ('BUILD-INPUTS.json', 'CRATE-NOTICES.txt', 'build-inputs.spdx.json')

def release_version(tag: str) -> str:
    if not isinstance(tag, str):
        raise ValueError('release tag must be strict SemVer with a v prefix')
    match = re.fullmatch(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)'
                         r'(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?', tag)
    if not match or (match[4] and any(part.isdigit() and len(part) > 1 and part[0] == '0'
                                    for part in match[4].split('.'))):
        raise ValueError('release tag must be strict SemVer with a v prefix')
    return tag[1:]

def build_stamp(source: str, release_tag: str | None) -> dict:
    if not re.fullmatch(r'[0-9a-f]{40}', source):
        raise ValueError('invalid source revision')
    if release_tag is None:
        return {'build_kind': 'candidate', 'release_tag': None, 'build_commit': source,
                'version': f'0.0.0-rust-candidate+{source[:12]}',
                'build_time': datetime.datetime.now(datetime.timezone.utc).isoformat()}
    version = release_version(release_tag)
    verify_release_source(release_tag, source)
    timestamp = capture(['git', 'show', '-s', '--format=%ct', f'refs/tags/{release_tag}^{{commit}}'], ROOT)
    if not re.fullmatch(r'[0-9]+', timestamp):
        raise ValueError('release source commit time is invalid')
    return {'build_kind': 'release', 'release_tag': release_tag, 'build_commit': source,
            'version': version,
            'build_time': datetime.datetime.fromtimestamp(int(timestamp), datetime.timezone.utc).isoformat()}

def verify_release_source(tag: str, source: str) -> None:
    release_version(tag)
    if capture(['git', 'rev-parse', '--verify', f'refs/tags/{tag}^{{commit}}'], ROOT) != source:
        raise ValueError('release tag commit does not match HEAD/source')

def windows_resource_script(version: str, icon: Path, filename: str) -> str:
    release_version('v' + version)
    numbers = version.split('-', 1)[0].split('+', 1)[0].split('.')
    if any(int(part) > 65535 for part in numbers):
        raise ValueError('Windows resource version components must fit unsigned 16-bit fields')
    if filename not in ('many-ai-cli.exe', 'many-ai-cli-launcher.exe') or any(c in icon.as_posix() for c in '\"\r\n'):
        raise ValueError('invalid Windows resource filename or icon path')
    numeric = ','.join([*numbers, '0'])
    prerelease = '-' in version.split('+', 1)[0]
    return f'''#pragma code_page(65001)
1 ICON "{icon.as_posix()}"
1 VERSIONINFO
 FILEVERSION {numeric}
 PRODUCTVERSION {numeric}
 FILEFLAGSMASK 0x3fL
 FILEFLAGS {2 if prerelease else 0}
 FILEOS 0x40004L
 FILETYPE 0x1L
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904B0"
  BEGIN
   VALUE "FileDescription", "many-ai-cli{' launcher' if 'launcher' in filename else ''}\\0"
   VALUE "FileVersion", "{version}\\0"
   VALUE "ProductName", "many-ai-cli\\0"
   VALUE "ProductVersion", "{version}\\0"
   VALUE "OriginalFilename", "{filename}\\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x409, 1200
 END
END
'''

def prepare_windows_resources(output: Path, version: str, env: dict[str, str]) -> dict:
    compiler = shutil.which('rc.exe')
    if not compiler:
        sdk = Path(env.get('ProgramFiles(x86)', r'C:\Program Files (x86)')) / 'Windows Kits' / '10' / 'bin'
        compilers = sorted(sdk.glob('*/x64/rc.exe'), reverse=True)
        compiler = str(compilers[0]) if compilers else None
    if not compiler:
        raise ValueError('Windows SDK resource compiler rc.exe is required for release builds')
    icon = ROOT / 'assets' / 'many-ai-cli.ico'
    receipt = {'version': version, 'icon_sha256': digest(icon), 'resources': []}
    output.mkdir()
    for stem in ('many-ai-cli', 'many-ai-cli-launcher'):
        source, resource = output / f'{stem}.rc', output / f'{stem}.res'
        source.write_text(windows_resource_script(version, icon, stem + '.exe'), encoding='utf-8')
        run([compiler, '/nologo', '/fo', str(resource), str(source)], env)
        receipt['resources'].append({'name': resource.name, 'sha256': digest(resource)})
    return receipt

def windows_file_version(binary: Path) -> dict[str, str]:
    # Read Windows' actual VERSIONINFO resource rather than executing either binary.
    import ctypes
    from ctypes import wintypes
    api = ctypes.WinDLL('version', use_last_error=True)
    api.GetFileVersionInfoSizeW.argtypes = [wintypes.LPCWSTR, ctypes.POINTER(wintypes.DWORD)]
    api.GetFileVersionInfoSizeW.restype = wintypes.DWORD
    api.GetFileVersionInfoW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p]
    api.GetFileVersionInfoW.restype = wintypes.BOOL
    api.VerQueryValueW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR, ctypes.POINTER(ctypes.c_void_p), ctypes.POINTER(wintypes.UINT)]
    api.VerQueryValueW.restype = wintypes.BOOL
    handle = wintypes.DWORD()
    size = api.GetFileVersionInfoSizeW(str(binary), ctypes.byref(handle))
    if not size:
        raise ValueError('Windows binary has no readable version resource')
    data = ctypes.create_string_buffer(size)
    if not api.GetFileVersionInfoW(str(binary), 0, size, data):
        raise ValueError('Windows binary version resource readback failed')
    result = {}
    for key in ('FileVersion', 'ProductVersion', 'OriginalFilename'):
        value, length = ctypes.c_void_p(), wintypes.UINT()
        if not api.VerQueryValueW(data, f'\\StringFileInfo\\040904B0\\{key}', ctypes.byref(value), ctypes.byref(length)) or not length.value:
            raise ValueError('Windows binary version resource field is missing')
        result[key] = ctypes.wstring_at(value.value, length.value).rstrip('\0')
    return result

def windows_icon_resources(binary: Path) -> list[tuple[tuple, bytes]]:
    """Read group 1 and its actual RT_ICON payloads without executing the binary."""
    import ctypes
    from ctypes import wintypes
    api = ctypes.WinDLL('kernel32', use_last_error=True)
    api.LoadLibraryExW.argtypes = [wintypes.LPCWSTR, wintypes.HANDLE, wintypes.DWORD]
    api.LoadLibraryExW.restype = wintypes.HMODULE
    api.FindResourceW.argtypes = [wintypes.HMODULE, ctypes.c_void_p, ctypes.c_void_p]
    api.FindResourceW.restype = wintypes.HANDLE
    api.SizeofResource.argtypes = [wintypes.HMODULE, wintypes.HANDLE]
    api.SizeofResource.restype = wintypes.DWORD
    api.LoadResource.argtypes = [wintypes.HMODULE, wintypes.HANDLE]
    api.LoadResource.restype = wintypes.HANDLE
    api.LockResource.argtypes = [wintypes.HANDLE]
    api.LockResource.restype = ctypes.c_void_p
    api.FreeLibrary.argtypes = [wintypes.HMODULE]
    api.FreeLibrary.restype = wintypes.BOOL
    # LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE prevents code execution.
    module = api.LoadLibraryExW(str(binary.resolve()), None, 0x00000022)
    if not module:
        raise ValueError('Windows icon resource module could not be opened')
    try:
        def resource(kind: int, identifier: int) -> bytes:
            found = api.FindResourceW(module, ctypes.c_void_p(identifier), ctypes.c_void_p(kind))
            if not found:
                raise ValueError('Windows group/icon resource is missing')
            size = api.SizeofResource(module, found)
            handle = api.LoadResource(module, found)
            pointer = api.LockResource(handle) if handle else None
            if not size or not pointer:
                raise ValueError('Windows group/icon resource could not be read')
            return ctypes.string_at(pointer, size)
        group = resource(14, 1)  # RT_GROUP_ICON, same ID as the generated 1 ICON input.
        if len(group) < 6 or group[:4] != b'\0\0\1\0':
            raise ValueError('Windows group icon directory is invalid')
        count = struct.unpack_from('<H', group, 4)[0]
        if not count or len(group) != 6 + count * 14:
            raise ValueError('Windows group icon entry count is invalid')
        images = []
        for index in range(count):
            entry = struct.unpack_from('<BBBBHHIH', group, 6 + index * 14)
            if entry[3] != 0 or not entry[6] or not entry[7]:
                raise ValueError('Windows group icon entry is invalid')
            payload = resource(3, entry[7])  # RT_ICON resolved by the group's nID.
            if len(payload) != entry[6]:
                raise ValueError('Windows icon resource size differs from its group entry')
            images.append((entry[:7], payload))
        return images
    finally:
        api.FreeLibrary(module)

def windows_icon_images_match(actual: list, expected: list) -> bool:
    # SDK rc.exe changes zero ICO planes to one in both PNG and DIB group entries.
    # Preserve exact matches and allow only that change; all other fields and the
    # actual referenced RT_ICON payloads must still match, including image count.
    return len(actual) == len(expected) and all(
        payload == wanted_payload and (entry == wanted_entry or (
            wanted_entry[4] == 0 and entry == wanted_entry[:4] + (1,) + wanted_entry[5:]))
        for (entry, payload), (wanted_entry, wanted_payload) in zip(actual, expected))

def verify_windows_resources(output: Path, version: str, prepared: dict) -> dict:
    icon = (ROOT / 'assets' / 'many-ai-cli.ico').read_bytes()
    if len(icon) < 6 or icon[:4] != b'\0\0\1\0':
        raise ValueError('release icon must be an ICO file')
    count = struct.unpack_from('<H', icon, 4)[0]
    if not count or len(icon) < 6 + count * 16 or digest(ROOT / 'assets' / 'many-ai-cli.ico') != prepared['icon_sha256']:
        raise ValueError('release icon directory changed or is invalid')
    images = []
    for index in range(count):
        entry = struct.unpack_from('<BBBBHHII', icon, 6 + index * 16)
        size, offset = entry[6:]
        if not size or offset < 6 + count * 16 or offset + size > len(icon):
            raise ValueError('release icon payload bounds are invalid')
        images.append((entry[:7], icon[offset:offset + size]))
    binaries = []
    for stem in ('many-ai-cli', 'many-ai-cli-launcher'):
        binary = output / (stem + '.exe')
        observed = windows_file_version(binary)
        if (observed['FileVersion'] != version or observed['ProductVersion'] != version
                or observed['OriginalFilename'] != binary.name
                or not windows_icon_images_match(windows_icon_resources(binary), images)):
            raise ValueError('Windows binary product version/icon readback mismatch')
        binaries.append({'name': binary.name, 'sha256': digest(binary), 'file_version': version,
                         'product_version': version, 'icon_payloads_verified': True})
    return {**prepared, 'binaries': binaries}

def capture(args: list[str], cwd: Path = ROOT) -> str:
    return subprocess.check_output(args, cwd=cwd, text=True).strip()

STEPS: list[dict[str, object]] = []
OUTPUT: Path | None = None

def run(args: list[str], env: dict[str, str], cwd: Path = ROOT,
        *, stdout_file: Path | None = None, check: bool = True) -> int:
    """Retain command, exit and complete output, including failed validation."""
    assert OUTPUT is not None
    name = f"{len(STEPS) + 1:02d}-{Path(args[0]).stem}-{args[1] if len(args) > 1 else 'run'}"
    name = "".join(c if c.isalnum() or c in "-." else "-" for c in name)
    log = OUTPUT / f"{name}.log"
    step = {"command": args, "cwd": cwd.relative_to(ROOT).as_posix(),
            "log": log.name, "stdout": stdout_file.name if stdout_file else log.name}
    STEPS.append(step)
    print("+ " + " ".join(args), flush=True)
    started = time.monotonic()
    try:
        with log.open("wb") as diagnostics:
            if stdout_file is None:
                process = subprocess.Popen(args, cwd=cwd, env=env,
                                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                assert process.stdout is not None
                for line in process.stdout:
                    diagnostics.write(line)
                    diagnostics.flush()
                    sys.stdout.buffer.write(line)
                    sys.stdout.buffer.flush()
                code = process.wait()
                process.stdout.close()
            else:
                with stdout_file.open("wb") as output:
                    code = subprocess.run(args, cwd=cwd, env=env, stdout=output,
                                          stderr=diagnostics, check=False).returncode
        step["exit_code"] = code
    except OSError as error:
        step["exit_code"] = 127
        log.write_text(f"{type(error).__name__}: command could not execute\n", encoding="utf-8")
        raise
    finally:
        step["elapsed_seconds"] = round(time.monotonic() - started, 3)
        (OUTPUT / "VALIDATION-STEPS.json").write_text(json.dumps(STEPS, indent=2) + "\n", encoding="utf-8")
    if check and code:
        raise subprocess.CalledProcessError(code, args)
    return code

def digest(path: Path) -> str:
    if path.is_symlink() or not path.is_file():
        raise ValueError("artifact must be a regular non-symlink file")
    value = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            value.update(block)
    return value.hexdigest()

def windows_runtime_receipt(path: Path, source: str, *, binary: Path | None = None) -> dict:
    """Bind the VS-only signature/version observation to the bytes built/shipped."""
    if path.is_symlink() or not path.is_file():
        raise ValueError('runtime receipt must be a regular file')
    receipt = json.loads(path.read_text(encoding='utf-8'))
    script = 'rust/assets/whisperruntime/fetch_windows_runtime.ps1'
    if (receipt.get('schema_version') != 1 or receipt.get('source_sha') != source
            or receipt.get('target') != 'x86_64-pc-windows-msvc'
            or receipt.get('acquisition') != 'visual-studio-redist'
            or receipt.get('source_script') != script
            or receipt.get('source_script_sha256') != digest(ROOT / script)):
        raise ValueError('runtime receipt source/provenance mismatch')
    entries = receipt.get('files')
    if (not isinstance(entries, list) or len(entries) != 4
            or not all(isinstance(entry, dict) for entry in entries)
            or sorted(entry.get('name', '') for entry in entries) != sorted(WINDOWS_RUNTIME_NAMES)):
        raise ValueError('runtime receipt requires exactly four distinct DLLs')
    payload = ROOT / 'rust/assets/whisperruntime/files/windows-amd64'
    if payload.is_symlink():
        raise ValueError('runtime payload directory must not be a symlink')
    binary_bytes = binary.read_bytes() if binary is not None else None
    for entry in entries:
        dll = payload / entry['name']
        if (entry.get('signature_status') != 'Valid' or entry.get('microsoft_signer') is not True
                or entry.get('pe_machine') != '0x8664'
                or not isinstance(entry.get('file_version'), str) or not entry['file_version'].strip()
                or digest(dll) != entry.get('sha256') or dll.stat().st_size != entry.get('bytes')):
            raise ValueError('runtime DLL verification/identity mismatch')
        data = dll.read_bytes()
        offset = int.from_bytes(data[0x3c:0x40], 'little') if len(data) >= 64 else len(data)
        if data[:2] != b'MZ' or data[offset:offset + 6] != b'PE\0\0\x64\x86':
            raise ValueError('runtime DLL must be x64 PE')
        if binary_bytes is not None and data not in binary_bytes:
            raise ValueError('verified runtime DLL bytes absent from main binary')
    if binary is not None:
        receipt['embedded_in'] = binary.name
        receipt['embedded_bytes_verified'] = True
    return receipt

def host_from_verbose(version: str) -> str:
    return next((line.removeprefix("host: ") for line in version.splitlines() if line.startswith("host: ")), "")

def main() -> None:
    global OUTPUT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--release-tag", help="explicit release build from this strict SemVer tag; never publishes")
    args = parser.parse_args()
    # HEAD identifies the compiled inputs only for a clean source checkout.
    # Dirty local recovery candidates need their separate source-manifest receipt.
    if capture(["git", "status", "--porcelain=v1", "--untracked-files=normal"]):
        raise SystemExit("candidate CI requires a clean checkout; use a separate source-manifest receipt for local recovery builds")
    source = capture(["git", "rev-parse", "HEAD"])
    try:
        stamp = build_stamp(source, args.release_tag)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"release/source validation failed: {error}") from error
    output = ROOT / ".rust-candidate-artifacts" / TARGETS[args.target]
    output.mkdir(parents=True, exist_ok=False)
    OUTPUT = output
    (output / "CANDIDATE-INPUT.json").write_text(json.dumps({
        "source_sha": source, "target": args.target, "status": "validation-started",
        "review_head_sha": os.environ.get("MANY_AI_REVIEW_HEAD_SHA", source),
        "cargo_lock_sha256": digest(ROOT / "rust" / "Cargo.lock"),
        **stamp,
    }, indent=2) + "\n", encoding="utf-8")
    rustc = capture(["rustc", "--version", "--verbose"])
    if not rustc.startswith("rustc 1.90.0 ") or host_from_verbose(rustc) != args.target:
        raise SystemExit("native runner host or pinned Rust version does not match matrix target")
    go = capture(["go", "version"])
    bun = capture(["bun", "--version"])
    if not go.startswith("go version go1.26.8 ") or bun != "1.3.14":
        raise SystemExit("pinned Go/Bun toolchain mismatch")
    if capture(["git", "rev-parse", "HEAD"]) != source:
        raise SystemExit("source changed during toolchain validation")
    env = os.environ.copy()
    env.update({
        "RUSTUP_TOOLCHAIN": "1.90.0", "GOTOOLCHAIN": "local",
        "CARGO_INCREMENTAL": "0", "CARGO_PROFILE_DEV_DEBUG": "0",
        "CARGO_PROFILE_TEST_DEBUG": "0", "CARGO_BUILD_JOBS": "2",
        "MANY_AI_BUILD_VERSION": stamp['version'],
        "MANY_AI_BUILD_COMMIT": stamp['build_commit'],
        "MANY_AI_BUILD_TIME": stamp['build_time'],
        "MANY_AI_BUILD_KIND": stamp['build_kind'],
    })
    # Populate selected caches before isolated Go fixtures deliberately disable
    # network access and replace HOME. Their child environment keeps these paths.
    env["GOMODCACHE"] = capture(["go", "env", "GOMODCACHE"])
    env["GOCACHE"] = capture(["go", "env", "GOCACHE"])
    env["GOTELEMETRY"] = "off"
    # The frozen Go oracle module lives under go/ since 2026-10-19.
    run(["go", "mod", "download"], env, ROOT / "go")
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    runtime_receipt = None
    resources = None
    runtime_path = output / 'WINDOWS-RUNTIME.json'
    if args.target == 'x86_64-pc-windows-msvc':
        run(['pwsh', '-NoProfile', '-File', str(ROOT / 'rust/packaging/prepare_windows_runtime.ps1'),
             '-ReceiptPath', str(runtime_path)], env)
        runtime_receipt = windows_runtime_receipt(runtime_path, source)
        env['MANY_AI_REQUIRE_WINDOWS_RUNTIME'] = '1'
        if args.release_tag:
            resource_dir = output / 'windows-resources'
            resources = prepare_windows_resources(resource_dir, stamp['version'], env)
            env['MANY_AI_WINDOWS_RESOURCE_DIR'] = str(resource_dir.resolve())
    run([sys.executable, "-m", "unittest", "discover", "-s", "rust/packaging", "-p", "test_*.py"], env)
    run(["bun", "install", "--frozen-lockfile"], env, ROOT / "web")
    run(["bun", "run", "check"], env, ROOT / "web")
    run(["bun", "run", "build"], env, ROOT / "web")
    cargo = ["cargo", "--locked", "--manifest-path", "rust/Cargo.toml"]
    if args.release_tag:
        if capture(['git', 'rev-parse', 'HEAD'], ROOT) != source:
            raise SystemExit('release source changed before compilation')
        verify_release_source(args.release_tag, source)
    # Cargo places command before its command-specific manifest/lock arguments.
    run(["cargo", "fmt", "--manifest-path", "rust/Cargo.toml", "--", "--check"], env)
    run(["cargo", "clippy", *cargo[1:], "--target", args.target, "--all-targets", "--", "-D", "warnings"], env)
    run(["cargo", "test", *cargo[1:], "--target", args.target, "--all-targets", "--no-fail-fast", "--", "--test-threads=8"], env)
    run(["cargo", "test", *cargo[1:], "--target", args.target, "--doc"], env)
    build_messages = output / "cargo-build.jsonl"
    run(["cargo", "build", *cargo[1:], "--target", args.target, "--release", "--bins", "--message-format=json"], env, stdout_file=build_messages)
    metadata = output / "cargo-metadata.json"
    run(["cargo", "metadata", *cargo[1:], "--format-version", "1", "--filter-platform", args.target], env, stdout_file=metadata)
    target_root = Path(env.get("CARGO_TARGET_DIR", str(ROOT / "rust" / "target")))
    if not target_root.is_absolute():
        target_root = ROOT / target_root
    binary_entries = []
    for stem in ("many-ai-cli", "many-ai-cli-launcher"):
        name = stem + (".exe" if "windows" in args.target else "")
        source_file = target_root / args.target / "release" / name
        checksum = digest(source_file)
        if source_file.stat().st_size == 0:
            raise SystemExit("empty candidate binary")
        destination = output / name
        shutil.copy2(source_file, destination)
        if digest(destination) != checksum:
            raise SystemExit("candidate copy hash mismatch")
        binary_entries.append({"name": name, "bytes": destination.stat().st_size, "sha256": checksum})
    if runtime_receipt is not None:
        confirmed = windows_runtime_receipt(runtime_path, source, binary=output / 'many-ai-cli.exe')
        if any(confirmed[key] != value for key, value in runtime_receipt.items()):
            raise SystemExit('runtime inputs changed during native build')
        runtime_receipt = confirmed
    if resources is not None:
        resources = verify_windows_resources(output, stamp['version'], resources)
    web_entries = [{"path": path.relative_to(ROOT / "web" / "dist").as_posix(), "sha256": digest(path)}
                   for path in sorted((ROOT / "web" / "dist").rglob("*")) if path.is_file()]
    notice_entries = []
    for relative in ("rust/THIRD-PARTY-NOTICES.md", "rust/src/approval/policy/go_regex/GO-LICENSE"):
        notice = ROOT / relative
        destination = output / notice.name
        checksum = digest(notice)
        shutil.copy2(notice, destination)
        if digest(destination) != checksum:
            raise SystemExit("candidate notice copy hash mismatch")
        notice_entries.append({"name": notice.name, "sha256": checksum})
    collector = ROOT / "rust" / "packaging" / "collect_inputs.py"
    packaging_exit = run([sys.executable, str(collector), "--lockfile", "rust/Cargo.lock",
                         "--metadata", str(metadata), "--build-messages", str(build_messages),
                         "--cargo-home", env.get("CARGO_HOME", str(Path.home() / ".cargo")),
                         "--target", args.target, "--source-revision", source,
                         "--output", str(output / "packaging-inputs")], env, check=False)
    packaging_inputs = []
    if packaging_exit == 0:
        for name in PACKAGING_INPUTS:
            path = output / 'packaging-inputs' / name
            packaging_inputs.append({'name': name, 'bytes': path.stat().st_size, 'sha256': digest(path)})
    receipt = {
        "schema_version": 2,
        "status": "native-release-build-not-accepted-for-publication" if args.release_tag else "native-build-only-not-accepted-for-cutover",
        **stamp,
        "source_sha": source, "review_head_sha": env.get("MANY_AI_REVIEW_HEAD_SHA", source),
        "oracle_sha": "d8fbf8598c3effd4e2f837e43ad6f0488c461de3",
        "target": args.target, "target_name": TARGETS[args.target],
        "rustc": rustc, "go": go, "bun": bun, "python": sys.version.split()[0],
        "version": env["MANY_AI_BUILD_VERSION"], "build_time": env["MANY_AI_BUILD_TIME"],
        "cargo_lock_sha256": digest(ROOT / "rust" / "Cargo.lock"),
        "binaries": binary_entries, "generated_web_asset_inputs": web_entries,
        "third_party_notices": notice_entries,
        "windows_runtime": runtime_receipt,
        "windows_resources": resources,
        "packaging_inputs": packaging_inputs,
        "validation_steps": STEPS, "packaging_input_exit_code": packaging_exit,
        "launcher_ui_input_sha256": digest(ROOT / "rust" / "assets" / "launcher" / "ui" / "index.html"),
        "receipt_scope": "native validation/build plus input hashes; linked runtime asset retention is not yet verified",
        "pending": ["binary-entrypoint-and-asset-retention", "full-application-behavior", "native-device-remote-acceptance", "signed-channel-packaging", "rollback-rehearsal", "user-cutover-approval"],
    }
    (output / "BUILD-RECEIPT.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    if args.release_tag:
        (output / 'RELEASE-EVIDENCE.txt').write_text('This receipt covers release stamping, binary byte identity and collected build-input evidence. Signing, native-device acceptance, rollback and distribution-license review require their separate evidence.\n', encoding='utf-8')
    else:
        (output / "NOT-FOR-CUTOVER.txt").write_text("WIP Rust migration candidate. Native compilation and synthetic tests are not full compatibility, signing, packaging, rollback or user acceptance. Do not replace an installed Go binary.\n", encoding="utf-8")
    print("CANDIDATE-BINARY-HASHES " + json.dumps({"source_sha": source, "target": args.target,
          "cargo_lock_sha256": receipt["cargo_lock_sha256"], "binaries": binary_entries}), flush=True)
    if packaging_exit:
        raise SystemExit("native binaries built; packaging input evidence is incomplete (see receipt)")
    print(f"Native {stamp['build_kind']} receipt: {TARGETS[args.target]} / {source}", flush=True)

if __name__ == "__main__":
    main()
