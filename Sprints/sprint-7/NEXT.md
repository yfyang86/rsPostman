# Sprint 7 – Retrospective and backlog

## Done

The benchmark can hold a load shape, separates server time from transfer time, shows where
the latency mass is and how it moved during the run, and can pass or fail a build on its own
or against a baseline.

## Backlog

1. Load profiles: ramp-up / step / spike stages.
2. Open-loop scheduling with coordinated-omission correction (HDR-style recording).
3. Sub-millisecond resolution; per-second percentiles on the timeline.
4. Per-iteration variation (re-resolve `{{$random…}}` and data files) and optional scripts.
5. Save benchmark definitions with a collection; keep a history of runs and plot the trend.
6. Statistical comparison (e.g. Mann-Whitney) instead of fixed tolerance rules.
