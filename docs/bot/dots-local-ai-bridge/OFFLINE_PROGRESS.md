# #5 C2 offline 実装看板

> 最終更新: 2026-10-03(土) 19:39:36

## context配分

| C | 種別 | 内容 | 備考/注意点 |
|---|---|---|---|
| C1 | done | 固定指示全文・基準規約・実装契約、独立branch | 設計履歴をmergeしない |
| C2 | planned | 専用SQLite queue/state/lease/予算 | ACK前commit、正確なowner |
| C3 | planned | fake Hub/worker/mailbox handshake | staging、verified run、one-use nonce |
| C4 | planned | fake M/S、合成fixture、offline CLI | 実サービス接続なし |
| C5 | planned | 障害/負例/race/vet/build、独立レビュー | 最終head固定で記録 |

branch: `feat/dots-bridge-c2-offline-20261003`
基準: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`
固定実装指示: `66980896680bdab7d5a746ceae7f3f379a4143b8`
固定設計: `d5e0d022490d55ef26b446f5317bba06be5798b4`

受付・固定指示全文読了。Go 1.26.8 linux/amd64確認。初版は実装前の契約・看板のみ。テスト/Build/実機/配備/自動監視はまだ実施していない。後続で新prototypeの内部テストとBuildのみ行う。

[実装契約・操作ガイド](C2_OPERATOR_GUIDE.md) / [設計PR #8](https://github.com/ishizakahiroshi/many-ai-cli/pull/8)
