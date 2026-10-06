# Windows whisper runtime supply and build receipts

> 最終更新: 2026-10-06(火) 01:47:03 UTC

Baseline reviewed: `4dc961ce600d12aa0911e5a9620e50411ab13d9f`. The owner corrected the original fixed-version/fixed-hash prerequisite: Go deliberately chooses the installed Visual Studio Redist version. This continuation therefore uses the same VS-only acquisition/validation policy and records actual input identities per build. The earlier missing-pin STOP is superseded. No new redistribution-eligibility judgment is made; the existing Go project's Visual Studio redistribution decision is inherited as instructed.

## Source and verification contract

The four x64 files are `vcomp140.dll`, `msvcp140.dll`, `vcruntime140.dll` and `vcruntime140_1.dll`. The existing [Go preparation script](https://github.com/ishizakahiroshi/many-ai-cli/blob/4dc961ce600d12aa0911e5a9620e50411ab13d9f/internal/whisperruntime/fetch_windows_runtime.ps1) selects `vswhere -latest`, the highest installed `VC/Redist/MSVC` directory, then x64 CRT/OpenMP files. It checks a Valid Authenticode signature whose signer Subject matches the case-insensitive substring/regular-expression text `Microsoft Corporation` (not an exact publisher-name or pinned-certificate comparison) and PE machine `0x8664`.

`rust/packaging/prepare_windows_runtime.ps1` invokes that script with its VS-only default and exposes no System32/alternate-source option. It rechecks the four resulting regular files, records their actual FileVersion, byte size, SHA-256, signature result and x64 machine, and binds the receipt to source HEAD plus the acquisition-script hash. No version or DLL hash is pre-pinned. VS servicing may legitimately change those observations on a later build.

The original Go `-AllowSystem32` option remains explicitly development-only; the Rust CI/preparation path never invokes or exposes it. No third-party DLL site, credential, global runtime installation or System32 write is used. The DLL directory remains the existing gitignored Go embed-input directory; no DLL is added to Git.

## Rust build, runtime and artifact binding

`rust/build.rs` embeds the exact prepared files into `WINDOWS_RUNTIME` only for Windows x64. Empty, unprepared development inputs preserve Go's placeholder-only no-op behavior. A partial set, nonregular file, malformed PE or wrong architecture fails the build. Candidate CI sets `MANY_AI_REQUIRE_WINDOWS_RUNTIME=1`, so a Windows candidate cannot silently succeed with an empty set.

The production serve composition supplies `assets::windows_runtime_payload()` to the existing Whisper manager. Its existing runtime copy behavior remains idempotent: present files are preserved and missing files are created through the held Whisper bin directory. Other target builds have an empty supplemental VC runtime payload. These Microsoft DLLs are separate from the selected whisper-server archive/model and their manifest identities.

`scripts/rust-candidate-ci.py` prepares the four files before Cargo, verifies the receipt and hashes, then checks them again after building. Each complete DLL byte sequence must be retained in the copied `many-ai-cli.exe`. The artifact's `BUILD-RECEIPT.json.windows_runtime` preserves the per-file versions/hashes and embedded-byte result alongside the main and launcher binary hashes. `WINDOWS-RUNTIME.json` retains the preparation observation. This proves the included bytes, not their execution on a machine lacking a system-wide runtime.

Regression coverage checks missing/partial/duplicate inputs, nonregular/symlink inputs, invalid PE/architecture, changed bytes/source provenance, invalid signature assertions, absent versions and a binary lacking the expected payload. Synthetic receipt fixtures do not claim to verify a real Authenticode signature. The final Windows CI supplies that native check; the Windows machine/device acceptance remains a separate gate.

## Licence and release boundary

Microsoft's [VS 2022 redistribution list](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution#visual-c-runtime-files) describes unmodified files under `VC/redist`, subject to the applicable Visual Studio terms; debug nonredistributable folders are excluded. [Application-local deployment](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files?view=msvc-170#install-individual-redistributable-files) is documented. The [Community licence](https://visualstudio.microsoft.com/wp-content/uploads/2021/11/Visual-Studio-2022-Community-License-EN.docx), [Build Tools terms](https://visualstudio.microsoft.com/license-terms/vs2022-ga-diagnosticbuildtools/) and [runtime terms](https://visualstudio.microsoft.com/license-terms/vs2022-cruntime/) were readable without credentials.

These remain Microsoft redistributable components, distinct from the project's MIT-licensed Rust source. The existing Go licensing treatment is carried into the Rust notice and input receipt. No new legal agreement was accepted, no signing/publishing credentials were accessed, and `.github/workflows/release.yml` was not changed. V03's production signer/publisher isolation and final distribution terms remain separate, documentation-only work in this continuation.

The immutable [4dc baseline artifact receipt](ARTIFACTS-4dc961ce.md) still describes its then-empty Rust payload. It must not be relabelled as a build of this new implementation. The final continuation SHA and actual Windows DLL values belong to the new native CI artifact receipt after validation.
