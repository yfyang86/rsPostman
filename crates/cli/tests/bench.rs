//! The `benchmark` command: load shapes, thresholds, reports and baseline comparison.

mod common;
use common::Cli;

fn requests(stdout: &str) -> usize {
    stdout.lines().find(|l| l.contains(" requests in ")).and_then(|l| l.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("no summary line in {stdout}"))
}

#[test]
fn load_shapes_and_output() {
    let srv = rspostman_core::echo::start("127.0.0.1", 0).unwrap();
    let cli = Cli::new();
    let url = format!("{}/get", srv.url());

    // fixed time at a fixed rate
    let o = cli.ok(&["bench", &url, "-t", "1s", "--rate", "20", "-c", "2", "--quiet"]);
    assert!((15..=21).contains(&requests(&o.stdout)), "{}", o.stdout);
    for needle in ["LATENCY (ms)", "P99.9", "first byte", "DISTRIBUTION (ms)", "status codes: 200 × ", "0.00% errors"] {
        assert!(o.stdout.contains(needle), "missing `{needle}` in {}", o.stdout);
    }
    // sub-second duration, warm-up, no keep-alive
    let o = cli.ok(&["bench", &url, "-t", "300ms", "--warmup", "3", "--no-keepalive", "--quiet"]);
    assert!(requests(&o.stdout) >= 3 && o.stdout.contains("3 warm-up request(s)"), "{}", o.stdout);
    // a long run reports progress instead of listing every request; a short one lists them
    let long = cli.ok(&["bench", &url, "-n", "200"]);
    assert!(!long.stderr.contains("#200"), "{}", long.stderr);
    let short = cli.ok(&["bench", &url, "-n", "3"]);
    assert!(short.stderr.contains("#3") && short.stderr.contains("3 iteration(s) · 1 worker(s)"), "{}", short.stderr);

    // invalid input is rejected before anything is sent
    for (args, message) in [
        (vec!["bench", url.as_str(), "-t", "soon"], "invalid duration"),
        (vec!["bench", url.as_str(), "-t", "5parsecs"], "invalid duration unit"),
        (vec!["bench", url.as_str(), "--rate", "0"], "--rate must be a positive number"),
        (vec!["bench", url.as_str(), "--assert", "p96<5"], "unknown threshold metric `p96`"),
        (vec!["bench", url.as_str(), "--assert", "p95 fast"], "needs one of <, <=, >, >="),
        (vec!["bench", url.as_str(), "--compare", "/nonexistent/base.json"], "cannot read baseline"),
        (vec!["bench", url.as_str(), "-n", "5", "-t", "1s"], "cannot be used with"),
        (vec!["bench", url.as_str(), "--rate", "5", "--delay", "10"], "cannot be used with"),
    ] {
        let o = cli.run(&args);
        assert!(o.code != 0 && o.stderr.contains(message), "{args:?}: {}", o.stderr);
        assert!(!o.stdout.contains("requests in"), "{args:?} must not run");
    }
}

#[test]
fn thresholds_decide_the_exit_code() {
    let srv = rspostman_core::echo::start("127.0.0.1", 0).unwrap();
    let cli = Cli::new();
    let ok = format!("{}/get", srv.url());
    let slow = format!("{}/delay/80", srv.url());
    let broken = format!("{}/status/500", srv.url());

    let o = cli.ok(&["bench", &ok, "-n", "5", "--quiet", "--assert", "p95<5s", "--assert", "error_rate<=0%", "--assert", "requests>=5"]);
    assert!(o.stdout.contains("THRESHOLDS") && o.stdout.matches('✓').count() == 3, "{}", o.stdout);

    let o = cli.run(&["bench", &slow, "-n", "3", "--quiet", "--assert", "p50<20ms", "--assert", "ttfb_p50<20ms", "--assert", "max<10s"]);
    assert_eq!(o.code, 1);
    assert!(o.stdout.contains("✗ p50<20ms") && o.stdout.contains("✗ ttfb_p50<20ms") && o.stdout.contains("✓ max<10s"), "{}", o.stdout);
    assert!(o.stdout.contains("measured 8") || o.stdout.contains("measured 9"), "{}", o.stdout);

    // without thresholds any HTTP error fails the run; with thresholds they decide
    assert_eq!(cli.run(&["bench", &broken, "-n", "2", "--quiet"]).code, 1);
    assert_eq!(cli.run(&["bench", &broken, "-n", "2", "--quiet", "--assert", "error_rate<=100%"]).code, 0);
    assert_eq!(cli.run(&["bench", &broken, "-n", "2", "--quiet", "--assert", "error_rate<50%"]).code, 1);
    // a latency threshold cannot pass when nothing answers
    let o = cli.run(&["bench", "http://127.0.0.1:1/", "-n", "1", "--quiet", "--assert", "p95<1s"]);
    assert!(o.code == 1 && o.stdout.contains("no request was answered"), "{}", o.stdout);
}

