//! Sprint 2 features through the real binary: runner, reporters, mock server, monitor,
//! WebSocket, importers and docs.

mod common;
use common::Cli;

const WORKFLOW: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/workflow.postman_collection.json");
const USERS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/users.csv");

#[test]
fn run_collection_with_scripts_data_and_reports() {
    let srv = rspostman_core::echo::start("127.0.0.1", 0).unwrap();
    let cli = Cli::new();
    cli.ok(&["import", WORKFLOW]);
    cli.ok(&["c", "set-var", "Workflow demo", "base", &srv.url()]);

    // a passing folder
    let o = cli.ok(&["run", "Workflow demo", "--folder", "Auth", "-v"]);
    assert!(o.stderr.contains("✓ login echoes the user"), "{}", o.stderr);
    assert!(o.stderr.contains("✓ inherited bearer token is sent"), "{}", o.stderr);
    assert!(o.stderr.contains("✓ collection: responds quickly"));
    assert!(o.stderr.contains("log: logged in as ann"), "{}", o.stderr);
    // the script stored the token as a collection variable
    assert!(cli.ok(&["c", "show", "Workflow demo"]).stdout.contains("token=ann-token"));

    // flow control: Loop runs three times, Skipped is skipped
    let o = cli.ok(&["run", "Workflow demo", "--folder", "Flow"]);
    for n in 1..=3 {
        assert!(o.stderr.contains(&format!("✓ loop pass {n}")), "{}", o.stderr);
    }
    assert!(o.stderr.contains("skipped by pm.execution.skipRequest()"));
    assert!(o.stdout.contains("requests    4"), "{}", o.stdout);

    // whole collection with data file: 2 iterations, failures -> exit code 1, reports written
    let (json, junit, html) = (cli.home.join("r.json"), cli.home.join("r.xml"), cli.home.join("r.html"));
    let o = cli.run(&[
        "run", "Workflow demo", "-d", USERS, "--no-persist",
        "--report-json", json.to_str().unwrap(), "--report-junit", junit.to_str().unwrap(), "--report-html", html.to_str().unwrap(),
    ]);
    assert_eq!(o.code, 1, "{}", o.stderr);
    assert!(o.stderr.contains("Iteration 2/2"));
    assert!(o.stdout.contains("Failures") && o.stdout.contains("Broken/Failing / status is 200 (iteration 2)"), "{}", o.stdout);
    assert!(o.stdout.contains("expected response to have status code 200 but got 500"));
    let report: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(report["stats"]["iterations"], 2);
    assert_eq!(report["stats"]["requests"], 14);
    assert_eq!(report["stats"]["requests_failed"], 2);
    assert_eq!(report["stats"]["tests_failed"], 2);
    assert_eq!(report["stats"]["requests_skipped"], 2);
    // iteration data overrides the collection variable `user`
    let second_login = report["executions"].as_array().unwrap().iter().find(|x| x["iteration"] == 1 && x["path"] == "Auth/Login").unwrap();
    assert!(second_login["exchange"]["request"]["body"]["text"].as_str().unwrap().contains("bob"));
    let xml = std::fs::read_to_string(&junit).unwrap();
    assert!(xml.contains("<testsuites name=\"Workflow demo\"") && xml.contains("<failure type=\"AssertionError\""));
    assert_eq!(xml.matches("<failure").count(), 2);
    let page = std::fs::read_to_string(&html).unwrap();
    assert!(page.contains("Run finished with failures") && page.contains("status is 200"));
    // --no-persist: the token from the bob iteration was not saved
    assert!(cli.ok(&["c", "show", "Workflow demo"]).stdout.contains("token=ann-token"));

    // --bail stops at the first failure, -n overrides iterations
    let o = cli.run(&["run", "Workflow demo", "--folder", "Broken", "--bail", "-n", "3", "-q"]);
    assert_eq!(o.code, 1);
    assert!(o.stdout.contains("run ended early: bailed after failure"), "{}", o.stdout);
    assert!(o.stdout.contains("requests    1"), "{}", o.stdout);

    // single request shows test results too and fails the exit code
    let o = cli.run(&["req", "Workflow demo", "Failing"]);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("✗ status is 200"));

    // running an exported file directly (newman style) with --var
    let o = cli.ok(&["run", WORKFLOW, "--folder", "Auth", "--var", &format!("base={}", srv.url()), "--var", "user=zed", "-v"]);
    assert!(o.stderr.contains("logged in as zed"), "{}", o.stderr);

    // monitor: two quick runs, log file written
    let o = cli.ok(&["monitor", "Workflow demo", "--folder", "Auth", "--every", "1s", "--count", "2"]);
    assert_eq!(o.stdout.matches("PASS").count(), 2, "{}", o.stdout);
    let log = std::fs::read_dir(cli.home.join("monitors")).unwrap().next().unwrap().unwrap().path();
    assert_eq!(std::fs::read_to_string(log).unwrap().lines().count(), 2);
    assert!(cli.run(&["monitor", "Workflow demo", "--every", "5 parsecs"]).code != 0);
}

