# #5 C2 offline 実装看板

> 最終更新: 2026-10-03(土) 20:10:47 JST

## context配分

| C | 種別 | 内容 | 備考/注意点 |
|---|---|---|---|
| C1 | done | 固定指示全文・基準規約・実装契約、独立branch | 設計履歴をmergeしない |
| C2 | done | 専用SQLite queue/state/lease/予算 | ACK前commit、正確なowner |
| C3 | done | fake Hub/worker/mailbox handshake | staging、verified run、one-use nonce |
| C4 | done | fake M/S、合成fixture、offline CLI | 実サービス接続なし |
| C5 | planned | 障害/負例/race/vet/build、独立レビュー | 最終head固定で記録 |

branch: `feat/dots-bridge-c2-offline-20261003`
基準: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`
固定実装指示: `66980896680bdab7d5a746ceae7f3f379a4143b8`
固定設計: `d5e0d022490d55ef26b446f5317bba06be5798b4`

受付・固定指示全文読了。Go 1.26.8 linux/amd64。初版契約を先行公開後、C2〜C4を実装。

## 検証（独立レビュー前）

- 新bridge/CLIの通常テスト、race、vet、prototype Build: 成功。
- M color / S color / M recovery / S recovery: 全て7区間完了、各fake run 1、logical delivery 1。recoveryは起動済runを維持して相手routeへ配送。
- CLI help/status/export/stop: 成功。liveは想定どおりexit 2で拒否。
- 未検証/未実施: 実MCP/Slack protocol、実dots同job続行、UX/実費用、実Hub/AI、配備、アプリ設定、購読、常設監視。
- `.omitnix/index.json`: baselineの古い生成物を保持。検証済みgenerator未導入のため未再生成、詳細はガイド。
- C5は独立固定SHAレビューと最終head CI確認までplannedを維持。

[実装契約・操作ガイド](C2_OPERATOR_GUIDE.md) / [設計PR #8](https://github.com/ishizakahiroshi/many-ai-cli/pull/8)

[実装Draft PR #10](https://github.com/ishizakahiroshi/many-ai-cli/pull/10)

## #3 Rust移植との統合境界

この実装はGo基準 `21d0bc7935a2c4696fb89ccff2e324157a528c2d` に固定し、#3のbranch/workspace/Rust/Hubには触れていない。将来developへ両方を統合するとき、CHANGELOGや生成indexの共通行は競合しうる。Rust Hubとこのbridgeのrequest/run/session/instance、active alias、GET回収、result_availableの互換性は未検証で、別の契約統合テストが必要。branchが別であることだけから実行時互換を保証しない。ここでは実接続やRust作業へ範囲を広げない。
