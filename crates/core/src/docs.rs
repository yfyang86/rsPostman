//! API documentation generated from a collection (Markdown or standalone HTML).

use crate::model::{Auth, Collection, Item, KV};
use crate::report::xml_escape as esc;

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn md_table(out: &mut String, title: &str, rows: &[KV]) {
    let rows: Vec<&KV> = rows.iter().filter(|r| !r.key.is_empty()).collect();
    if rows.is_empty() {
        return;
    }
    out.push_str(&format!("**{title}**\n\n| Key | Value | Description |\n|---|---|---|\n"));
    for r in rows {
        let key = if r.disabled { format!("~~{}~~", cell(&r.key)) } else { cell(&r.key) };
        let value = if r.is_file() { format!("*file* {}", r.file_paths().join(", ")) } else { cell(&r.value) };
        out.push_str(&format!("| {key} | {value} | {} |\n", cell(r.description.as_deref().unwrap_or(""))));
    }
    out.push('\n');
}

fn auth_line(a: &Auth) -> String {
    match a.kind.as_str() {
        "noauth" => "No auth".into(),
        "basic" => "Basic auth".into(),
        "bearer" => "Bearer token".into(),
        "apikey" => format!("API key (`{}` in {})", a.get("key"), if a.get("in") == "query" { "query" } else { "header" }),
        "digest" => "Digest auth".into(),
        "oauth1" => "OAuth 1.0".into(),
        "oauth2" => "OAuth 2.0".into(),
        "awsv4" => "AWS Signature v4".into(),
        "jwt" => "JWT bearer".into(),
        other => other.to_string(),
    }
}

fn fence(lang: &str, text: &str) -> String {
    format!("```{lang}\n{}\n```\n\n", text.trim_end())
}

fn md_items(out: &mut String, items: &[Item], depth: usize) {
    let h = "#".repeat((depth + 2).min(6));
    for it in items {
        if let Some(children) = &it.item {
            out.push_str(&format!("{h} {}\n\n", it.name));
            if let Some(d) = &it.description {
                out.push_str(&format!("{d}\n\n"));
            }
            if let Some(a) = &it.auth {
                out.push_str(&format!("Authorization: {}\n\n", auth_line(a)));
            }
            md_items(out, children, depth + 1);
            continue;
        }
        let Some(r) = &it.request else { continue };
        out.push_str(&format!("{h} {} {}\n\n", r.method, it.name));
        out.push_str(&fence("", &format!("{} {}", r.method, r.url.raw)));
        if let Some(d) = &r.description {
            out.push_str(&format!("{d}\n\n"));
        }
        if let Some(a) = &r.auth {
            out.push_str(&format!("Authorization: {}\n\n", auth_line(a)));
        }
        md_table(out, "Path variables", &r.url.variable);
        md_table(out, "Query parameters", &r.url.query);
        md_table(out, "Headers", &r.header);
        if let Some(b) = r.body.as_ref().filter(|b| !b.disabled) {
            match b.mode.as_str() {
                "raw" => {
                    if let Some(t) = b.raw.as_ref().filter(|t| !t.trim().is_empty()) {
                        out.push_str("**Body**\n\n");
                        out.push_str(&fence(b.language(), t));
                    }
                }
                "urlencoded" => md_table(out, "Body (x-www-form-urlencoded)", &b.urlencoded),
                "formdata" => md_table(out, "Body (form-data)", &b.formdata),
                "graphql" => {
                    if let Some(g) = &b.graphql {
                        out.push_str("**GraphQL query**\n\n");
                        out.push_str(&fence("graphql", &g.query));
                        if !g.variables.trim().is_empty() {
                            out.push_str("**GraphQL variables**\n\n");
                            out.push_str(&fence("json", &g.variables));
                        }
                    }
                }
                "file" => out.push_str(&format!("**Body**: binary file `{}`\n\n", b.file_path().unwrap_or_default())),
                _ => {}
            }
        }
        for ex in &it.response {
            out.push_str(&format!("**Example: {}** — `{} {}`\n\n", ex.name, ex.code, ex.status));
            if !ex.body.trim().is_empty() {
                out.push_str(&fence(ex.preview_language.as_deref().unwrap_or(""), &ex.body));
            }
        }
    }
}

