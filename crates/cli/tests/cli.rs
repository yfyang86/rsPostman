//! Drives the real `rspostman` binary against the in-process echo server.

#![allow(clippy::new_without_default)]

use std::path::PathBuf;
use std::process::Command;

pub struct Cli {
    pub home: PathBuf,
}

pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Cli {
    pub fn new() -> Self {
        let home = std::env::temp_dir().join(format!("rsp-cli-{}", rspostman_core::model::new_id()));
        let cli = Cli { home };
        cli.ok(&["settings", "set", "no_system_proxy", "true"]);
        cli
    }
    pub fn run(&self, args: &[&str]) -> Out {
        let o = Command::new(env!("CARGO_BIN_EXE_rspostman"))
            .arg("--workspace")
            .arg(&self.home)
            .args(args)
            .env("NO_COLOR", "1")
            .output()
            .expect("run rspostman");
        Out { code: o.status.code().unwrap_or(-1), stdout: String::from_utf8_lossy(&o.stdout).into(), stderr: String::from_utf8_lossy(&o.stderr).into() }
    }
    pub fn ok(&self, args: &[&str]) -> Out {
        let o = self.run(args);
        assert_eq!(o.code, 0, "command {args:?} failed\nstdout: {}\nstderr: {}", o.stdout, o.stderr);
        o
    }
    pub fn json(&self, args: &[&str]) -> serde_json::Value {
        let o = self.ok(args);
        serde_json::from_str(&o.stdout).unwrap_or_else(|e| panic!("{args:?} did not print JSON ({e}): {}", o.stdout))
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.home).ok();
    }
}

