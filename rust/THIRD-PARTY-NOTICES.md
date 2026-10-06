# Third-party source notices for the Rust candidate

## Microsoft Visual C++ Runtime (Windows x64)

Windows candidates embed the four Microsoft VC++ runtime DLLs prepared by the
existing Go Visual Studio Redist acquisition policy: `vcomp140.dll`,
`msvcp140.dll`, `vcruntime140.dll`, and `vcruntime140_1.dll`.
Copyright Microsoft Corporation. These are Microsoft redistributable
components, not MIT-licensed Rust project code. The existing Go project's
redistribution decision is inherited; this continuation makes no new licence
eligibility decision and does not publish a release.

The applicable source is the Visual Studio `VC/redist` distribution under
[Microsoft's redistribution terms](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution#visual-c-runtime-files).
No System32 fallback is used by candidate CI. Actual FileVersion, SHA-256,
signature and architecture observations are retained per file in
`WINDOWS-RUNTIME.json` and `BUILD-RECEIPT.json.windows_runtime`; versions/hashes
are observed per build, not pre-pinned. Non-Windows candidates embed none of
these supplemental files.

## Go Unicode / regular-expression compatibility data

The files under `src/approval/policy/go_regex/` contain Unicode15.0 data and
regular-expression compatibility behavior derived from the Go Authors' Go1.26.8
source. Exact source URLs/hashes and reproduction commands are recorded in
`tests/fixtures/services/autoapproval-unicode/provenance.json` and its README.
There is no runtime Go subprocess dependency.

The following license applies to this Go-derived material. Candidate binary
archives carry this notice and a separate copy of GO-LICENSE. Other dependency
license/SBOM and final distribution acceptance remain part of the unfinished
release checklist; this file is not a claim that all release notices are complete.

Copyright 2009 The Go Authors.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

   * Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.
   * Redistributions in binary form must reproduce the above
copyright notice, this list of conditions and the following disclaimer
in the documentation and/or other materials provided with the
distribution.
   * Neither the name of Google LLC nor the names of its
contributors may be used to endorse or promote products derived from
this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
