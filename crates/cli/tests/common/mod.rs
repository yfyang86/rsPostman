//! Shared helper: runs the real `rspostman` binary in a throw-away workspace.
#![allow(dead_code, clippy::new_without_default)]

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
    /// Run with extra environment variables and optional stdin.
    pub fn run_with(&self, args: &[&str], env: &[(&str, &str)], stdin: Option<&str>) -> Out {
        use std::io::Write;
        use std::process::Stdio;
        let mut child = Command::new(env!("CARGO_BIN_EXE_rspostman"))
            .arg("--workspace")
            .arg(&self.home)
            .args(args)
            .env("NO_COLOR", "1")
            .env_remove("RSPOSTMAN_PASSWORD")
            .envs(env.iter().copied())
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run rspostman");
        if let Some(text) = stdin {
            child.stdin.take().expect("stdin").write_all(text.as_bytes()).expect("write stdin");
        }
        let o = child.wait_with_output().expect("wait");
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
