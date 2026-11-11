//! Terminal output helpers.

use rspostman_core::http::{human_size, human_time, Prepared, Response};
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

static COLOR: AtomicBool = AtomicBool::new(false);

pub fn init(no_color: bool) {
    let on = !no_color && std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal() && std::io::stdout().is_terminal();
    COLOR.store(on, Ordering::Relaxed);
}

pub fn paint(code: &str, s: &str) -> String {
    if COLOR.load(Ordering::Relaxed) {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}
pub fn bold(s: &str) -> String {
    paint("1", s)
}
pub fn dim(s: &str) -> String {
    paint("2", s)
}
pub fn green(s: &str) -> String {
    paint("32", s)
}
pub fn red(s: &str) -> String {
    paint("31", s)
}
pub fn yellow(s: &str) -> String {
    paint("33", s)
}
pub fn cyan(s: &str) -> String {
    paint("36", s)
}

pub fn status(code: u16, text: &str) -> String {
    let s = format!("{code} {text}");
    match code {
        200..=299 => green(&s),
        300..=399 => cyan(&s),
        400..=499 => yellow(&s),
        _ => red(&s),
    }
}

pub fn method(m: &str) -> String {
    let padded = format!("{m:<7}");
    match m {
        "GET" => green(&padded),
        "POST" => yellow(&padded),
        "PUT" => cyan(&padded),
        "DELETE" => red(&padded),
        _ => paint("35", &padded),
    }
}

/// Print rows as an aligned table.
pub fn table(headers: &[&str], rows: &[Vec<String>]) {
    let mut w: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            if i < w.len() {
                w[i] = w[i].max(c.chars().count());
            }
        }
    }
    let line = |cells: Vec<String>| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c}{}", " ".repeat(w.get(i).copied().unwrap_or(0).saturating_sub(c.chars().count()))))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    println!("{}", bold(&line(headers.iter().map(|s| s.to_string()).collect())));
    for r in rows {
        println!("{}", line(r.clone()));
    }
}

pub fn request(p: &Prepared) {
    eprintln!("{} {}", bold(&p.method), p.url);
    for (k, v) in &p.headers {
        eprintln!("{} {v}", dim(&format!("> {k}:")));
    }
    let b = p.body.preview();
    if !b.is_empty() {
        eprintln!("{}", dim(">"));
        for l in b.lines().take(40) {
            eprintln!("{} {l}", dim(">"));
        }
    }
    eprintln!();
}

pub fn response_meta(r: &Response, headers: bool) {
    eprintln!(
        "{} {}  {}  {}",
        dim(&r.http_version),
        bold(&status(r.status, &r.status_text)),
        dim(&human_time(r.time_ms)),
        dim(&human_size(r.size))
    );
    if headers {
        for (k, v) in &r.headers {
            eprintln!("{} {v}", cyan(&format!("{k}:")));
        }
        eprintln!();
    }
}

/// Body as shown to the user: pretty JSON when possible, a note for binary data.
pub fn body_text(r: &Response, raw: bool) -> String {
    if !r.is_text() && std::str::from_utf8(&r.body).is_err() {
        return format!("<binary body, {} — use -o FILE to save it>", human_size(r.size));
    }
    if !raw {
        if let Some(j) = r.json() {
            if j.is_object() || j.is_array() {
                return serde_json::to_string_pretty(&j).unwrap_or_else(|_| r.text());
            }
        }
    }
    r.text()
}
