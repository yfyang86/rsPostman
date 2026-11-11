//! Request execution on top of a workspace: variable scopes, auth inheritance,
//! cookies and history.

use crate::cookies::CookieJar;
use crate::http::{self, Prepared, Response, Settings};
use crate::model::{Auth, Collection, Environment, Request};
use crate::script::{self, Info, LogLine, ScriptContext, TestResult};
use crate::store::{HistoryEntry, Workspace};
use crate::vars::Scopes;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Mutable state shared by the requests of one run.
#[derive(Debug, Clone, Default)]
pub struct Session {
    pub scopes: Scopes,
    pub jar: CookieJar,
    pub settings: Settings,
    pub environment: Option<Environment>,
    pub globals: Environment,
    /// Zero-based iteration index and total, exposed to scripts as `pm.info`.
    pub iteration: usize,
    pub iteration_count: usize,
}

/// Scripts that apply to one request, outermost (collection) first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scripts {
    #[serde(default)]
    pub prerequest: Vec<String>,
    #[serde(default)]
    pub test: Vec<String>,
}

impl Scripts {
    /// Collection, folder and request level scripts of an item.
    pub fn for_item(collection: &Collection, item_id: &str) -> Scripts {
        let mut s = Scripts::default();
        for listen in ["prerequest", "test"] {
            let mut codes = vec![crate::model::script_of(&collection.event, listen)];
            for it in collection.trail(item_id).unwrap_or_default() {
                codes.push(it.script(listen));
            }
            codes.retain(|c| !c.trim().is_empty());
            if listen == "prerequest" {
                s.prerequest = codes;
            } else {
                s.test = codes;
            }
        }
        s
    }
}

/// `pm.execution.setNextRequest` outcome.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Next {
    /// `None` stops the run.
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Exchange {
    pub id: String,
    #[serde(default)]
    pub item_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub request: Option<Prepared>,
    #[serde(default)]
    pub response: Option<Response>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub tests: Vec<TestResult>,
    #[serde(default)]
    pub console: Vec<LogLine>,
    /// Uncaught error of a test script.
    #[serde(default)]
    pub script_error: Option<String>,
    /// Set when a script called `setNextRequest`.
    #[serde(default)]
    pub next: Option<Next>,
    /// The request was skipped by `pm.execution.skipRequest()`.
    #[serde(default)]
    pub skipped: bool,
    #[serde(default)]
    pub visualizer: Option<serde_json::Value>,
}

impl Exchange {
    pub fn failed_tests(&self) -> usize {
        self.tests.iter().filter(|t| !t.passed).count()
    }
    /// Network/script error or at least one failed assertion.
    pub fn failed(&self) -> bool {
        self.error.is_some() || self.script_error.is_some() || self.failed_tests() > 0
    }
}

impl Session {
    /// Load variable scopes, cookies and settings from the workspace.
    /// `environment`: explicit environment (name, id or file); falls back to the active one.
    pub fn load(ws: &Workspace, collection: Option<&Collection>, environment: Option<&str>) -> Result<Session> {
        let env = match environment {
            Some(r) => Some(ws.find_environment(r)?),
            None => ws.active_environment()?,
        };
        let globals = ws.globals()?;
        let settings = ws.settings()?;
        Ok(Session {
            scopes: Scopes {
                globals: globals.map(),
                collection: collection.map(|c| c.variables()).unwrap_or_default(),
                environment: env.as_ref().map(|e| e.map()).unwrap_or_default(),
                ..Default::default()
            },
            jar: if settings.cookies { ws.cookies()? } else { CookieJar::default() },
            settings,
            environment: env,
            globals,
            iteration: 0,
            iteration_count: 1,
        })
    }

    /// Make the secrets of an unlocked vault available as `{{vault:name}}` for this session.
    pub fn unlock_vault(&mut self, vault: &crate::vault::Vault) -> Result<()> {
        self.scopes.vault = vault.all()?;
        Ok(())
    }

    pub fn vault_values(&self) -> Vec<String> {
        self.scopes.vault.values().cloned().collect()
    }

    /// Write cookies and changed variables back to the workspace.
    pub fn persist(&mut self, ws: &Workspace, collection: Option<&mut Collection>) -> Result<()> {
        if self.settings.cookies {
            ws.save_cookies(&self.jar)?;
        }
        if self.globals.map() != self.scopes.globals {
            self.globals.apply_map(&self.scopes.globals);
            ws.save_globals(&self.globals)?;
        }
        if let Some(env) = self.environment.as_mut() {
            if env.map() != self.scopes.environment {
                env.apply_map(&self.scopes.environment);
                // only environments that live in the workspace are written back
                if ws.environment(&env.id)?.is_some() {
                    ws.save_environment(env)?;
                }
            }
        }
        if let Some(c) = collection {
            if c.variables() != self.scopes.collection {
                c.set_variables(&self.scopes.collection);
                if ws.collection(c.id())?.is_some() {
                    ws.save_collection(c)?;
                }
            }
        }
        Ok(())
    }

    /// Send a single request with the given (already inherited) auth, without scripts.
    pub fn send(&mut self, name: &str, req: &Request, auth: Option<&Auth>) -> Exchange {
        self.run(name, None, "", req, auth, &Scripts::default())
    }

