# Clawork Preflight Report

- Started at: 2026-02-27T19:46:54.4086568+09:00
- Finished at: 2026-02-27T19:47:05.3564287+09:00
- Overall: passed
- Total duration: 10.95 sec
- rustc: rustc 1.88.0 (6b00bc388 2025-06-23)
- cargo: cargo 1.88.0 (873a06493 2025-05-10)
- node: v24.13.0
- node@22: v22.22.0
- Release CLI size: n/a

| Step | Status | Duration(sec) | Error |
|---|---|---:|---|
| cargo fmt --all --check | passed | 0.34 |  |
| cargo clippy --workspace --all-targets -- -D warnings | passed | 1.12 |  |
| cargo test --workspace | passed | 3.19 |  |
| UI build (Node 22) | passed | 3.39 |  |
| local smoke (daemon/API/CLI) | passed | 2.88 |  |
