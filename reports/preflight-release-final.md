# Clawork Preflight Report

- Started at: 2026-02-28T00:02:49.2700091+09:00
- Finished at: 2026-02-28T00:03:26.2591466+09:00
- Overall: passed
- Total duration: 36.99 sec
- rustc: rustc 1.88.0 (6b00bc388 2025-06-23)
- cargo: cargo 1.88.0 (873a06493 2025-05-10)
- node: v24.13.0
- node@22: v22.22.0
- Release CLI size budget: <= 15 MB
- Release CLI size: 2.2 MB

| Step | Status | Duration(sec) | Error |
|---|---|---:|---|
| cargo fmt --all --check | passed | 0.39 |  |
| cargo clippy --workspace --all-targets -- -D warnings | passed | 1.18 |  |
| cargo test --workspace | passed | 3.47 |  |
| UI build (Node 22) | passed | 3.44 |  |
| local smoke (daemon/API/CLI) | passed | 2.94 |  |
| cargo build --release -p clawork-cli | passed | 25.53 |  |
| release size budget (<= 15 MB) | passed | 0.01 |  |
