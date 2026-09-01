// PTY 出力から DEC マウス追跡モードを追跡する純関数。
//
// xterm.js は選択を維持するため parse 後にマウス追跡をローカルで解除するので、
// term.modes.mouseTrackingMode は CLI が追跡を有効にしたかの判定には使えない。
// このモジュールは表示バイト列を変更せず、PTY から来た DECSET / DECRST だけを観測する。

const ESC = 0x1b;
const CSI = 0x5b;
const PRIVATE = 0x3f;
const SET_MODE = 0x68; // h
const RESET_MODE = 0x6c; // l

export interface MouseModeTrackerState {
  carry: Uint8Array;
  tracking: boolean;
  sgr: boolean;
  urxvt: boolean;
}

export type WheelEncoding = 'sgr' | 'x10' | 'page';

export function initialMouseModeTrackerState(): MouseModeTrackerState {
  return {
    carry: new Uint8Array(0),
    tracking: false,
    sgr: false,
    urxvt: false,
  };
}

function isDigit(byte: number): boolean {
  return byte >= 0x30 && byte <= 0x39;
}

function applyMode(
  parameter: number,
  enabled: boolean,
  state: { tracking: boolean; sgr: boolean; urxvt: boolean },
): void {
  switch (parameter) {
    case 9:
    case 1000:
    case 1002:
    case 1003:
      state.tracking = enabled;
      break;
    case 1005:
      // UTF-8 mouse encoding is tracked for completeness, but the sender currently
      // uses X10 whenever SGR is unavailable (the same fallback as the existing plan).
      break;
    case 1006:
      state.sgr = enabled;
      break;
    case 1015:
      state.urxvt = enabled;
      break;
    default:
      break;
  }
}

function applyMouseModeSequence(
  bytes: Uint8Array,
  start: number,
  end: number,
  enabled: boolean,
  state: { tracking: boolean; sgr: boolean; urxvt: boolean },
): void {
  let parameter = 0;
  let hasDigit = false;
  for (let i = start; i < end; i++) {
    const byte = bytes[i];
    if (isDigit(byte)) {
      parameter = parameter * 10 + byte - 0x30;
      hasDigit = true;
      continue;
    }
    if (byte === 0x3b) { // ;
      applyMode(hasDigit ? parameter : 0, enabled, state);
      parameter = 0;
      hasDigit = false;
    }
  }
  if (hasDigit) applyMode(parameter, enabled, state);
}

/**
 * PTY の 1 チャンクからマウス追跡状態を更新する。
 * 不完全な CSI ? ... h/l は carry に残し、次のチャンクと連結してから解釈する。
 */
export function scanMouseModePure(
  bytes: Uint8Array,
  state: MouseModeTrackerState,
): MouseModeTrackerState {
  const combined = new Uint8Array(state.carry.length + bytes.length);
  combined.set(state.carry, 0);
  combined.set(bytes, state.carry.length);

  const next = {
    tracking: state.tracking,
    sgr: state.sgr,
    urxvt: state.urxvt,
  };
  let i = 0;

  while (i < combined.length) {
    if (combined[i] !== ESC) {
      i++;
      continue;
    }
    if (i + 1 >= combined.length) {
      return { carry: combined.slice(i), ...next };
    }
    if (combined[i + 1] !== CSI) {
      i++;
      continue;
    }
    if (i + 2 >= combined.length) {
      return { carry: combined.slice(i), ...next };
    }
    if (combined[i + 2] !== PRIVATE) {
      i++;
      continue;
    }

    const parameterStart = i + 3;
    if (parameterStart >= combined.length) {
      return { carry: combined.slice(i), ...next };
    }
    let final = parameterStart;
    while (final < combined.length && (isDigit(combined[final]) || combined[final] === 0x3b)) {
      final++;
    }
    if (final >= combined.length) {
      return { carry: combined.slice(i), ...next };
    }
    if (combined[final] !== SET_MODE && combined[final] !== RESET_MODE) {
      i++;
      continue;
    }
    applyMouseModeSequence(combined, parameterStart, final, combined[final] === SET_MODE, next);
    i = final + 1;
  }

  return { carry: new Uint8Array(0), ...next };
}

function centeredCoordinate(value: number): number {
  return Math.max(1, Math.floor((Number.isFinite(value) ? value : 0) / 2));
}

function x10Coordinate(value: number): number {
  return Math.min(223, centeredCoordinate(value));
}

/** 代替画面へ送る 1 ノッチ分のホイール／PageUp・PageDown シーケンスを作る。 */
export function encodeWheelSeq(
  direction: number,
  cols: number,
  rows: number,
  mode: WheelEncoding,
): string {
  if (mode === 'page') return direction < 0 ? '\x1b[5~' : '\x1b[6~';

  const button = direction < 0 ? 64 : 65;
  const col = centeredCoordinate(cols);
  const row = centeredCoordinate(rows);
  if (mode === 'sgr') return `\x1b[<${button};${col};${row}M`;

  return `\x1b[M${String.fromCharCode(32 + button)}${String.fromCharCode(32 + x10Coordinate(cols))}${String.fromCharCode(32 + x10Coordinate(rows))}`;
}
