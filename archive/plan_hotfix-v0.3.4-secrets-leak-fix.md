# [完了] many-ai-cli hotfix v0.3.4 — kb 由来実値フィクスチャ/ドキュメント漏洩修正

> 最終更新: 2026-06-22(月) 19:58:10 — **クローズ**。C1〜C6 全完了 + npm 過去版 deprecate（5 packages × v0.3.2/v0.3.3）完了。kb 由来語 4 種は GitHub Code Search remote / ローカル git log すべて 0 件。GitHub Releases 9 件・npm 5 パッケージ・winget PR / homebrew cask すべて維持。ユーザー指示「閉じる」を受け [様子見] → [完了] へ。archive 候補。

## context配分

| C | 種別 | 内容 | 並列 |
|---|---|---|---|
| C1 | fix | hotfix/v0.3.4 ブランチ作成（main から派生・clean 確認） | — |
| C2 | fix | 3 ファイル修正適用（実値 → 合成置換） | — |
| C3 | fix | 検証（secrets-scan + parser test 動作確認） | — |
| C4 | fix | hotfix → main マージ + v0.3.4 タグ + push（release 自動化トリガ） | — |
| C5 | fix | hotfix → develop マージ（修正の develop 取り込み） | — |
| C6 | fix | 履歴クリーンアップ（git filter-repo + force push）+ 過去リリース扱い判断 | — |

実行順序: `C1 → C2 → C3 → C4 → C5 → C6`

---

## 概要

2026-06-22 の worklog-bridge セッションで実施した kb-driven secrets-scan の cross-repo sweep 結果、many-ai-cli 内に worklog-bridge bf0b750 と同種の **kb 由来実値漏洩** を検出。

検出箇所:
- `web/src/app/approval-parser-fixtures.ts:51, 54` — テストフィクスチャに業務承認プロンプト実値（3 社名同時）
- `CLAUDE/operations.md:9` — ドキュメント中に実名（漢字）
- `CLAUDE/windows_setup.md:5` — 同上

緊急ホットフィックス **v0.3.4** として配布。fix は main マージ → develop へ伝搬。過去履歴は filter-repo で書き換え、唯一の fork はボットのため force push の実害なし。

worklog-bridge セッションで working tree 編集を試行したが、本 plan 整備のため revert 済。実作業は本 plan に従い many-ai-cli ローカルセッションで実施する。

## 前提

- ブランチ構成（remote 確認済）: `main` / `develop` / `hotfix/v0.3.3` が存在・GitFlow 運用
- 既存タグ: `v0.3.0` 〜 `v0.3.3`。次は **`v0.3.4`**
- remote: `https://github.com/ishizakahiroshi/many-ai-cli.git`
- 修正案 4 件は worklog-bridge セッションでユーザー承認済（synthetic 置換）
- secrets-scan: worklog-bridge の `scripts/secrets-scan.mjs` を流用（`KB_ROOT=C:/dev/kb` 既定で動作）
- fork: ボット 1 件のみ → 履歴 force push の影響なし

## C1: hotfix/v0.3.4 ブランチ作成

### 作業手順

```bash
cd C:/dev/github/public/many-ai-cli

# 0. 現状確認（worklog-bridge セッションで working tree に試行編集を残していたら破棄）
git status
git checkout -- web/src/app/approval-parser-fixtures.ts CLAUDE/operations.md CLAUDE/windows_setup.md 2>/dev/null || true

# 1. main を最新化
git checkout main
git pull origin main

# 2. hotfix ブランチ作成
git checkout -b hotfix/v0.3.4

# 3. 出発点が main の HEAD であることを verify
git log --oneline -3
```

### 完了条件
- `hotfix/v0.3.4` が remote `main` の最新コミットから派生
- working tree が clean

---

## C2: 3 ファイル修正

### 2-1: `web/src/app/approval-parser-fixtures.ts`（L50-56 付近）

**Before:**

```typescript
assert.deepEqual(labels(parser.extractPlainYesNoApproval([
  'マーキュリー・メイジ・タイムリーの3台で連絡先関連機能をOFFにしますか？ （Y：1／N：0）',
])), ['Yes (1)', 'No (0)']);
assert.deepEqual(labels(parser.extractPlainYesNoApproval([
  'マーキュリー・メイジ・タイムリーの3台で連絡先関連機能をOFFにしますか？ (Y:1/',
  'N:0)',
])), ['Yes (1)', 'No (0)']);
```

**After:**

```typescript
assert.deepEqual(labels(parser.extractPlainYesNoApproval([
  'A拠点・B拠点・C拠点の3台で対象機能を無効化しますか？ （Y：1／N：0）',
])), ['Yes (1)', 'No (0)']);
assert.deepEqual(labels(parser.extractPlainYesNoApproval([
  'A拠点・B拠点・C拠点の3台で対象機能を無効化しますか？ (Y:1/',
  'N:0)',
])), ['Yes (1)', 'No (0)']);
```

