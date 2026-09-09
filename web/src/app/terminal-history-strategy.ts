// terminal-history-strategy.ts — 端末の履歴表示方式を Provider 名から切り離して決める。
//
// terminal.ts / multi-pane.ts / detached-grid.ts は Provider ID を直接比較せず、ここで
// 正規化した capability と現在の buffer 種別だけを見る。未知の Provider は安全な
// auto profile を使い、専用履歴 surface が必要な Provider だけ adapter profile で宣言する。

export type TerminalHistoryStrategy =
  | 'native-scrollback'
  | 'alt-input-confirmed'
  | 'transcript-viewer'
  | 'none';

export type TerminalBufferType = 'normal' | 'alternate' | 'unknown';
export type TranscriptViewerKind = 'grok-chat';

export interface TerminalHistoryCapabilities {
  altInput: 'auto' | 'none';
  transcriptViewer?: TranscriptViewerKind;
}

const DEFAULT_CAPABILITIES: TerminalHistoryCapabilities = { altInput: 'auto' };

// Provider 固有事情は adapter profile の宣言へ閉じ込める。端末の各入力経路へ
// `provider === ...` を増殖させないための唯一の対応表。
const CAPABILITY_PROFILES: Readonly<Record<string, TerminalHistoryCapabilities>> = {
  // Grok の TUI は履歴内で次の応答を画面外へ描くため、PTY への履歴入力を使わず
  // chat_history.jsonl 由来の専用 viewer を使う。
  grok: { altInput: 'none', transcriptViewer: 'grok-chat' },
};

export function terminalHistoryCapabilitiesForProvider(provider: unknown): TerminalHistoryCapabilities {
  const key = typeof provider === 'string' ? provider.trim().toLowerCase() : '';
  return CAPABILITY_PROFILES[key] || DEFAULT_CAPABILITIES;
}

export function resolveTerminalHistoryStrategy(
  bufferType: TerminalBufferType,
  capabilities: TerminalHistoryCapabilities,
  nativeScrollbackAvailable = true,
): TerminalHistoryStrategy {
  if (bufferType === 'normal') {
    if (!nativeScrollbackAvailable && capabilities.transcriptViewer) return 'transcript-viewer';
    return 'native-scrollback';
  }
  if (bufferType !== 'alternate') return 'none';
  if (capabilities.transcriptViewer) return 'transcript-viewer';
  if (capabilities.altInput !== 'none') return 'alt-input-confirmed';
  return 'none';
}

export interface TerminalScreenSnapshot {
  bufferType: TerminalBufferType;
  cols: number;
  rows: number;
  lines: string[];
}

function isMeaningfulAltLine(line: string): boolean {
  const trimmed = line.trim();
  if (trimmed.length >= 4) return true;
  // 全角文字を含む場合は 2 文字以上で十分な情報量（表示幅 >= 4）を持つ
  return /[^\x00-\x7F]/.test(trimmed) && trimmed.length >= 2;
}

function areAltLinesMatching(a: string, b: string): boolean {
  if (a === b) return true;
  // 全角折り返しマージンやスクロールバー記号などの末尾 1〜2 文字の揺らぎを許容
  const diff = Math.abs(a.length - b.length);
  return diff <= 2 && (a.startsWith(b) || b.startsWith(a));
}

/**
 * 送信前後で「同じ寸法の代替画面の本文」が操作方向へずれたときだけ履歴移動を確定する。
 * cursor 位置や PTY 出力件数は本文が同じでも変わり得るため、成功根拠に含めない。
 * resize や main buffer への復帰もスクロール成功ではないので false に倒す。
 */
export function isConfirmedAltScreenChange(
  before: TerminalScreenSnapshot,
  after: TerminalScreenSnapshot,
  direction: number,
): boolean {
  if (before.bufferType !== 'alternate' || after.bufferType !== 'alternate') return false;
  if (before.cols !== after.cols || before.rows !== after.rows) return false;
  if (before.lines.length !== after.lines.length) return false;
  const oldLines = before.lines.map(line => line.trimEnd());
  const newLines = after.lines.map(line => line.trimEnd());
  if (oldLines.every((line, index) => line === newLines[index])) return false;

  // 上へ移動すると旧画面の行は新画面の下側へ、下へ移動すると上側へずれる。
  // 同じ座標の差分だけを見ると spinner・status 行の再描画を成功と誤認するため、
  // 方向に沿った非空行の overlap を要求する。通常画面高では偶然一致を避けるため 2 行、
  // 極端に小さい fixture / terminal だけ 1 行を下限にする。
  // また、十分な長さを持つ特徴的な行（日本語 4 文字以上または英数 8 文字以上）が 1 行でも
  // 正しい方向へ shift していれば確定とする。
  const requiredMatches = oldLines.length >= 4 ? 2 : 1;
  for (let shift = 1; shift < oldLines.length; shift++) {
    let matches = 0;
    let hasStrongMatch = false;
    for (let i = 0; i + shift < oldLines.length; i++) {
      const oldLine = direction < 0 ? oldLines[i] : oldLines[i + shift];
      const newLine = direction < 0 ? newLines[i + shift] : newLines[i];
      if (!isMeaningfulAltLine(oldLine) || !areAltLinesMatching(oldLine, newLine)) continue;
      matches++;
      const trimmed = oldLine.trim();
      if (trimmed.length >= 8 || (/[^\x00-\x7F]/.test(trimmed) && trimmed.length >= 4)) {
        hasStrongMatch = true;
      }
      if (matches >= requiredMatches || (matches >= 1 && hasStrongMatch)) return true;
    }
  }
  return false;
}
