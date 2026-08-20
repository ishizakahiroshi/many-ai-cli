// index.ts — sink 登録の唯一の入口。
//
// 現役の観測 module を副作用 import として並べるだけ。module 側の top-level で
// probe.ts の registerProbeSink() を呼ぶ。ここに並ぶ module は
// instrumentation.json の status=active と一致していなければならない
// （check-instrumentation.mjs の第 4 パスが両方向で検査する）。
//
// リリースビルドでは web/scripts/build.mjs がこのファイルを空の
// dist/debug/index.js に差し替えるため、下の import 先は成果物に入らない。
//
// 撤去（make debug-purge）はこのファイルを import 0 本の状態へ戻し、
// 対応する module ファイルを削除する。

import './approval-identity.js';
import './mobile-view.js';
