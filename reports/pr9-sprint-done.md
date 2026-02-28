# Clawork PR-9 Sprint Done Report

- Generated at: 2026-02-27T23:55:00+09:00
- Scope: PR-9 安定化・リリース準備
- Status: done

## Evidence

1. Preflight gate report: `reports/preflight-pr9.md`
2. Release readiness report: `reports/release-readiness-pr9.md`
3. Smoke E2E script: `scripts/smoke-local-api.ps1`
4. Operational docs: `README.md`

## Acceptance Checklist

1. README/運用手順更新: pass
- README に preflight/readiness/smoke 手順、checksum 方針、既知制約を明記。

2. 既知制約明記: pass
- Node version 制約、iMessage OS 制約、OAuth 実装範囲、operator/memory 未初期化時の smoke skip を記載。

3. リリース判定レポート作成: pass
- `reports/preflight-pr9.md` は `Overall: passed`。
- `reports/release-readiness-pr9.md` は `Overall: ready`。

4. 品質ゲート通過: pass
- preflight 内で `fmt`, `clippy -D warnings`, `test`, `UI build`, `local smoke` を通過。

## Residual Constraints

1. `operator` / `memory` DB が初期化されない環境では、smoke script は該当シナリオを skip する。
2. 本判定は `-SkipReleaseBuild` で実施（リリースバイナリサイズ判定は別途 release workflow で実行）。
