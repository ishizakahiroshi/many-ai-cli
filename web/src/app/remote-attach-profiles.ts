// 添付の送り先（ssh 先）のプロファイル・前回の送り先・シェル起動時の既定。
// DOM も localStorage も触らない純粋な部分だけをここに置く（fixtures から node で叩けるように）。
//
// 保存先は user_prefs の spawn.defaults（文字列だけの表。Hub へ同期され、ブラウザを替えても残る）。
// 入れ子を持てないので、一覧は JSON 文字列 1 本にして次の 3 キーへ入れる。
//   remote_attach_profiles : 名前付きの送り先の一覧（上限 MAX_REMOTE_PROFILES）
//   remote_attach_last     : 最後に決めた ssh 先（シェル起動時の「前回と同じ」が使う）
//   remote_attach_launch   : シェル起動フォームで最後に選んだ「起動時の送り先」
// 送り先そのものを 1 セッションごとに覚える仕組み（attachments.ts の remoteAttachTargetsV1）は変えない。
// 番号の使い回しで無関係なセッションへ添付が ssh で出るのを防ぐ決まりが、そちらにあるため。
// ここで決めるのは「新しいシェルの最初の送り先」だけ。

export const REMOTE_PROFILES_KEY = 'remote_attach_profiles';
export const REMOTE_LAST_KEY = 'remote_attach_last';
export const REMOTE_LAUNCH_KEY = 'remote_attach_launch';
export const MAX_REMOTE_PROFILES = 20;
export const MAX_PROFILE_NAME = 40;

export type RemoteAttachProfile = { name: string; target: string; identity: string };
export type RemoteAttachLast = { target: string; identity: string };

/** 起動フォームの選択値。'' は「決めない（最初の添付を送るときに聞く）」。 */
export const LAUNCH_CHOICE_ASK = '';
export const LAUNCH_CHOICE_LOCAL = 'local';
export const LAUNCH_CHOICE_LAST = 'last';
export const LAUNCH_CHOICE_PROFILE_PREFIX = 'profile:';

export type LaunchResolution =
  | { kind: 'ask' }
  | { kind: 'local' }
  | { kind: 'target'; target: string; identity: string };

// Hub 側（remote_attach.rs の validate_target）と同じ規則。送る前に打ち間違いを見せるためだけに使う。
export function isValidRemoteTarget(target: string): boolean {
  const host = target.startsWith('ssh://') ? target.slice('ssh://'.length) : target;
  return host.length > 0 && target.length <= 255 && !host.startsWith('-') && /^[A-Za-z0-9._@:[\]-]+$/.test(host);
}

/**
 * シェルのセッション名（`ssh -i 鍵 user@host` など）から ssh 先と鍵を読み取る。読み取り専用の「開く」の自動判定にだけ使う。
 * 接続の意味が変わる指定（-p / -J / -F / -l）があるときは、間違った所へつなぐより出さないほうがよいので null を返す。
 */
