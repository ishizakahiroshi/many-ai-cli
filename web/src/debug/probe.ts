// probe.ts — 恒久の観測フック。sink が 1 つも無ければ完全な no-op。
//
// 呼び出し点（製品コード側の記録点）はここを経由し、そのまま残り続ける。
// 着脱するのは sink（収集して出す側）だけで、sink は index.ts が読み込む
// module が自分で registerProbeSink() する。
//
// リリースビルドでは __MAI_DEBUG__ が false に畳まれるため、下の関数本体ごと
// 到達不能になる。加えて build.mjs が index.ts を空の出力に差し替えるので、
// sink 側の module は成果物に 1 バイトも入らない。
//
// 台帳（instrumentation.json）が登録を要求するのは sink 側であって、この
// ファイル自身と呼び出し点ではない（check-instrumentation.mjs の EXCLUDE）。

declare const __MAI_DEBUG__: boolean;

export type ProbeFields = Record<string, unknown>;
export type ProbeSink = (channel: string, fields: ProbeFields) => void;

const sinks = new Map<string, ProbeSink>();

export function registerProbeSink(channel: string, sink: ProbeSink): void {
  sinks.set(channel, sink);
}

export function probeOn(channel: string): boolean {
  return __MAI_DEBUG__ && sinks.has(channel);
}

/** 1 点で完結する記録。sink が無ければ fields は 1 度も評価されない。 */
export function probe(channel: string, fields: () => ProbeFields): void {
  if (!__MAI_DEBUG__) return;
  const sink = sinks.get(channel);
  if (!sink) return;
  sink(channel, fields());
}

export interface ProbeHandle {
  emit(fields: () => ProbeFields): void;
}

/**
 * 処理の前後を突き合わせる記録。before は sink がある時だけ即座に評価される。
 *
 * 「何が起きたか」（置き換えが起きたのか、補完だったのか、変化なしか）の
 * 意味づけは sink 側が before / after の差分から計算する。呼び出し点に
 * 観測用の状態機械を置かない。
 */
export function probeScope(channel: string, before: () => ProbeFields): ProbeHandle | null {
  if (!__MAI_DEBUG__) return null;
  const sink = sinks.get(channel);
  if (!sink) return null;
  const head = before();
  return {
    emit(after: () => ProbeFields): void {
      sink(channel, { ...head, ...after() });
    },
  };
}