#[test]
fn reports_and_baseline_comparison() {
    let srv = rspostman_core::echo::start("127.0.0.1", 0).unwrap();
    let cli = Cli::new();
    let fast = format!("{}/get?access_token=BENCH-SECRET", srv.url());
    let slow = format!("{}/delay/120", srv.url());
    let p = |name: &str| cli.home.join(name).to_str().unwrap().to_string();

    cli.ok(&["bench", &fast, "-n", "8", "--quiet", "--report-json", &p("fast.json"), "--report-csv", &p("fast.csv"), "--report-html", &p("fast.html")]);
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p("fast.json")).unwrap()).unwrap();
    assert_eq!(json["total_requests"], 8);
    assert_eq!(json["status_counts"]["200"], 8);
    assert_eq!(json["ttfb"]["count"], 8);
    assert_eq!(json["histogram"].as_array().unwrap().iter().map(|b| b["count"].as_u64().unwrap()).sum::<u64>(), 8);
    assert!(json["timeline"].as_array().unwrap().iter().map(|t| t["requests"].as_u64().unwrap()).sum::<u64>() == 8);
    let csv = std::fs::read_to_string(p("fast.csv")).unwrap();
    assert!(csv.starts_with("iteration,name,started_ms,time_ms,ttfb_ms,status,size,error\n"));
    assert_eq!(csv.lines().count(), 9);
    let html = std::fs::read_to_string(p("fast.html")).unwrap();
    assert!(html.contains("Latency distribution") && html.contains("benchCharts(") && html.contains("<td>200</td>"));
    for file in ["fast.json", "fast.csv", "fast.html"] {
        let text = std::fs::read_to_string(p(file)).unwrap();
        assert!(!text.contains("BENCH-SECRET") && text.contains("access_token="), "{file}");
    }

    // against itself: no regression
    let o = cli.ok(&["bench", &fast, "-n", "8", "--quiet", "--compare", &p("fast.json"), "--tolerance", "100000"]);
    assert!(o.stdout.contains("COMPARED WITH THE BASELINE (tolerance 100000%)") && !o.stdout.contains("REGRESSION"), "{}", o.stdout);
    // a much slower endpoint against the fast baseline: regression, exit 1, shown in the HTML report
    let o = cli.run(&["bench", &slow, "-n", "4", "--quiet", "--compare", &p("fast.json"), "--report-json", &p("slow.json"), "--report-html", &p("slow.html")]);
    assert_eq!(o.code, 1, "{}", o.stdout);
    assert!(o.stdout.contains("REGRESSION") && o.stdout.contains("p50 (ms)"), "{}", o.stdout);
    assert!(std::fs::read_to_string(p("slow.html")).unwrap().contains("This run is worse than the baseline."));
    // getting faster than the baseline is fine
    assert_eq!(cli.run(&["bench", &fast, "-n", "8", "--quiet", "--compare", &p("slow.json"), "--tolerance", "100000"]).code, 0);
    // a file that is not a report
    std::fs::write(p("junk.json"), "[1, 2]").unwrap();
    assert!(cli.run(&["bench", &fast, "-n", "1", "--compare", &p("junk.json")]).stderr.contains("is not a benchmark JSON report"));
}