fn wait_for_port(port: u16) {
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("server on port {port} did not start");
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[test]
fn mock_server_websocket_docs_and_openapi() {
    let srv = rspostman_core::echo::start("127.0.0.1", 0).unwrap();
    let cli = Cli::new();
    cli.ok(&["import", WORKFLOW]);

    // mock server from saved examples
    let port = free_port();
    let mut mock = std::process::Command::new(env!("CARGO_BIN_EXE_rspostman"))
        .args(["--workspace", cli.home.to_str().unwrap(), "mock", "Workflow demo", "--port", &port.to_string()])
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_for_port(port);
    let base = format!("http://127.0.0.1:{port}");
    let j = cli.json(&["send", "POST", &format!("{base}/post"), "--no-history"]);
    assert_eq!(j["token"], "example-token");
    assert_eq!(j["request"].as_str().unwrap().len(), 36, "dynamic variables are resolved in mock bodies");
    assert_eq!(cli.json(&["send", &format!("{base}/bearer")])["plan"], "pro");
    let o = cli.ok(&["send", &format!("{base}/bearer"), "-H", "x-mock-response-code: 401", "-i"]);
    assert!(o.stderr.contains("401") && o.stderr.contains("x-mock-example: unauthorized"), "{}", o.stderr);
    assert!(cli.ok(&["send", &format!("{base}/nope")]).stderr.contains("404"));
    mock.kill().ok();
    mock.wait().ok();
    assert!(cli.run(&["mock", "nope"]).code != 0);

    // websocket one-shot mode
    let ws_url = format!("ws://127.0.0.1:{}/ws", srv.port);
    let o = cli.ok(&["ws", &ws_url, "-m", "hello", "-m", "{\"n\": 1}", "--wait", "500"]);
    assert!(o.stdout.contains("hello") && o.stdout.contains("{\"n\": 1}") && o.stdout.contains("connected"), "{}", o.stdout);
    assert!(cli.run(&["ws", "ws://127.0.0.1:1/x", "-m", "x"]).code != 0);

    // documentation
    let md = cli.ok(&["docs", "Workflow demo"]).stdout;
    assert!(md.starts_with("# Workflow demo") && md.contains("### POST Login") && md.contains("**Example: logged in**"), "{md}");
    let html = cli.home.join("docs.html");
    cli.ok(&["docs", "Workflow demo", "-f", "html", "-o", html.to_str().unwrap()]);
    assert!(std::fs::read_to_string(&html).unwrap().contains("<title>Workflow demo – API documentation</title>"));
    assert!(cli.run(&["docs", "Workflow demo", "-f", "pdf"]).code != 0);

    // OpenAPI import -> runnable requests against the echo server
    let spec = cli.home.join("api.yaml");
    std::fs::write(
        &spec,
        format!(
            "openapi: 3.0.0\ninfo: {{ title: Echo API }}\nservers: [{{ url: '{}' }}]\npaths:\n  /anything/{{id}}:\n    get:\n      summary: Fetch thing\n      parameters:\n        - {{ name: id, in: path, required: true, schema: {{ type: integer, example: 5 }} }}\n        - {{ name: q, in: query, required: true, schema: {{ type: string, default: abc }} }}\n      responses:\n        '200': {{ description: ok, content: {{ application/json: {{ example: {{ mocked: true }} }} }} }}\n",
            srv.url()
        ),
    )
    .unwrap();
    assert!(cli.ok(&["import", spec.to_str().unwrap()]).stdout.contains("collection `Echo API` with 1 requests"));
    let j = cli.json(&["req", "Echo API", "Fetch thing"]);
    assert_eq!((j["path"].as_str(), j["args"]["q"].as_str()), (Some("/anything/5"), Some("abc")));
    // and its example can be mocked
    let port = free_port();
    let mut mock = std::process::Command::new(env!("CARGO_BIN_EXE_rspostman"))
        .args(["--workspace", cli.home.to_str().unwrap(), "mock", "Echo API", "--port", &port.to_string()])
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_for_port(port);
    assert_eq!(cli.json(&["send", &format!("http://127.0.0.1:{port}/anything/99")])["mocked"], true);
    mock.kill().ok();
    mock.wait().ok();
}
