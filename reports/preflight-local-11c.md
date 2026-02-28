# Clawork Preflight Report

- Started at: 2026-02-27T19:59:17.7803557+09:00
- Finished at: 2026-02-27T19:59:29.0054846+09:00
- Overall: passed
- Total duration: 11.23 sec
- rustc: rustc 1.88.0 (6b00bc388 2025-06-23)
- cargo: cargo 1.88.0 (873a06493 2025-05-10)
- node: v24.13.0
- node@22: v22.22.0
- Release CLI size budget: <= 15 MB
- Release CLI size: 2.15 MB

| Step | Status | Duration(sec) | Error |
|---|---|---:|---|
| cargo fmt --all --check | passed | 0.33 |  |
| cargo clippy --workspace --all-targets -- -D warnings | passed | 1.13 |  |
| cargo test --workspace | passed | 3.22 |  |
| UI build (Node 22) | passed | 3.17 |  |
| local smoke (daemon/API/CLI) | passed | 2.88 |  |
| cargo build --release -p clawork-cli | passed | 0.46 |  |
| release size budget (<= 15 MB) | passed | 0 |  |
