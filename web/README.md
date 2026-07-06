# web/ — Hub UI frontend

このディレクトリは Hub UI のフロントエンド（静的 HTML/CSS/TypeScript + `web/src/vendor/` のクラシックスクリプト）を管理する。Go 側は `web/dist/` を `//go:embed` で取り込むだけで、ここが真実のソース。

## パッケージマネージャは **bun のみ**

このディレクトリでは **`bun` を唯一の package manager として使う**。`npm` / `pnpm` / `yarn` は使わない。

- 依存の追加・更新: `bun add <pkg>` / `bun install`
- ビルド: `bun run build`
- TypeScript typecheck: `bun run check`
- テスト: `bun run test`

### なぜ npm 禁止か

歴史的経緯で `package-lock.json` (npm) と `bun.lock` (bun) の両方が git 管理下に残っている。両方が並存すると:

- 誤って `npm install` すると `package-lock.json` だけが更新され `bun.lock` と drift する
- レビュー時に「本当の依存版はどっち？」が分からなくなる
- CVE スキャナ / Dependabot も両方を見て二重通知する

そのため:

- **真実は `bun.lock`** ─ CI (`.github/workflows/validate.yml`) が `bun install --frozen-lockfile` で厳格チェックする
- **`package-lock.json` は手動で更新しない** ─ 変更 PR には CI が warning を出す（`web-package-manager-consistency` job）
- 将来的に `package-lock.json` の物理削除は判断待ち D グループ扱い（本ラウンド審査対象外）

## Vendor スクリプト

`web/src/vendor/*.min.js` はクラシックスクリプト（ESM でない）として index.html から `<script>` タグで読み込まれる。バージョンと upstream URL は `web/src/vendor/THIRD_PARTY_LICENSES.txt` にまとめて記録している (2026-07-05 時点):

| パッケージ | Version | URL |
|---|---|---|
| DOMPurify | 3.4.1 | https://github.com/cure53/DOMPurify/tree/3.4.1/ |
| marked | 12.0.2 | https://github.com/markedjs/marked/tree/v12.0.2/ |
| highlight.js | 11.10.0 | https://github.com/highlightjs/highlight.js/tree/11.10.0/ |
| qrcode-generator | unknown | https://github.com/kazuhikoarase/qrcode-generator/ |
| xterm.js 系 | 6.0.0 + addons | https://github.com/xtermjs/xterm.js/ |

CVE が出た場合はここを手動照合する。将来的に `package.json` の devDependencies に取り込んで `npm audit` / Dependabot の可視範囲に入れる案は `docs/local/plan_audit_score_s_promotion_2026-07-05.md` の C3 段階 2 参照。