export function detectSshFromTitle(title: string): { target: string; identity: string } | null {
  const m = /(?:^|[\s/\\])ssh(?:\.exe)?\s+(.+)$/i.exec(String(title || ''));
  if (!m) return null;
  const tokens = m[1].split(/\s+/).map((s) => s.replace(/^["']|["']$/g, '')).filter(Boolean);
  const withArg = new Set(['-b', '-c', '-D', '-E', '-e', '-I', '-L', '-m', '-O', '-o', '-Q', '-R', '-S', '-W', '-w', '-B']);
  const unsafe = new Set(['-p', '-J', '-F', '-l']);
  let identity = '';
  for (let i = 0; i < tokens.length; i += 1) {
    const tok = tokens[i];
    if (unsafe.has(tok)) return null;
    if (tok === '-i') { identity = tokens[i + 1] || ''; i += 1; continue; }
    if (withArg.has(tok)) { i += 1; continue; }
    if (tok.startsWith('-')) continue;
    if (!isValidRemoteTarget(tok) || !isValidRemoteIdentity(identity)) return null;
    return { target: tok, identity };
  }
  return null;
}

/** 空は「指定なし」で有効。 */
export function isValidRemoteIdentity(identity: string): boolean {
  if (identity === '') return true;
  return identity.length <= 4096 && !identity.startsWith('-') && !/[\x00-\x1f\x7f-\x9f]/.test(identity);
}

export function isValidProfileName(name: string): boolean {
  return name.length > 0 && name.length <= MAX_PROFILE_NAME && !/[\x00-\x1f\x7f-\x9f]/.test(name);
}

function parseJson(raw: unknown): unknown {
  if (typeof raw !== 'string' || raw === '') return null;
  try {
    return JSON.parse(raw);
  } catch (_) {
    return null;
  }
}

/** 壊れた要素・規則に合わない要素・同名の 2 件目は捨てて、読める分だけ返す。 */
export function parseRemoteProfiles(raw: unknown): RemoteAttachProfile[] {
  const parsed = parseJson(raw);
  if (!Array.isArray(parsed)) return [];
  const out: RemoteAttachProfile[] = [];
  const seen = new Set<string>();
  for (const item of parsed) {
    if (!item || typeof item !== 'object') continue;
    const { name, target, identity } = item as Record<string, unknown>;
    if (typeof name !== 'string' || typeof target !== 'string') continue;
    const id = typeof identity === 'string' ? identity : '';
    if (!isValidProfileName(name) || !isValidRemoteTarget(target) || !isValidRemoteIdentity(id)) continue;
    if (seen.has(name)) continue;
    seen.add(name);
    out.push({ name, target, identity: id });
    if (out.length >= MAX_REMOTE_PROFILES) break;
  }
  return out;
}

export function serializeRemoteProfiles(profiles: RemoteAttachProfile[]): string {
  return JSON.stringify(profiles.slice(0, MAX_REMOTE_PROFILES));
}

/**
 * 同名があればその場で置き換え、無ければ末尾へ足す。
 * 規則に合わない値、または上限（MAX_REMOTE_PROFILES）を超える追加は null（呼び出し側が理由を見せる）。
 */
export function upsertRemoteProfile(
  profiles: RemoteAttachProfile[],
  next: RemoteAttachProfile,
): RemoteAttachProfile[] | null {
  const name = next.name.trim();
  const target = next.target.trim();
  const identity = next.identity.trim();
  if (!isValidProfileName(name) || !isValidRemoteTarget(target) || !isValidRemoteIdentity(identity)) return null;
  const entry = { name, target, identity };
  const index = profiles.findIndex((p) => p.name === name);
  if (index >= 0) {
    const copy = profiles.slice();
    copy[index] = entry;
    return copy;
  }
  if (profiles.length >= MAX_REMOTE_PROFILES) return null;
  return [...profiles, entry];
}

export function removeRemoteProfile(profiles: RemoteAttachProfile[], name: string): RemoteAttachProfile[] {
  return profiles.filter((p) => p.name !== name);
}

export function parseRemoteLast(raw: unknown): RemoteAttachLast | null {
  const parsed = parseJson(raw);
  if (!parsed || typeof parsed !== 'object') return null;
  const { target, identity } = parsed as Record<string, unknown>;
  if (typeof target !== 'string' || !isValidRemoteTarget(target)) return null;
  const id = typeof identity === 'string' ? identity : '';
  if (!isValidRemoteIdentity(id)) return null;
  return { target, identity: id };
}

export function serializeRemoteLast(last: RemoteAttachLast): string {
  return JSON.stringify({ target: last.target, identity: last.identity });
}

export function profileChoiceValue(name: string): string {
  return `${LAUNCH_CHOICE_PROFILE_PREFIX}${name}`;
}

/**
 * 起動フォームの選択を、新しいシェルの最初の送り先へ解く。
 * 指した先（プロファイル・前回）が無いときは黙って別の送り先にせず、「最初の添付で聞く」へ戻す。
 */
export function resolveLaunchChoice(
  choice: string,
  profiles: RemoteAttachProfile[],
  last: RemoteAttachLast | null,
): LaunchResolution {
  if (choice === LAUNCH_CHOICE_LOCAL) return { kind: 'local' };
  if (choice === LAUNCH_CHOICE_LAST) {
    return last ? { kind: 'target', target: last.target, identity: last.identity } : { kind: 'ask' };
  }
  if (choice.startsWith(LAUNCH_CHOICE_PROFILE_PREFIX)) {
    const name = choice.slice(LAUNCH_CHOICE_PROFILE_PREFIX.length);
    const hit = profiles.find((p) => p.name === name);
    return hit ? { kind: 'target', target: hit.target, identity: hit.identity } : { kind: 'ask' };
  }
  return { kind: 'ask' };
}

/** 保存済みの選択値として読める形か。読めなければ「聞く」（''）にする。 */
export function normalizeLaunchChoice(raw: unknown): string {
  if (typeof raw !== 'string') return LAUNCH_CHOICE_ASK;
  if (raw === LAUNCH_CHOICE_LOCAL || raw === LAUNCH_CHOICE_LAST) return raw;
  if (raw.startsWith(LAUNCH_CHOICE_PROFILE_PREFIX) && isValidProfileName(raw.slice(LAUNCH_CHOICE_PROFILE_PREFIX.length))) return raw;
  return LAUNCH_CHOICE_ASK;
}