pub fn markdown(c: &Collection) -> String {
    let mut out = format!("# {}\n\n", c.info.name);
    if let Some(d) = &c.info.description {
        out.push_str(&format!("{d}\n\n"));
    }
    if let Some(a) = &c.auth {
        out.push_str(&format!("Authorization: {}\n\n", auth_line(a)));
    }
    md_table(&mut out, "Variables", &c.variable);
    md_items(&mut out, &c.item, 0);
    out
}

fn html_table(out: &mut String, title: &str, rows: &[KV]) {
    let rows: Vec<&KV> = rows.iter().filter(|r| !r.key.is_empty()).collect();
    if rows.is_empty() {
        return;
    }
    out.push_str(&format!("<h4>{}</h4><table><tr><th>Key</th><th>Value</th><th>Description</th></tr>", esc(title)));
    for r in rows {
        let value = if r.is_file() { format!("file {}", r.file_paths().join(", ")) } else { r.value.clone() };
        out.push_str(&format!(
            "<tr{}><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
            if r.disabled { " class=\"off\"" } else { "" },
            esc(&r.key),
            esc(&value),
            esc(r.description.as_deref().unwrap_or(""))
        ));
    }
    out.push_str("</table>");
}

fn html_items(out: &mut String, nav: &mut String, items: &[Item], depth: usize) {
    for it in items {
        let anchor = format!("i-{}", esc(&it.id));
        if let Some(children) = &it.item {
            nav.push_str(&format!("<a class=\"f d{depth}\" href=\"#{anchor}\">{}</a>", esc(&it.name)));
            out.push_str(&format!("<section id=\"{anchor}\"><h2>{}</h2>", esc(&it.name)));
            if let Some(d) = &it.description {
                out.push_str(&format!("<p>{}</p>", esc(d).replace('\n', "<br>")));
            }
            html_items(out, nav, children, depth + 1);
            out.push_str("</section>");
            continue;
        }
        let Some(r) = &it.request else { continue };
        nav.push_str(&format!("<a class=\"d{depth}\" href=\"#{anchor}\"><b class=\"m {m}\">{m}</b>{}</a>", esc(&it.name), m = esc(&r.method)));
        out.push_str(&format!(
            "<article id=\"{anchor}\"><h3><b class=\"m {m}\">{m}</b> {}</h3><pre class=\"url\">{}</pre>",
            esc(&it.name),
            esc(&r.url.raw),
            m = esc(&r.method)
        ));
        if let Some(d) = &r.description {
            out.push_str(&format!("<p>{}</p>", esc(d).replace('\n', "<br>")));
        }
        if let Some(a) = &r.auth {
            out.push_str(&format!("<p><b>Authorization:</b> {}</p>", esc(&auth_line(a))));
        }
        html_table(out, "Path variables", &r.url.variable);
        html_table(out, "Query parameters", &r.url.query);
        html_table(out, "Headers", &r.header);
        if let Some(b) = r.body.as_ref().filter(|b| !b.disabled) {
            match b.mode.as_str() {
                "raw" => {
                    if let Some(t) = b.raw.as_ref().filter(|t| !t.trim().is_empty()) {
                        out.push_str(&format!("<h4>Body ({})</h4><pre>{}</pre>", esc(b.language()), esc(t)));
                    }
                }
                "urlencoded" => html_table(out, "Body (x-www-form-urlencoded)", &b.urlencoded),
                "formdata" => html_table(out, "Body (form-data)", &b.formdata),
                "graphql" => {
                    if let Some(g) = &b.graphql {
                        out.push_str(&format!("<h4>GraphQL</h4><pre>{}</pre>", esc(&g.query)));
                    }
                }
                _ => {}
            }
        }
        for ex in &it.response {
            out.push_str(&format!("<h4>Example: {} <span class=\"code\">{} {}</span></h4>", esc(&ex.name), ex.code, esc(&ex.status)));
            if !ex.body.trim().is_empty() {
                out.push_str(&format!("<pre>{}</pre>", esc(&ex.body)));
            }
        }
        out.push_str("</article>");
    }
}

