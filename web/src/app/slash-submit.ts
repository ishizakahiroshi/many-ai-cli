// 「/」で始まる入力まわりの、provider ごとの違い（宣言から読む）。
//
// - 1 行の「/コマンド」を、本文と Enter の 1 かたまり（once）で送るか、貼り付け＋別送の Enter（paste）で送るか。
//   宣言 input.slash_submit。既定は once（同梱 7 本の従来の動き）。コマンド候補の画面が最初の Enter を
//   「候補の確定」に使う CLI では、once だと送信にならないことがあるため、paste を宣言できる。
// - 「/」の候補のうち、既定の固定リストを出してよい provider か。固定リストは Claude 向けの内容なので、
//   同梱の provider にだけ出し、登録した AI CLI には別の CLI 向けの候補を出さない（宣言が無ければ候補なし）。
// - 候補を選んだ瞬間に即送信する /clear・/model・/models を、どの provider で許すか。
//
// DOM を持たない純粋な関数だけにして、web/tests から直接試せるようにしてある。

import { hasProviderDeclarations, providerDeclaration, providerIsBuiltin } from './provider-declarations.js';

export type SlashSubmitMode = 'once' | 'paste';

export function providerSlashSubmit(id: unknown): SlashSubmitMode {
  return providerDeclaration(id)?.input?.slash_submit === 'paste' ? 'paste' : 'once';
}

// 本文送信の共通判定。textPart は PTY へ送る文字列、deferEnter は確定の Enter を別送するか。
// 生送信（本文 + \r を 1 かたまり）のまま残すのは「キー入力として届くべきもの」だけ:
// shell・空・1 文字・数字のみ・「/」始まりの 1 行（slash_submit が once のとき）。
export function buildBodySubmit(provider: string, rawText: string): { textPart: string; deferEnter: boolean } {
  if (rawText.includes('\n')) {
    // 複数行は provider を問わずペースト包み（\n の途中 Enter 解釈を防ぐ）
    return { textPart: '\x1b[200~' + rawText + '\x1b[201~', deferEnter: true };
  }
  const slashAsKey = rawText.startsWith('/') && providerSlashSubmit(provider) === 'once';
  const keyLike = rawText === '' || rawText.length === 1 || /^[0-9]+$/.test(rawText) || slashAsKey;
  if (provider === 'shell' || keyLike) {
    return { textPart: rawText + '\r', deferEnter: false };
  }
  return { textPart: '\x1b[200~' + rawText + '\x1b[201~', deferEnter: true };
}

// 既定の固定リストを候補に出してよいか。同梱の provider だけ。宣言がまだ 1 つも届いていない間は、
// どの provider か分からないので従来どおり（同梱の挙動を変えない）。
export function mayUseSlashFallbackList(provider: unknown): boolean {
  if (!hasProviderDeclarations()) return true;
  return providerIsBuiltin(provider);
}

const QUICK_SEND_COMMANDS = new Set(['/clear', '/model', '/models']);

// 候補を選んだ瞬間に即送信してよいコマンドか。同梱の provider は従来どおり。登録した provider は、
// その provider が自分の候補（宣言した取得元か一覧）として持っているコマンドだけ。
export function slashCommandQuickSends(provider: unknown, cmd: string, candidates: readonly { cmd: string }[]): boolean {
  if (!QUICK_SEND_COMMANDS.has(cmd)) return false;
  if (!hasProviderDeclarations() || providerIsBuiltin(provider)) return true;
  return candidates.some((candidate) => candidate.cmd === cmd);
}
