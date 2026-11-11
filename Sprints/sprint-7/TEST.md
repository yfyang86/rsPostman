# Sprint 7 – Test & fix report

## Regression run

| Layer | Count | Result |
|-------|------:|--------|
| Rust unit tests – `bench` (7): error accounting and status codes, latency against a known server delay, duration and rate shaping, thresholds, histogram / timeline / CSV, baseline comparison; `bench_report` (1) | 60 | pass |
| HTTP / engine integration | 6 | pass |
| CLI end-to-end – new `tests/bench.rs` (3): load shapes and input validation, thresholds and exit codes, reports and comparison | 14 | pass |
| Playwright – +2 in `perf.spec.js`: fixed time at a rate with thresholds, charts (tooltip, keyboard, table view), all three report downloads, baseline comparison incl. a refused junk file and a regression; stopping a run | 37 | pass, two parallel runs |
| `cargo clippy --all-targets -- -D warnings`, desktop crate build | – | clean |

## How the measurements themselves were checked

* Against the echo server's `/delay/60`: minimum latency ≥ 60 ms, time to first byte carries
  the delay, three workers finish six requests in about two rounds.
* `--rate 20` for one second yields 15–21 requests, spread over the second.
* A duration run ends on time; `-n 10 --rate 50` takes at least 170 ms.
* In the GUI: 30 req/s for 2 s gives 45–62 requests.

These are loose bounds on purpose (the tests run on shared CI-like machines); they establish
that the load shape is honoured, not that timings are precise to the millisecond.

## Defects and corrections during the sprint

| # | Found by | Problem | Fix |
|---|----------|---------|-----|
| 1 | screenshot review | With fixed 1-2-5 buckets, a typical narrow latency distribution collapsed into a single bar | Adaptive histogram: equal-width buckets with a clean step for narrow distributions, logarithmic bounds for wide ones |
| 2 | screenshot review | Numeric table headers in the HTML report were left-aligned over right-aligned numbers | Headers aligned with their columns |
| 4 | README verification | Comparing two identical 0.1 s runs against each other reported a "requests/s" regression of 12 %: pure noise | Throughput is only judged when both runs lasted at least one second; unit test added |
| 3 | review | Printing one line per request floods the terminal on long runs | Requests are listed only for short runs; otherwise one progress line per second |

## Limits (also in the README)

* A closed-loop client on one machine: at high rates the client becomes the bottleneck, and
  latency under overload is under-reported compared with an open-loop generator (no
  coordinated-omission correction).
* Latency resolution is 1 ms.
* The timeline chart shows the mean per second; percentiles per second are not computed.
* Interim results on very long runs use the most recent 300 000 samples.
* Comparison rules are simple fixed rules (see README), not a statistical test.
