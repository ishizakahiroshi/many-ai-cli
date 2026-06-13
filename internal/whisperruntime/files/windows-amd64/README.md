# windows-amd64 同梱ランタイム（VC++ Runtime）

このディレクトリには、`whisper-server.exe` が依存する **Microsoft Visual C++
ランタイム DLL 4 点**を配置する。`go:embed` で本体バイナリに同梱され、
managed install 時に `~/.many-ai-cli/whisper/bin/` へ展開される（`whisperruntime.Ensure`）。

これにより、VC++ ランタイム未導入の Windows でも `whisper-server.exe` 起動時の
`0xC0000135`(STATUS_DLL_NOT_FOUND) を回避する（plan C2 / D2）。

## 同梱する 4 点（x64）

| DLL | 役割 |
|---|---|
| `vcomp140.dll` | OpenMP（`ggml-base.dll` / `ggml-cpu.dll` が実 import） |
| `msvcp140.dll` | C++ 標準ライブラリ |
| `vcruntime140.dll` | C ランタイム |
| `vcruntime140_1.dll` | x64 の FH4 例外処理（x86 には無い） |

UCRT（`ucrtbase.dll` / `api-ms-win-crt-*`）は Windows 10/11 同梱のため**非同梱**。

## 取得方法（自動）

リポジトリルートから次を実行すると、**Visual Studio の `\VC\Redist\MSVC\<ver>\x64`**
（`vswhere` で探索する `Microsoft.VC*.CRT` + `Microsoft.VC*.OpenMP`）から
**正規・署名済み**の 4 点をこのディレクトリへ取得し、Authenticode 署名と
PE machine(0x8664) を検証する:

```powershell
pwsh internal/whisperruntime/fetch_windows_runtime.ps1
```

VS が無い環境では既定でエラー終了する。ローカル検証に限り `-AllowSystem32` で
System32 のコピーを使えるが、それは OS servicing 版で **再頒布許諾の対象外**
（plan D7）なので**リリース／CI では使わない**こと。

> ⚠️ このスクリプトは `vc_redist.x64.exe /layout` は使わない（VS Redist フォルダ
> 由来が再頒布許諾の正路のため）。VS 未導入なら VS Build Tools の C++ ワークロード、
> または vc_redist を VS layout 形式で展開してから実行する。

## ライセンス（D7）

4 点は Visual Studio の "Distributable Code"。`\VC\redist` フォルダ単位許諾の
対象で、**未改変・x64・正規 VS ライセンス保有者の再頒布**に限り許諾される。
app-local 配置は公式に認められた配備手段。デバッグ版 DLL(`*140d.dll`)は再頒布禁止。
About 画面では OSS と区別して "Redistributed Microsoft Components" として謝辞表示する。

> このディレクトリの `.dll` は `.gitignore` 済み（バイナリをリポジトリに含めない）。
> リリース／CI 時に上記スクリプトで取得してからビルドすること。
