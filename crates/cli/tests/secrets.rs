//! Credentials must not leak into the history, reports or terminal output, and must not have
//! to be typed on the command line. Also covers the `benchmark` and `profile` commands.

mod common;
use common::Cli;

fn history(cli: &Cli) -> String {
    std::fs::read_to_string(cli.home.join("history.jsonl")).unwrap_or_default()
}

#[test]
fn history_and_output_do_not_contain_credentials() {
    let srv = rspostman_core::echo::start("127.0.0.1", 0).unwrap();
    let cli = Cli::new();
    let base = srv.url();

    // basic auth, a bearer header, an API key in the query and a password in a JSON body
    let o = cli.ok(&["send", &format!("{base}/basic-auth/bob/hunter2"), "-u", "bob:hunter2", "-v"]);
    assert!(o.stderr.contains("> Authorization: Basic <redacted>"), "{}", o.stderr);
    assert!(!o.stderr.contains("Ym9iOmh1bnRlcjI"), "base64 credentials leaked into verbose output");
    let o = cli.ok(&["send", &format!("{base}/bearer"), "--bearer", "TOPSECRET-1", "-v", "--show-secrets"]);
    assert!(o.stderr.contains("Bearer TOPSECRET-1"));
    cli.ok(&["send", &format!("{base}/get?api_key=KEY-IN-QUERY&page=2"), "-H", "X-Api-Key: KEY-IN-HEADER"]);
    cli.ok(&["send", &format!("{base}/post"), "--json", r#"{"user": "bob", "password": "PW-IN-BODY", "max_tokens": 5}"#]);
    cli.ok(&["send", &format!("{base}/cookies/set?session_token=COOKIE-VALUE")]);

    let h = history(&cli);
    // (`hunter2` is also part of the echo URL path, so look for it as a stored auth value)
    for leaked in ["\"value\":\"hunter2\"", "Ym9iOmh1bnRlcjI", "TOPSECRET-1", "KEY-IN-QUERY", "KEY-IN-HEADER", "PW-IN-BODY", "COOKIE-VALUE"] {
        assert!(!h.contains(leaked), "`{leaked}` was written to history.jsonl");
    }
    assert!(h.contains("<redacted>") && h.contains("page=2") && h.contains("bob") && h.contains("max_tokens"));
    // `history show` prints what is stored
    let shown = cli.ok(&["history", "show", "5"]);
    assert!(shown.stderr.contains("Basic <redacted>"), "{}", shown.stderr);

    // opting in keeps credentials, and such entries can be replayed
    cli.ok(&["settings", "set", "history_store_secrets", "true"]);
    cli.ok(&["send", &format!("{base}/basic-auth/bob/hunter2"), "-u", "bob:hunter2"]);
    assert!(history(&cli).contains("\"value\":\"hunter2\""));
    assert_eq!(cli.json(&["history", "replay", "1"])["authenticated"], true);
    cli.ok(&["settings", "set", "history_store_secrets", "false"]);

    // a saved request is replayed from the collection, which still has its credentials
    cli.ok(&["collection", "create", "Vault"]);
    cli.ok(&["c", "add-request", "Vault", "Login", &format!("{base}/basic-auth/ann/pw9"), "-u", "ann:pw9"]);
    cli.ok(&["request", "Vault", "Login"]);
    let replay = cli.ok(&["history", "replay", "1"]);
    assert!(replay.stdout.contains("\"authenticated\": true"), "{}", replay.stdout);
    assert!(replay.stderr.contains("replaying the saved request"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for f in ["history.jsonl", "cookies.json", "settings.json"] {
            let mode = std::fs::metadata(cli.home.join(f)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{f} must be readable by the owner only");
        }
    }
}

#[test]
fn credentials_from_environment_file_and_stdin() {
    let srv = rspostman_core::echo::start("127.0.0.1", 0).unwrap();
    let cli = Cli::new();
    let base = srv.url();

    let o = cli.run_with(&["send", &format!("{base}/bearer"), "--bearer", "env:MY_TOKEN"], &[("MY_TOKEN", "from-env")], None);
    assert!(o.stdout.contains("\"token\": \"from-env\""), "{} {}", o.stdout, o.stderr);
    let missing = cli.run(&["send", &format!("{base}/bearer"), "--bearer", "env:NOT_SET_ANYWHERE"]);
    assert_eq!(missing.code, 1);
    assert!(missing.stderr.contains("environment variable `NOT_SET_ANYWHERE`"));

    let file = cli.home.join("token.txt");
    std::fs::write(&file, "from-file\n").unwrap();
    let o = cli.ok(&["send", &format!("{base}/bearer"), "--bearer", &format!("@{}", file.display())]);
    assert!(o.stdout.contains("\"token\": \"from-file\""));
    let o = cli.run_with(&["send", &format!("{base}/headers"), "--api-key", "X-Api-Key=env:K"], &[("K", "k-env")], None);
    assert!(o.stdout.contains("\"x-api-key\": \"k-env\""));

    // -u USER: password from RSPOSTMAN_PASSWORD, from USER:env:NAME, or from stdin when piped
    let url = format!("{base}/basic-auth/bob/s3cret");
    for (args, env, stdin) in [
        (vec!["send", url.as_str(), "-u", "bob"], vec![("RSPOSTMAN_PASSWORD", "s3cret")], None),
        (vec!["send", url.as_str(), "-u", "bob:env:PW"], vec![("PW", "s3cret")], None),
        (vec!["send", url.as_str(), "-u", "bob"], vec![], Some("s3cret\n")),
        (vec!["send", url.as_str(), "--auth", "digest", "--auth-param", "username=bob", "--auth-param", "password=env:PW"], vec![("PW", "x")], None),
    ] {
        let o = cli.run_with(&args, &env, stdin);
        let expect_ok = !args.contains(&"digest");
        assert_eq!(o.stdout.contains("\"authenticated\": true"), expect_ok, "{args:?}: {} {}", o.stdout, o.stderr);
    }
    let none = cli.run_with(&["send", &url, "-u", "bob"], &[], None);
    assert_eq!(none.code, 1);
    assert!(none.stderr.contains("no password for bob given"), "{}", none.stderr);

    // secret variables: value from stdin, masked in listings
    cli.ok(&["env", "create", "prod"]);
    let o = cli.run_with(&["env", "set", "prod", "api_token", "--secret"], &[], Some("tok-123\n"));
    assert_eq!(o.code, 0, "{}", o.stderr);
    cli.ok(&["env", "set", "prod", "host", &base]);
    let shown = cli.ok(&["env", "show", "prod"]).stdout;
    assert!(shown.contains("********") && !shown.contains("tok-123") && shown.contains(&base), "{shown}");
    assert!(cli.ok(&["env", "show", "prod", "--show-secrets"]).stdout.contains("tok-123"));
    cli.ok(&["globals", "set", "g_secret", "gs", "--secret"]);
    assert!(!cli.ok(&["globals", "list"]).stdout.contains("gs\n"));
    let stored: serde_json::Value = serde_json::from_str(&cli.ok(&["export", "env", "prod"]).stdout).unwrap();
    assert_eq!(stored["values"][0]["type"], "secret");
    // the secret variable still resolves when sending
    let o = cli.ok(&["send", "{{host}}/bearer", "--bearer", "{{api_token}}", "-e", "prod"]);
    assert!(o.stdout.contains("\"token\": \"tok-123\""));
    // a {{reference}} is not a secret: it stays in the history and can be replayed
    assert!(history(&cli).contains("{{api_token}}"));
    assert!(cli.ok(&["history", "replay", "1", "-e", "prod"]).stdout.contains("tok-123"));

    cli.ok(&["collection", "create", "C"]);
    cli.ok(&["c", "set-var", "C", "client_secret", "cs-1", "--secret"]);
    let show = cli.ok(&["c", "show", "C"]).stdout;
    assert!(show.contains("client_secret=********") && !show.contains("cs-1"), "{show}");
}

#[test]
fn run_reports_are_redacted() {
    let srv = rspostman_core::echo::start("127.0.0.1", 0).unwrap();
    let cli = Cli::new();
    cli.ok(&["collection", "create", "R"]);
    cli.ok(&["c", "add-request", "R", "Me", &format!("{}/bearer?access_token=QTOKEN", srv.url()), "--bearer", "RTOKEN"]);
    let (json, html) = (cli.home.join("r.json"), cli.home.join("r.html"));
    cli.ok(&["run", "R", "--report-json", json.to_str().unwrap(), "--report-html", html.to_str().unwrap()]);
    for file in [&json, &html] {
        let text = std::fs::read_to_string(file).unwrap();
        assert!(!text.contains("RTOKEN") && !text.contains("QTOKEN"), "credentials leaked into {}: {}", file.display(), text.lines().filter(|l| l.contains("TOKEN")).collect::<Vec<_>>().join(" | "));
        assert!(text.contains("/bearer"));
    }
}

#[test]
fn benchmark_and_profile_commands() {
    let srv = rspostman_core::echo::start("127.0.0.1", 0).unwrap();
    let cli = Cli::new();
    let base = srv.url();

    let report = cli.home.join("bench.json");
    let o = cli.ok(&["benchmark", &format!("{base}/get?api_key=BENCHKEY"), "-n", "12", "-c", "3", "--report-json", report.to_str().unwrap()]);
    for needle in ["P10", "P50", "P99", "12 requests in", "0 failed to connect", "0 answered with HTTP 4xx/5xx"] {
        assert!(o.stdout.contains(needle), "missing `{needle}` in {}", o.stdout);
    }
    assert!(!o.stderr.contains("BENCHKEY") && o.stderr.contains("api_key=<redacted>"), "{}", o.stderr);
    let r: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert_eq!((r["total_requests"].as_u64(), r["overall"]["count"].as_u64()), (Some(12), Some(12)));
    assert!(!r.to_string().contains("BENCHKEY"));

    // HTTP errors and unreachable hosts fail the exit code and are reported separately
    let o = cli.run(&["bench", &format!("{base}/status/503"), "-n", "3", "--quiet"]);
    assert_eq!(o.code, 1);
    assert!(o.stdout.contains("3 answered with HTTP 4xx/5xx"), "{}", o.stdout);
    let o = cli.run(&["bench", "http://127.0.0.1:1/", "-n", "2", "--quiet"]);
    assert_eq!(o.code, 1);
    assert!(o.stdout.contains("no request was answered") && o.stdout.contains("2 failed to connect"), "{}", o.stdout);
    assert!(cli.run(&["benchmark"]).code != 0);

    // collection mode with per-request rows
    cli.ok(&["collection", "create", "B"]);
    cli.ok(&["c", "add-request", "B", "one", &format!("{base}/get")]);
    cli.ok(&["c", "add-request", "B", "two", &format!("{base}/json")]);
    let o = cli.ok(&["benchmark", "--collection", "B", "-n", "2", "--quiet"]);
    assert!(o.stdout.contains("REQUEST") && o.stdout.contains("one") && o.stdout.contains("two") && o.stdout.contains("4 requests in"), "{}", o.stdout);
    assert!(cli.ok(&["benchmark", "--collection", "B", "--request", "two", "-n", "1", "--quiet"]).stdout.contains("1 requests in"));

    // LLM profile
    let o = cli.run_with(&["profile", &format!("{base}/llm"), "--model", "m-test", "--bearer", "env:LLM_KEY", "-n", "2", "-v"], &[("LLM_KEY", "sk-test")], None);
    assert_eq!(o.code, 0, "{} {}", o.stdout, o.stderr);
    assert!(o.stdout.contains("healthy (2/2)") && o.stdout.contains("13 completion tokens") && o.stdout.contains("tok/s") && o.stdout.contains("m-test"), "{}", o.stdout);
    assert!(o.stdout.contains("streamed test reply from the echo LLM"));
    let degraded = cli.run(&["profile", &format!("{base}/llm/fail"), "-n", "1"]);
    assert_eq!(degraded.code, 1);
    assert!(degraded.stdout.contains("degraded") && degraded.stdout.contains("empty stream"), "{}", degraded.stdout);
    let bad = cli.run(&["profile", &format!("{base}/status/401"), "-n", "1"]);
    assert_eq!(bad.code, 1);
    assert!(bad.stdout.contains("unhealthy") && bad.stdout.contains("HTTP 401"));
    // an explicit body is kept (and asked to stream)
    let o = cli.ok(&["profile", &format!("{base}/llm"), "--json", r#"{"model": "explicit-model"}"#, "-n", "1"]);
    assert!(o.stdout.contains("explicit-model"), "{}", o.stdout);
}
