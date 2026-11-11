//! The local vault through the real binary: creation, use in requests, and the guarantee that
//! vault secrets do not reach the history, reports or terminal output.

mod common;
use common::Cli;

const PASS: (&str, &str) = ("RSPOSTMAN_VAULT_PASSPHRASE", "correct horse battery");

fn all_files(dir: &std::path::Path) -> String {
    let mut out = String::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.push_str(&all_files(&p));
        } else {
            out.push_str(&String::from_utf8_lossy(&std::fs::read(&p).unwrap()));
        }
    }
    out
}

#[test]
fn vault_lifecycle() {
    let cli = Cli::new();
    assert!(cli.ok(&["vault", "status"]).stdout.contains("no vault"));
    assert!(cli.ok(&["vault", "list"]).stdout.contains("No vault"));
    // without a terminal the passphrase must come from the environment
    let o = cli.run(&["vault", "init"]);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("RSPOSTMAN_VAULT_PASSPHRASE"), "{}", o.stderr);
    let short = cli.run_with(&["vault", "init"], &[("RSPOSTMAN_VAULT_PASSPHRASE", "short")], None);
    assert!(short.stderr.contains("at least 8 characters"), "{}", short.stderr);

    let o = cli.run_with(&["vault", "init"], &[PASS], None);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(cli.run_with(&["vault", "init"], &[PASS], None).stderr.contains("already exists"));

    // value as argument, and piped on stdin (the passphrase is never taken from stdin)
    assert!(cli.run_with(&["vault", "set", "api_token", "VAULT-TOK-1"], &[PASS], None).stdout.contains("{{vault:api_token}}"));
    assert_eq!(cli.run_with(&["vault", "set", "db.password"], &[PASS], Some("VAULT-PW-2\n")).code, 0);
    assert!(cli.run_with(&["vault", "set", "bad name", "x"], &[PASS], None).stderr.contains("invalid secret name"));
    assert_eq!(cli.ok(&["vault", "list"]).stdout, "api_token\ndb.password\n", "listing needs no passphrase");
    assert!(cli.ok(&["vault", "status"]).stdout.contains("2 secret(s)"));
    assert_eq!(cli.run_with(&["vault", "get", "db.password"], &[PASS], None).stdout, "VAULT-PW-2\n");
    assert!(cli.run_with(&["vault", "get", "nope"], &[PASS], None).stderr.contains("no secret named"));
    let wrong = cli.run_with(&["vault", "get", "api_token"], &[("RSPOSTMAN_VAULT_PASSPHRASE", "not the passphrase")], None);
    assert_eq!(wrong.code, 1);
    assert!(wrong.stderr.contains("wrong vault passphrase") && !wrong.stdout.contains("VAULT-TOK-1"));

    // nothing in the workspace is readable
    let disk = all_files(&cli.home);
    assert!(!disk.contains("VAULT-TOK-1") && !disk.contains("VAULT-PW-2") && !disk.contains("correct horse"));

    // change the passphrase
    let o = cli.run_with(&["vault", "passwd"], &[PASS, ("RSPOSTMAN_VAULT_NEW_PASSPHRASE", "a new long passphrase")], None);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(cli.run_with(&["vault", "get", "api_token"], &[PASS], None).code != 0);
    let new = ("RSPOSTMAN_VAULT_PASSPHRASE", "a new long passphrase");
    assert_eq!(cli.run_with(&["vault", "get", "api_token"], &[new], None).stdout, "VAULT-TOK-1\n");
    assert_eq!(cli.run_with(&["vault", "rm", "db.password"], &[new], None).code, 0);
    assert_eq!(cli.ok(&["vault", "ls"]).stdout, "api_token\n");

    // `vault set` on a fresh workspace creates the vault
    let fresh = Cli::new();
    let o = fresh.run_with(&["vault", "set", "first", "v1"], &[PASS], None);
    assert!(o.code == 0 && o.stderr.contains("creating one"), "{}", o.stderr);
    assert_eq!(fresh.ok(&["vault", "list"]).stdout, "first\n");
}