    /// Full request lifecycle: pre-request scripts, send, test scripts.
    pub fn run(&mut self, name: &str, item_id: Option<&str>, collection_name: &str, req: &Request, auth: Option<&Auth>, scripts: &Scripts) -> Exchange {
        let mut ex = Exchange { id: crate::model::new_id(), name: name.to_string(), item_id: item_id.map(|s| s.to_string()), ..Default::default() };
        let mut req = req.clone();
        let info = |event: &str, s: &Session| Info {
            event_name: event.to_string(),
            iteration: s.iteration,
            iteration_count: s.iteration_count,
            request_name: name.to_string(),
            request_id: item_id.unwrap_or("").to_string(),
            collection_name: collection_name.to_string(),
            environment: s.environment.as_ref().map(|e| e.name.clone()).unwrap_or_default(),
        };
        let cookies = |s: &Session, req: &Request| -> Vec<(String, String)> {
            url::Url::parse(&http::build_url(&req.url, &s.scopes))
                .map(|u| s.jar.matching(&u).iter().map(|c| (c.name.clone(), c.value.clone())).collect())
                .unwrap_or_default()
        };

        for code in &scripts.prerequest {
            let ctx = ScriptContext { info: info("prerequest", self), cookies: cookies(self, &req), scopes: &mut self.scopes, request: &mut req, response: None, settings: &self.settings };
            let o = script::run(code, ctx);
            ex.tests.extend(o.tests);
            ex.console.extend(o.console);
            if let Some(n) = o.next {
                ex.next = Some(Next { name: n });
            }
            if let Some(e) = o.error {
                ex.error = Some(format!("pre-request script error: {e}"));
                return ex;
            }
            if o.skip {
                ex.skipped = true;
                return ex;
            }
        }

        self.scopes.vault_used.take();
        match http::prepare(&req, auth, &self.scopes) {
            Ok(p) => {
                let jar = if self.settings.cookies { Some(&mut self.jar) } else { None };
                match http::execute(&p, &self.settings, jar) {
                    Ok(r) => ex.response = Some(r),
                    Err(e) => ex.error = Some(e.to_string()),
                }
                ex.request = Some(p);
            }
            Err(e) => ex.error = Some(e.to_string()),
        }
        // A request that used the vault never exposes those secrets in its snapshot: not to
        // the GUI, not to verbose output, not to the history or reports.
        if self.scopes.vault_used.take() {
            let values = self.vault_values();
            ex.request = ex.request.as_ref().map(|p| crate::secrets::scrub_prepared(&crate::secrets::redact_prepared(p), &values));
            ex.error = ex.error.as_ref().map(|e| crate::secrets::scrub_text(e, &values));
        }

        if let Some(resp) = ex.response.clone() {
            for code in &scripts.test {
                let ctx = ScriptContext { info: info("test", self), cookies: cookies(self, &req), scopes: &mut self.scopes, request: &mut req, response: Some(&resp), settings: &self.settings };
                let o = script::run(code, ctx);
                ex.tests.extend(o.tests);
                ex.console.extend(o.console);
                if let Some(n) = o.next {
                    ex.next = Some(Next { name: n });
                }
                if o.visualizer.is_some() {
                    ex.visualizer = o.visualizer;
                }
                if let Some(e) = o.error {
                    ex.script_error = Some(format!("test script error: {e}"));
                    break;
                }
            }
        }
        ex
    }

    /// Run a request stored in a collection (auth and scripts are inherited from its parents).
    pub fn send_item(&mut self, collection: &Collection, item_id: &str) -> Exchange {
        let Some(item) = collection.find(item_id) else {
            return Exchange { id: crate::model::new_id(), error: Some(format!("item `{item_id}` not found")), ..Default::default() };
        };
        let Some(req) = item.request.as_ref() else {
            return Exchange { id: crate::model::new_id(), name: item.name.clone(), error: Some("item is a folder".into()), ..Default::default() };
        };
        let auth = collection.effective_auth(item_id);
        self.run(&item.name, Some(item_id), &collection.info.name, req, auth.as_ref(), &Scripts::for_item(collection, item_id))
    }
}

/// Record an exchange in the workspace history.
///
/// Unless `settings.history_store_secrets` is on, literal credentials are replaced by
/// `<redacted>` in the stored request, the wire snapshot and the response. Vault secrets are
/// always removed, wherever they appear.
pub fn record(ws: &Workspace, session: &Session, ex: &Exchange, source: Option<&Request>, collection_id: Option<&str>) -> Result<()> {
    let settings = &session.settings;
    let keep = settings.history_store_secrets;
    let vault = session.vault_values();
    ws.append_history(
        HistoryEntry {
            id: ex.id.clone(),
            timestamp: chrono::Utc::now(),
            name: if ex.name.is_empty() { None } else { Some(ex.name.clone()) },
            collection_id: collection_id.map(|s| s.to_string()),
            item_id: ex.item_id.clone(),
            source: source.map(|r| if keep { r.clone() } else { crate::secrets::redact_request(r) }),
            request: ex.request.as_ref().map(|p| crate::secrets::scrub_prepared(&if keep { p.clone() } else { crate::secrets::redact_prepared(p) }, &vault)),
            response: ex.response.as_ref().map(|r| crate::secrets::scrub_response(&if keep { r.clone() } else { crate::secrets::redact_response(r) }, &vault)),
            error: ex.error.as_ref().map(|e| crate::secrets::scrub_text(e, &vault)),
        },
        settings,
    )
}
