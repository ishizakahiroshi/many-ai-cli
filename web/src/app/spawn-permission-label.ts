// 起動パネルの許可モード「auto」「bypassPermissions」の文言を選ぶ。
//
// 訳の名前は spawn_permission_<種類>_<id（- は _）>。provider 専用の訳を探すのは同梱の
// CLI（宣言の builtin）だけにする。登録した AI CLI は、id が同梱の CLI に似ていても
// （例: cursor で始まる id）その CLI 専用の文言を出さず、共通の文言にする。
// 同梱の cursor-agent だけは、訳を cursor の名前で持っている。

import { providerIsBuiltin } from './provider-declarations.js';

export type PermissionLabelKind = 'auto' | 'bypass';

// 探す訳の名前（先頭から順に、訳があるものを使う）。最後は必ず共通の文言。
export function permissionLabelKeys(kind: PermissionLabelKind, provider: string): string[] {
  const base = `spawn_permission_${kind}`;
  if (!providerIsBuiltin(provider)) return [base];
  const suffix = provider.replace(/-/g, '_');
  const keys = [`${base}_${suffix}`];
  if (suffix.startsWith('cursor_')) keys.push(`${base}_cursor`);
  keys.push(base);
  return keys;
}

// translate は訳が無いとき名前をそのまま返す（i18n の t と同じ約束）。
export function permissionLabelText(
  kind: PermissionLabelKind,
  provider: string,
  translate: (key: string) => string,
): string {
  const keys = permissionLabelKeys(kind, provider);
  for (const key of keys.slice(0, -1)) {
    const text = translate(key);
    if (text && text !== key) return text;
  }
  return translate(keys[keys.length - 1]);
}