#[test]
fn vault_secrets_in_requests_and_nowhere_else() {
    let srv = rspostman_core::echo::start("127.0.0.1", 0).unwrap();
    let cli = Cli::new();
    let base = srv.url();
    cli.run_with(&["vault", "set", "token", "VAULT-TOKEN-XYZ"], &[PASS], None);
    cli.run_with(&["vault", "set", "pw", "VAULT-PASSWORD-9"], &[PASS], None);

    // locked (no passphrase available): the request is not sent
    let o = cli.run(&["send", &format!("{base}/bearer"), "--bearer", "{{vault:token}}"]);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("vault passphrase is needed"), "{}", o.stderr);
    // unknown secret: clear error instead of sending the placeholder
    let o = cli.run_with(&["send", &format!("{base}/bearer"), "--bearer", "{{vault:missing}}"], &[PASS], None);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("vault secret `missing` is not available"), "{}", o.stderr);

    // bearer, basic auth, a header with an unusual name, the URL path and a free-text body
    let o = cli.run_with(&["send", &format!("{base}/bearer"), "--bearer", "{{vault:token}}", "-v", "--show-secrets"], &[PASS], None);
    assert!(o.stdout.contains("\"authenticated\": true"), "{} {}", o.stdout, o.stderr);
    assert!(!o.stderr.contains("VAULT-TOKEN-XYZ"), "vault secrets are not printed even with --show-secrets: {}", o.stderr);
    let o = cli.run_with(&["send", &format!("{base}/basic-auth/bob/{{{{vault:pw}}}}"), "-u", "bob:{{vault:pw}}"], &[PASS], None);
    assert!(o.stdout.contains("\"authenticated\": true"), "{} {}", o.stdout, o.stderr);
    let o = cli.run_with(
        &["send", "POST", &format!("{base}/anything/{{{{vault:token}}}}/x"), "-H", "X-Odd: pre-{{vault:token}}", "-d", "free text {{vault:pw}} here"],
        &[PASS],
        None,
    );
    assert!(o.stdout.contains("\"x-odd\": \"pre-VAULT-TOKEN-XYZ\"") && o.stdout.contains("free text VAULT-PASSWORD-9 here"), "{}", o.stdout);

    // through an environment variable, in a saved collection, with a run report
    cli.ok(&["env", "create", "prod"]);
    cli.ok(&["env", "set", "prod", "api_token", "{{vault:token}}"]);
    cli.ok(&["env", "use", "prod"]);
    cli.ok(&["collection", "create", "V"]);
    cli.ok(&["c", "add-request", "V", "Me", &format!("{base}/bearer"), "--bearer", "{{api_token}}"]);
    assert!(cli.run_with(&["request", "V", "Me"], &[PASS], None).stdout.contains("\"token\": \"VAULT-TOKEN-XYZ\""));
    let report = cli.home.join("run.json");
    let o = cli.run_with(&["run", "V", "--report-json", report.to_str().unwrap(), "-v"], &[PASS], None);
    assert_eq!(o.code, 0, "{} {}", o.stdout, o.stderr);
    // replaying from the history works: only the reference was stored
    assert!(cli.run_with(&["history", "replay", "1"], &[PASS], None).stdout.contains("VAULT-TOKEN-XYZ"));
    assert!(cli.run_with(&["codegen", "curl", "-c", "V", "-r", "Me"], &[PASS], None).stdout.contains("Bearer VAULT-TOKEN-XYZ"), "snippets carry real credentials");

    // the whole workspace – history, report, collection, environment, vault – has no secret in clear
    let disk = all_files(&cli.home);
    for secret in ["VAULT-TOKEN-XYZ", "VAULT-PASSWORD-9", "correct horse"] {
        assert!(!disk.contains(secret), "`{secret}` found in the workspace files");
    }
    assert!(disk.contains("{{vault:token}}") && disk.contains("<redacted>"));
}