pub fn html(c: &Collection) -> String {
    let mut body = String::new();
    let mut nav = String::new();
    html_items(&mut body, &mut nav, &c.item, 0);
    let mut intro = String::new();
    if let Some(d) = &c.info.description {
        intro.push_str(&format!("<p>{}</p>", esc(d).replace('\n', "<br>")));
    }
    if let Some(a) = &c.auth {
        intro.push_str(&format!("<p><b>Authorization:</b> {}</p>", esc(&auth_line(a))));
    }
    html_table(&mut intro, "Variables", &c.variable);
    format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{name} – API documentation</title>
<style>
body{{font:15px/1.6 system-ui,sans-serif;margin:0;color:#1f2430;display:flex}}
nav{{width:280px;flex:none;height:100vh;position:sticky;top:0;overflow:auto;background:#f6f7f9;border-right:1px solid #e3e6eb;padding:16px 0;box-sizing:border-box}}
nav h1{{font-size:16px;margin:0 16px 12px}} nav a{{display:block;padding:4px 16px;color:#1f2430;text-decoration:none;font-size:13px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}}
nav a:hover{{background:#e9ecf1}} nav a.f{{font-weight:600;margin-top:6px}} nav .d1{{padding-left:28px}} nav .d2{{padding-left:40px}} nav .d3{{padding-left:52px}}
main{{flex:1;max-width:900px;padding:24px 40px;min-width:0}}
article{{border-top:1px solid #e3e6eb;padding:16px 0}} h3{{margin:0 0 8px}} h4{{margin:16px 0 6px;font-size:13px;text-transform:uppercase;letter-spacing:.04em;color:#667085}}
pre{{background:#1f2430;color:#e6e9ef;padding:12px 14px;border-radius:6px;overflow:auto;font-size:13px}} pre.url{{background:#f0f2f5;color:#1f2430}}
table{{border-collapse:collapse;width:100%;font-size:13px}} td,th{{border:1px solid #e3e6eb;padding:5px 8px;text-align:left;vertical-align:top}} th{{background:#f6f7f9}} tr.off{{opacity:.5}}
.m{{font-size:11px;margin-right:6px;color:#667085}} .GET{{color:#17803d}} .POST{{color:#b7791f}} .PUT{{color:#2563eb}} .PATCH{{color:#7c3aed}} .DELETE{{color:#c53030}}
.code{{font-weight:400;color:#667085;text-transform:none}}
@media (max-width:800px){{body{{display:block}} nav{{width:auto;height:auto;position:static}} main{{padding:16px}}}}
</style></head><body>
<nav><h1>{name}</h1>{nav}</nav>
<main><h1>{name}</h1>{intro}{body}</main>
</body></html>
"#,
        name = esc(&c.info.name),
        nav = nav,
        intro = intro,
        body = body
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Body, Example, Request};

    #[test]
    fn renders_markdown_and_html() {
        let mut c = Collection::new("Docs <API>");
        c.info.description = Some("Top level".into());
        c.variable.push(KV::new("base", "http://x"));
        let mut folder = Item::folder("Users");
        let mut r = Request::new("POST", "{{base}}/users/:id?verbose=1");
        r.description = Some("Creates a user".into());
        r.header.push(KV::new("X-A", "a|b"));
        r.body = Some(Body::raw("{\"name\": \"<n>\"}", "json"));
        let mut it = Item::request("Create", r);
        it.response.push(Example { name: "ok".into(), code: 201, status: "Created".into(), body: "{\"id\":1}".into(), ..Default::default() });
        folder.item.as_mut().unwrap().push(it);
        c.item.push(folder);

        let md = markdown(&c);
        assert!(md.starts_with("# Docs <API>\n"));
        for needle in ["## Users", "### POST Create", "POST {{base}}/users/:id?verbose=1", "| X-A | a\\|b |", "```json", "**Example: ok** — `201 Created`", "| verbose | 1 |", "| id |"] {
            assert!(md.contains(needle), "missing {needle} in\n{md}");
        }
        let h = html(&c);
        assert!(h.contains("<title>Docs &lt;API&gt; – API documentation</title>"));
        assert!(h.contains("&lt;n&gt;") && !h.contains("<n>"));
        assert!(h.contains("class=\"m POST\""));
    }
}