**検証ポイントの維持:**
- 長い日本語前置き ✓（"A拠点・B拠点・C拠点の3台で対象機能を無効化しますか？"）
- 全角括弧 `（）` / 全角コロン `：` / 全角スラッシュ `／` ✓
- 半角折り返し `(Y:1/\nN:0)` 構造 ✓
- 3 項目 + `・` 区切り ✓

### 2-2: `CLAUDE/operations.md` L9

**Before:** `**原則：** Git コミットはユーザー（石坂）が実施する`
**After:** `**原則：** Git コミットはユーザーが実施する`

### 2-3: `CLAUDE/windows_setup.md` L5

**Before:** `開発端末は Windows 11。\`many-ai-cli\` 自体はクロスプラットフォーム（Win/Mac/Linux）だが、本ドキュメントは石坂環境固有の手順をまとめる。`
**After:** `開発端末は Windows 11。\`many-ai-cli\` 自体はクロスプラットフォーム（Win/Mac/Linux）だが、本ドキュメントは作者の環境固有の手順をまとめる。`

### 完了条件

- 3 ファイル編集完了
- `git diff --stat` で +4 / -4 の 3 ファイル差分

---

## C3: 検証

### 3-1: secrets-scan（kb 由来語残存なし）

```bash
# worklog-bridge の共通スクリプトを流用
cd C:/dev/github/public/many-ai-cli
node C:/dev/github/public/worklog-bridge/scripts/secrets-scan.mjs --all-tracked --dry-run --format=json \
  | node -e "let d='';process.stdin.on('data',c=>d+=c);process.stdin.on('end',()=>{const j=JSON.parse(d);const w=(j.hits||[]).filter(h=>h.kind==='watchlist');console.log('watchlist hits: '+w.length);for(const h of w)console.log('  '+h.matched+' @ '+h.file+':'+h.lineNumber);});"
```

**期待値:**
- 修正前: 4 件（マーキュリー ×2, 石坂 ×2）+ noise（かつ、WF）
- 修正後: noise（かつ・WF）のみ → 修正対象 4 件が消えていれば OK

### 3-2: parser test 動作確認

```bash
# bun または npm/pnpm 環境に応じて
cd web
bun test src/app/approval-parser-fixtures.ts
# または
npm test
```

**期待値:** すべての assert pass。修正は文字列だけで parser のロジックを触らないため、構造的検証は完全に維持されている。

### 完了条件

- watchlist hits 0（noise を除く）
- parser test 全 pass

---

## C4: hotfix → main マージ + v0.3.4 タグ + push

### 作業手順

```bash
# 1. hotfix ブランチでコミット
git checkout hotfix/v0.3.4
git add web/src/app/approval-parser-fixtures.ts CLAUDE/operations.md CLAUDE/windows_setup.md
git commit -m "$(cat <<'EOF'
fix: replace real-data test fixtures and CLAUDE docs with synthetic equivalents

worklog-bridge の cross-repo secrets-scan sweep (2026-06-22) で本リポ内に
kb 由来実値の漏洩を検出。同種の漏洩が worklog-bridge bf0b750 にもあり、
4 層防御の整備 (docs/local/ に設計まとめ) と並行して緊急修正。

修正内容:
- web/src/app/approval-parser-fixtures.ts L51/54: 業務承認プロンプト実値
  (3 拠点同時無効化を尋ねる文面) を合成データ "A拠点・B拠点・C拠点" に
  置換。テストの parser 検証カバレッジ (長い日本語前置き / 全角句読点 /
  半角折り返し / 3 項目 ・ 区切り) は完全維持。
- CLAUDE/operations.md L9: 主体表記から漢字氏名を除去
- CLAUDE/windows_setup.md L5: 環境固有の主体表記から漢字氏名を除去

参考: worklog-bridge/docs/local/secrets-scan-design/index.html (4 層防御 / 戦略 2)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>
EOF
)"

# 2. main にマージ
git checkout main
git merge --no-ff hotfix/v0.3.4 -m "Merge hotfix/v0.3.4: secrets leak fix"

# 3. タグ作成
git tag -a v0.3.4 -m "hotfix: remove kb-derived real data from test fixtures and CLAUDE docs"

# 4. push（main → リリース workflow 起動）
git push origin main
git push origin v0.3.4

# 5. release workflow 監視
gh run watch
```

### 完了条件

- main に hotfix merge commit が記録
- `v0.3.4` タグが remote に push 済
- `gh run list --workflow=Release --limit 1` が success
- npm: `npm view many-ai-cli version` が `0.3.4`
- GitHub Release: `gh release view v0.3.4` で公開済

---

## C5: hotfix → develop マージ

### 作業手順

```bash
git checkout develop
git pull origin develop
git merge --no-ff hotfix/v0.3.4 -m "Merge hotfix/v0.3.4 into develop"
git push origin develop
```

### 完了条件

- develop に hotfix/v0.3.4 のコミットが含まれる
- `git log develop --grep "real-data test fixtures"` で commit が見える

