// workflow-progress.ts — Workflow ライブ進捗の「純粋パーサ」。
//
// 同一セッションで走る Claude Code の Workflow（/workflows 相当の再描画 TUI）の
// xterm バッファ文字列を受け取り、フェーズ / エージェント label / 走行・完了状態へ
// 構造化する。DOM も i18n も触らない純関数だけを置く（approval-parser.ts と同じ思想）。
// → Node の node:test からそのまま import して fixtures 検証できる。
//
// ⚠ 重要な前提（plan_workflow-progress-modal.md「割り切り」）:
//   Workflow の進捗表示フォーマットは CLI 公式仕様ではなく、更新で変わり得る。
//   ここは VT スクレイプ前提の **ベストエフォート**。認識できなければ detected:false を返し、
//   呼び出し側はボタンを出さない / モーダルに「解釈できませんでした」を出す方針。
//   実フォーマットは実機 1 サンプルで較正する（CALIBRATE コメント箇所）。

export type WfAgentState = 'running' | 'done' | 'failed' | 'pending';

export interface WfAgent {
  label: string;
  state: WfAgentState;
  /** 行頭の生グリフ（スピナー回転を検知してフレーム凍結判定に使う。表示には使わない）。 */
  glyph: string;
}

export interface WfPhase {
  /** フェーズ / グループ見出し（▸ 行や phase() タイトル由来。無名なら ''）。 */
  title: string;
  agents: WfAgent[];
}

export interface WorkflowProgress {
  /** Workflow 進捗ブロックを認識できたか（false ならボタンを出さない）。 */
  detected: boolean;
  /** 走行中のエージェントが 1 件以上あるか（false かつ detected=true は「完了」）。 */
  running: boolean;
  /**
   * 「N/M agents done」サマリー行が存在し未完（done<total）か。
   * これは Claude Code がバックグラウンド Workflow 実行中に毎秒更新するステータス行で、
   * 走行の権威的な証拠。true の間は呼び出し側はフレーム凍結 settle を行ってはならない
   * （インラインツリーが再描画されず固まっても、実際は走行中のため）。
   */
  live: boolean;
  /** Workflow 名（⚙ 行 / workflow: 行から拾えれば。無ければ ''）。 */
  name: string;
  phases: WfPhase[];
  runningCount: number;
  doneCount: number;
  failedCount: number;
  totalCount: number;
  /** 完了率（0..100）。明示の % / M/N があればそれを、無ければ done/total から算出。 */
  percent: number | null;
  /**
   * フレーム凍結判定用の署名。生グリフ（スピナー回転）と状態・件数を含む。
   * 生きている Workflow はスピナーが回るので毎フレーム変化し、完了して静止した
   * フレームでは不変になる。呼び出し側は連続一致で「実質完了」とみなす。
   */
  frameSig: string;
}

// ── グリフ定義（CALIBRATE: 実機サンプルで増減する） ───────────────────────────
// スピナー（走行中）: braille スピナー一式 + 円弧スピナー。
const SPINNER_GLYPHS = '⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏⣾⣽⣻⢿⡿⣟⣯⣷◐◓◑◒◜◝◞◟';
const DONE_GLYPHS = '✓✔';
const FAILED_GLYPHS = '✗✘';
const PENDING_GLYPHS = '○◌◯';

// 行頭のツリー描画・インデント記号（除去対象）。`▸`/`▹`/`‣` はグループ box の見出し記号でもあり、
// 除去後の残りをフェーズ見出しとして扱う。
const TREE_PREFIX_RE = /^[\s│├└─╰╭╮╯┃┣┗┏┓┛┃┆┊▕▏▸▹‣•·]+/;