#[test]
fn send_adhoc_requests() {
    let srv = rspostman_core::echo::start("127.0.0.1", 0).unwrap();
    let cli = Cli::new();
    let base = srv.url();

    let j = cli.json(&["send", &format!("{base}/get"), "-q", "a=1", "-H", "X-T: yes"]);
    assert_eq!(j["args"]["a"], "1");
    assert_eq!(j["headers"]["x-t"], "yes");

    let o = cli.ok(&["send", &format!("{base}/post"), "--json", r#"{"n": 1}"#, "-i"]);
    assert!(o.stderr.contains("200 OK"), "{}", o.stderr);
    assert!(o.stderr.contains("content-type: application/json"), "{}", o.stderr);
    assert!(o.stdout.contains("\"method\": \"POST\""));

    let j = cli.json(&["send", "PUT", &format!("{base}/put"), "--form", "x=1 2"]);
    assert_eq!(j["form"]["x"], "1 2");

    let f = cli.home.join("up.txt");
    std::fs::write(&f, "DATA").unwrap();
    let j = cli.json(&["send", &format!("{base}/post"), "-F", "note=hi", "-F", &format!("file=@{}", f.display())]);
    assert_eq!(j["files"]["file"]["text"], "DATA");
    assert_eq!(j["form"]["note"], "hi");

    assert_eq!(cli.json(&["send", &format!("{base}/basic-auth/a/b"), "-u", "a:b"])["authenticated"], true);
    assert_eq!(cli.json(&["send", &format!("{base}/bearer"), "--bearer", "T1"])["token"], "T1");
    assert_eq!(cli.json(&["send", &format!("{base}/digest-auth/a/b"), "--auth", "digest", "--auth-param", "username=a", "--auth-param", "password=b"])["authenticated"], true);

    assert_eq!(cli.run(&["send", &format!("{base}/status/404"), "--fail"]).code, 22);
    assert_eq!(cli.run(&["send", &format!("{base}/status/404")]).code, 0);
    let bad = cli.run(&["send", "http://127.0.0.1:1/x"]);
    assert_eq!(bad.code, 1);
    assert!(bad.stderr.contains("error:"));

    let out = cli.home.join("body.bin");
    cli.ok(&["send", &format!("{base}/bytes/300"), "-o", out.to_str().unwrap()]);
    assert_eq!(std::fs::read(&out).unwrap().len(), 300);
    assert!(cli.ok(&["send", &format!("{base}/bytes/300")]).stdout.contains("binary body"));

    // history
    let h = cli.ok(&["history", "list"]);
    assert!(h.stdout.contains("/bytes/300") && h.stdout.contains("ERR"), "{}", h.stdout);
    assert!(cli.ok(&["history", "show", "1"]).stderr.contains("GET"));
    assert_eq!(cli.json(&["history", "replay", "8"])["authenticated"], true);
    cli.ok(&["history", "clear"]);
    assert!(cli.ok(&["history", "list"]).stdout.contains("empty"));
}

#[test]
fn collections_environments_and_variables() {
    let srv = rspostman_core::echo::start("127.0.0.1", 0).unwrap();
    let cli = Cli::new();

    cli.ok(&["collection", "create", "Shop", "-d", "demo"]);
    cli.ok(&["c", "set-var", "Shop", "base", &srv.url()]);
    cli.ok(&["c", "add-folder", "Shop", "Orders"]);
    cli.ok(&["c", "add-request", "Shop", "List", "{{base}}/get?stage={{stage}}", "--parent", "Orders"]);
    cli.ok(&["c", "add-request", "Shop", "Create", "POST", "{{base}}/post", "--json", r#"{"sku": "{{sku}}"}"#, "--parent", "Orders"]);
    cli.ok(&["c", "add-request", "Shop", "Me", "{{base}}/bearer"]);
    cli.ok(&["c", "set-auth", "Shop", "--type", "bearer", "--param", "token={{token}}"]);
    let show = cli.ok(&["c", "show", "Shop"]).stdout;
    assert!(show.contains("Orders") && show.contains("POST") && show.contains("auth: bearer"), "{show}");

    cli.ok(&["env", "create", "dev"]);
    cli.ok(&["env", "set", "dev", "stage", "development"]);
    cli.ok(&["env", "set", "dev", "token", "dev-token"]);
    cli.ok(&["env", "create", "prod"]);
    cli.ok(&["env", "set", "prod", "stage", "production"]);
    cli.ok(&["globals", "set", "sku", "G-1"]);
    cli.ok(&["globals", "set", "token", "global-token"]);

    // no environment: the unresolved placeholder is sent verbatim
    assert_eq!(cli.json(&["request", "Shop", "Orders/List"])["args"]["stage"], "{{stage}}");
    cli.ok(&["env", "use", "dev"]);
    assert!(cli.ok(&["env", "list"]).stdout.contains("*  dev"));
    assert_eq!(cli.json(&["request", "Shop", "List"])["args"]["stage"], "development");
    assert_eq!(cli.json(&["req", "Shop", "List", "-e", "prod"])["args"]["stage"], "production");
    assert_eq!(cli.json(&["req", "Shop", "List", "--var", "stage=local"])["args"]["stage"], "local");
    assert_eq!(cli.json(&["req", "Shop", "Create"])["json"]["sku"], "G-1");
    // inherited bearer auth; env beats globals
    assert_eq!(cli.json(&["req", "Shop", "Me"])["token"], "dev-token");
    assert_eq!(cli.json(&["req", "Shop", "Me", "-e", "prod"])["token"], "global-token");
    cli.ok(&["c", "set-auth", "Shop", "--item", "Me", "--type", "noauth"]);
    assert_eq!(cli.json(&["req", "Shop", "Me"])["authenticated"], false);

    // tree edits
    cli.ok(&["c", "move", "Shop", "Me", "--to", "Orders", "--index", "0"]);
    cli.ok(&["c", "rename-item", "Shop", "Me", "Whoami"]);
    assert!(cli.ok(&["c", "show", "Shop"]).stdout.contains("Whoami"));
    cli.ok(&["c", "remove", "Shop", "Orders/Whoami"]);
    assert!(cli.run(&["req", "Shop", "Whoami"]).code != 0);

    // export -> import round trip, plus codegen from a collection
    let file = cli.home.join("shop.json");
    cli.ok(&["export", "collection", "Shop", "-o", file.to_str().unwrap()]);
    let exported: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert!(exported["info"]["schema"].as_str().unwrap().contains("v2.1.0"));
    cli.ok(&["c", "rename", "Shop", "Shop v1"]);
    assert!(cli.ok(&["import", file.to_str().unwrap()]).stdout.contains("2 requests"));
    assert!(cli.ok(&["c", "list"]).stdout.contains("Shop v1"));
    // a collection file can be used directly, newman style
    assert_eq!(cli.json(&["req", file.to_str().unwrap(), "List"])["args"]["stage"], "development");

    let envfile = cli.home.join("dev.json");
    cli.ok(&["export", "env", "dev", "-o", envfile.to_str().unwrap()]);
    cli.ok(&["env", "delete", "dev"]);
    assert!(cli.ok(&["import", envfile.to_str().unwrap()]).stdout.contains("2 variables"));

    let code = cli.ok(&["codegen", "python", "-c", "Shop", "-r", "Create", "-e", "dev"]).stdout;
    assert!(code.contains("import requests") && code.contains("G-1") && code.contains("Bearer dev-token"), "{code}");
    assert!(cli.ok(&["codegen", "--list"]).stdout.contains("rust"));
    let curl = cli.ok(&["codegen", "curl", "POST", "http://x.test/a", "--json", "{}"]).stdout;
    assert!(curl.contains("--data-raw '{}'"));

    cli.ok(&["c", "delete", "Shop v1"]);
    assert!(cli.run(&["c", "show", "nope"]).stderr.contains("not found"));
}

#[test]
fn curl_import_cookies_and_settings() {
    let srv = rspostman_core::echo::start("127.0.0.1", 0).unwrap();
    let cli = Cli::new();
    let base = srv.url();

    let cmd = format!("curl -X POST '{base}/post' -H 'X-From: curl' -d 'a=1&b=two'");
    cli.ok(&["import", &cmd, "--collection", "Curl", "--name", "posted"]);
    let j = cli.json(&["req", "Curl", "posted"]);
    assert_eq!(j["form"], serde_json::json!({"a": "1", "b": "two"}));
    assert_eq!(j["headers"]["x-from"], "curl");

    // cookies persist between invocations
    cli.ok(&["send", &format!("{base}/cookies/set?sid=42")]);
    assert_eq!(cli.json(&["send", &format!("{base}/cookies")])["cookies"]["sid"], "42");
    assert!(cli.ok(&["cookies", "list"]).stdout.contains("sid"));
    cli.ok(&["cookies", "set", &base, "extra=1; Path=/"]);
    assert_eq!(cli.json(&["send", &format!("{base}/cookies")])["cookies"]["extra"], "1");
    cli.ok(&["cookies", "clear", "127.0.0.1", "sid"]);
    assert!(cli.json(&["send", &format!("{base}/cookies")])["cookies"].get("sid").is_none());
    cli.ok(&["cookies", "clear"]);
    assert!(cli.ok(&["cookies", "list"]).stdout.contains("empty"));

    // settings
    cli.ok(&["settings", "set", "follow_redirects", "false"]);
    assert!(cli.ok(&["send", &format!("{base}/redirect/1")]).stderr.contains("302"));
    assert!(cli.ok(&["send", &format!("{base}/redirect/1"), "--no-history"]).stderr.contains("302"));
    cli.ok(&["settings", "reset"]);
    cli.ok(&["settings", "set", "no_system_proxy", "true"]);
    assert!(cli.ok(&["send", &format!("{base}/redirect/1")]).stderr.contains("200"));
    assert!(cli.run(&["settings", "set", "bogus", "1"]).code != 0);
    assert!(cli.run(&["settings", "set", "timeout_ms", "abc"]).code != 0);
    assert!(cli.run(&["send", &format!("{base}/delay/800"), "--timeout", "100"]).code == 1);
}