---

## C6: 履歴クリーンアップ + 過去リリース扱い

### 6-1: git filter-repo で履歴から実値を削除

**前提:**
- `pip install git-filter-repo` 済 / または `gh-extension` 経由
- バックアップとして別ディレクトリに mirror clone を取得しておく

**作業手順:**

```bash
# 0. バックアップ（万一に備えて）
cd /tmp
git clone --mirror C:/dev/github/public/many-ai-cli many-ai-cli-backup-2026-06-22.git

# 1. 履歴書き換え（全ブランチ・全タグの全コミットに適用）
cd C:/dev/github/public/many-ai-cli
git filter-repo --replace-text <(cat <<'EOF'
マーキュリー==>A拠点
メイジ==>B拠点
タイムリー==>C拠点
石坂==>ユーザー
EOF
)

# 2. force push（main / develop / 全 hotfix ブランチ / 全タグ）
git push --force origin --all
git push --force origin --tags
```

**注意:**
- **破壊的操作**。filter-repo は全コミットのハッシュを変える
- 既存タグ `v0.3.0` 〜 `v0.3.3` を含む全コミットの SHA が変わる
- fork ボット 1 件は古い履歴を保持し続けるが、影響軽微（ユーザー指示で無視）
- filter-repo の代替: BFG Repo Cleaner（こちらも可）

### 6-2: npm 過去版扱い

**確認:**

```bash
npm view many-ai-cli versions
# 例: ["0.1.1","0.1.2","0.1.3","0.2.0","0.2.2","0.3.0","0.3.1","0.3.2","0.3.3"]

for v in 0.3.0 0.3.1 0.3.2 0.3.3; do
  npm pack many-ai-cli@$v
  tar -tzf many-ai-cli-$v.tgz | xargs -I{} sh -c 'tar -xzOf many-ai-cli-'$v'.tgz {} 2>/dev/null | grep -l "マーキュリー\|石坂" >/dev/null && echo "$v: {}"' 2>/dev/null
done
```

**対応案:**
- 該当版があれば: `npm deprecate many-ai-cli@<version> "Contains kb-derived data; upgrade to v0.3.4+"`
- 72 時間以内なら `npm unpublish` も技術的に可能だが SemVer エコシステム上の事故扱い → **非推奨**
- 公開済みアーティファクト（tgz）自体は npm から消えない（rehosted/cached 経路もあり完全消去は不可）

### 6-3: GitHub Release アーカイブ扱い

**確認:**

```bash
gh release list --limit 20
# 各 release の asset を確認
for v in v0.3.0 v0.3.1 v0.3.2 v0.3.3; do
  gh release download $v --pattern "*.zip" -O /tmp/$v.zip
  unzip -p /tmp/$v.zip | grep -l "マーキュリー\|石坂" >/dev/null && echo "$v contains leak"
done
```

**対応案:**
- 該当 release があれば: `gh release delete $v --yes`
- 履歴 force push 後はそもそも該当タグの参照不能になるため、release も自動的に意味を失う
- 安全策: release を削除 → ユーザーには v0.3.4 への移行案内

### 完了条件

- `git filter-repo + force push` 完了
- npm 過去版で漏洩のあるものに deprecate 適用（unpublish は基本回避）
- GitHub Release で漏洩のあるものを delete

---

## リスク

- **filter-repo は破壊的**。全タグ・全ブランチのコミット SHA が変わる。既存 clone を持つユーザー（fork bot）は再 clone が必要だが影響軽微
- **npm unpublish は推奨されない**。deprecate で警告止まり → ユーザーがアップグレードしない限り旧版は使われ続ける
- **GitHub Actions の release workflow** が secrets-scan を未配線（worklog-bridge は予定）。tag push で自動リリースされるが、本 hotfix では事前検証 (C3) で代替

## 推奨実施順

1. C1 → C2 → C3（修正 + 検証） を順次（中断可能・破壊操作なし）
2. C4 で main マージ + タグ push（不可逆になる地点）
3. C5 で develop へ伝搬（C4 後すぐ）
4. C6 は別セッションで判断 OK（C4 完了で当面の漏洩拡散は止まる）

## 関連リンク

- 設計議論整理（worklog-bridge）: `C:/dev/github/public/worklog-bridge/docs/local/secrets-scan-design/index.html`
- インシデント本体: `C:/dev/github/public/worklog-bridge/docs/local/incident-public-repo-leak/index.html`
- sweep レポート: `C:/dev/github/public/worklog-bridge/docs/local/sweep-report_2026-06-22/index.html`
- 共通 scan スクリプト: `C:/dev/github/public/worklog-bridge/scripts/secrets-scan.mjs`
- release 原則 P10: `~/.claude/guides/reference_release-pipeline.md`
- release スキル前提チェック: `C:/dev/workshop/skills/release/SKILL.md`（secrets-scan 追記済）
- 既存 hotfix/v0.3.3 ブランチ（参考運用例）: remote に存在