const ANSI_RE = /\x1b\[[0-9;?]*[ -/]*[@-~]/g;

function stripAnsi(s: string): string {
  return String(s == null ? '' : s).replace(ANSI_RE, '');
}

function stripTree(line: string): string {
  return stripAnsi(line).replace(TREE_PREFIX_RE, '');
}

// ラベル末尾の桁揃え列（経過時間・トークン等が 2 個以上の空白で区切られて続く）を落とし、
// 内部の連続空白を 1 個へ畳む。
function cleanLabel(raw: string): string {
  return String(raw || '')
    .replace(/\s{2,}.*$/, '')
    .replace(/\s+/g, ' ')
    .trim();
}

function cleanTitle(raw: string): string {
  return stripAnsi(raw).replace(TREE_PREFIX_RE, '').replace(/\s+/g, ' ').trim().slice(0, 80);
}

// 行頭の状態グリフ → エージェント行を判定。グリフ + 空白 + ラベル の形のみ採用する。
function matchAgentRow(stripped: string): WfAgent | null {
  const first = stripped.charAt(0);
  if (!first) return null;
  let state: WfAgentState | null = null;
  if (SPINNER_GLYPHS.includes(first) || first === '●') state = 'running';
  else if (DONE_GLYPHS.includes(first)) state = 'done';
  else if (FAILED_GLYPHS.includes(first)) state = 'failed';
  else if (PENDING_GLYPHS.includes(first)) state = 'pending';
  if (!state) return null;
  const rest = stripped.slice(1);
  // グリフ直後は空白区切りであること（"✓done" のような語中マッチを避ける）。
  if (rest && !/^\s/.test(rest)) return null;
  const label = cleanLabel(rest);
  if (!label || label.length > 200) return null;
  return { label, state, glyph: first };
}

// Workflow 見出し行か（⚙ か "workflow" を含む）。name 抽出も兼ねる。
function matchHeader(stripped: string): { name: string } | null {
  const hasGear = stripped.includes('⚙');
  const hasWord = /\bworkflow\b/i.test(stripped);
  if (!hasGear && !hasWord) return null;
  let name = stripped
    .replace(/⚙/g, '')
    .replace(/\bworkflow\b\s*[:：]?/i, '')
    .replace(/\(.*?\)/g, '')
    .replace(/\s+/g, ' ')
    .trim();
  // "running" 等の状態語だけが残ったら名前扱いしない。
  if (/^(running|done|complete|completed|in progress)$/i.test(name)) name = '';
  return { name: name.slice(0, 60) };
}

// サマリー行（"3 agents running · 5 done" / "60%" / "5/12" 等）から percent を拾う。
// 「N/M agents done」サマリー行（Claude Code のバックグラウンド Workflow ステータス行）。
// 例: "◑ ur… 43/90 agents done · 5m 25s · ↓ 1.9m"。done/total と raw（経過時間込み）を返す。
const AGENTS_SUMMARY_RE = /\b(\d{1,4})\s*\/\s*(\d{1,4})\s+agents?\b/i;
function matchAgentsSummary(stripped: string): { done: number; total: number } | null {
  const m = stripped.match(AGENTS_SUMMARY_RE);
  if (!m) return null;
  const done = parseInt(m[1], 10);
  const total = parseInt(m[2], 10);
  if (!(total > 0) || done < 0 || done > total) return null;
  return { done, total };
}

function matchSummaryPercent(stripped: string): number | null {
  const pct = stripped.match(/(\d{1,3})\s*%/);
  if (pct) {
    const v = parseInt(pct[1], 10);
    if (v >= 0 && v <= 100) return v;
  }
  const frac = stripped.match(/\b(\d{1,4})\s*\/\s*(\d{1,4})\b/);
  if (frac) {
    const done = parseInt(frac[1], 10);
    const total = parseInt(frac[2], 10);
    if (total > 0 && done >= 0 && done <= total) return Math.round((done / total) * 100);
  }
  return null;
}

function emptyResult(): WorkflowProgress {
  return {
    detected: false,
    running: false,
    live: false,
    name: '',
    phases: [],
    runningCount: 0,
    doneCount: 0,
    failedCount: 0,
    totalCount: 0,
    percent: null,
    frameSig: '',
  };
}

/**
 * xterm バッファ行配列から Workflow 進捗を構造化する。
 * 認識できなければ detected:false（呼び出し側はボタンを出さない）。
 */
export function parseWorkflowProgress(lines: string[]): WorkflowProgress {
  const src = Array.isArray(lines) ? lines.map(stripAnsi) : [];
  if (src.length === 0) return emptyResult();

  // scrollback に古いブロックが残っても末尾の生きたブロックを採るため、
  // 最後の見出し行（⚙ / workflow）以降だけを解析対象にする。
  let headerIdx = -1;
  let headerName = '';
  for (let i = src.length - 1; i >= 0; i--) {
    const h = matchHeader(stripTree(src[i]));
    if (h) { headerIdx = i; headerName = h.name; break; }
  }
  // 見出しが無くても「N/M agents done」サマリー行があればそこを起点に解析する
  // （バックグラウンド Workflow はインラインの ⚙ ツリーをビューポートに出さないため）。
  if (headerIdx === -1) {
    for (let i = src.length - 1; i >= 0; i--) {
      if (matchAgentsSummary(stripTree(src[i]))) { headerIdx = i; break; }
    }
  }
  if (headerIdx === -1) return emptyResult();

  const block = src.slice(headerIdx);
  const phases: WfPhase[] = [];
  let current: WfPhase | null = null;
  let pendingTitle: string | null = null;
  let explicitPercent: number | null = null;
  // 「N/M agents done」サマリー（権威信号）。最後の値と生行（経過時間込み）を保持。
  let summary: { done: number; total: number } | null = null;
  let summaryRaw = '';

  for (let i = 0; i < block.length; i++) {
    const stripped = stripTree(block[i]);
    if (!stripped) continue;

    // agents サマリーは見出し行自体でも拾う（背景実行時は起点行が summary 行のため）。
    const sum = matchAgentsSummary(stripped);
    if (sum) { summary = sum; summaryRaw = stripped; continue; }

    if (i === 0) continue; // 見出し行自体はスキップ（name は採取済み）

    const agent = matchAgentRow(stripped);
    if (agent) {
      if (!current || pendingTitle !== null) {
        current = { title: pendingTitle != null ? pendingTitle : '', agents: [] };
        phases.push(current);
        pendingTitle = null;
      }
      current.agents.push(agent);
      continue;
    }

    // サマリー行（percent）は最後の値を優先採用。
    const pct = matchSummaryPercent(stripped);
    if (pct !== null) { explicitPercent = pct; continue; }

    // それ以外の非空行はフェーズ / グループ見出しの候補として保留。
    // 次にエージェント行が来たときだけフェーズ化する（無関係な本文行をフェーズにしない）。
    pendingTitle = cleanTitle(block[i]);
  }

  // 集計はエージェント状態から算出（サマリー文字列より信頼できる）。
  let runningCount = 0;
  let doneCount = 0;
  let failedCount = 0;
  let totalCount = 0;
  for (const p of phases) {
    for (const a of p.agents) {
      totalCount++;
      if (a.state === 'running') runningCount++;
      else if (a.state === 'done') doneCount++;
      else if (a.state === 'failed') failedCount++;
    }
  }

  // 「N/M agents done」サマリーがあれば、走行/件数はそれを権威として採用する
  // （バックグラウンド実行はインラインのエージェント行が出ない / 古い静止フレームのため）。
  let running: boolean;
  let live = false;
  if (summary) {
    totalCount = summary.total;
    doneCount = summary.done;
    runningCount = Math.max(0, summary.total - summary.done);
    failedCount = 0;
    running = summary.done < summary.total;
    live = running; // done<total の生サマリー＝走行中の権威的証拠
  } else {
    running = runningCount > 0;
  }

  if (totalCount === 0) return emptyResult();

  let percent = explicitPercent;
  if (summary) {
    percent = Math.round((summary.done / summary.total) * 100);
  } else if (percent === null) {
    const finished = doneCount + failedCount;
    percent = totalCount > 0 ? Math.round((finished / totalCount) * 100) : null;
  }

  // フレーム署名: 生グリフ込みのエージェント行 + 生サマリー行（経過時間込み）。
  // 走行中は経過時間が毎秒変わるので frameSig が変化し、凍結誤判定を防ぐ。
  const frameSig = phases
    .map(p => p.title + ':' + p.agents.map(a => a.glyph + a.label).join(','))
    .join('|') + '||S:' + summaryRaw;

  return {
    detected: true,
    running,
    live,
    name: headerName,
    phases,
    runningCount,
    doneCount,
    failedCount,
    totalCount,
    percent,
    frameSig,
  };
}
