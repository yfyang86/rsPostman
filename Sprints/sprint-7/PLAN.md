# Sprint 7 – Benchmark enhancements

Request: "enhance the benchmark testing". Interpreted as: make the benchmark a tool that can
answer the questions people actually run benchmarks for, and make its results checkable in CI.

## Gaps in the sprint 5 state

| Gap | Why it matters |
|-----|----------------|
| Only "N iterations as fast as possible" | No way to hold a load for a period or at a chosen rate |
| First requests include connection setup | Skews small runs; no warm-up |
| Only total latency | Cannot tell a slow server (time to first byte) from a large download |
| No status breakdown, distribution or timeline | Percentiles hide bimodal behaviour and drift during a run |
| No pass/fail criteria | Cannot gate a build on performance |
| No comparison between runs | "Did it get slower?" needed manual reading of two reports |
| JSON report only; GUI showed numbers at the end | Hard to share; no feedback during long runs |

## Scope

| # | Feature |
|---|---------|
| 1 | `core::bench` (split out of `profile`): `BenchOptions` – iterations **or** duration, concurrency, target rate, per-worker delay, warm-up rounds, keep-alive on/off |
| 2 | Per-sample start offset and **time to first byte** (`http::Response::ttfb_ms`, also available to the rest of the app) |
| 3 | Report: P75 and P99.9, TTFB stats, status-code counts, error rate, adaptive latency histogram, per-second timeline; sample list capped at 50 000 in the report while statistics cover all |
| 4 | **Thresholds** (`p95<200ms`, `error_rate<=1%`, `rps>=50`, `ttfb_p99<1s` …), validated before the run; exit code follows them |
| 5 | **Baseline comparison** with tolerance; regression gives exit 1 |
| 6 | Reports: JSON (baseline format), CSV of samples, standalone HTML with charts |
| 7 | CLI: new options, text histogram, progress line for long runs |
| 8 | GUI: new inputs, interim results while running, two charts (latency distribution, mean latency over time) with tooltips, keyboard access and table view, thresholds, status table, report links, compare with an uploaded baseline, stop |
| 9 | Tests at unit, CLI and Playwright level; README section with the measurement method and its limits |

## Design notes

* Rate limiting is slot-based: work item *n* may not start before `n / rate` seconds. No
  catching up in bursts; if workers are saturated the achieved rate is simply lower and visible.
* Charts follow one small shared module (`ui/js/charts.js`) used by the GUI and embedded
  verbatim into the HTML report, so both show the same thing. One series per chart, one axis,
  a validated colour, hover + focus tooltips, and a "Show data" table for every chart.
* Interim results in the GUI are computed by the same `summarize()` as the final report.

## Out of scope

Ramp-up stages / load profiles, distributed load generation, coordinated-omission correction,
running scripts during a benchmark, per-request data variation.
