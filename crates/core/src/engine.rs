//! Request execution on top of a workspace: variable scopes, auth inheritance,
//! cookies and history.

use crate::cookies::CookieJar;
use crate::http::{self, Prepared, Response, Settings};
use crate::model::{Auth, Collection, Environment, Request};
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
        })
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

    /// Send a single request with the given (already inherited) auth.
    pub fn send(&mut self, name: &str, req: &Request, auth: Option<&Auth>) -> Exchange {
        let mut ex = Exchange { id: crate::model::new_id(), name: name.to_string(), ..Default::default() };
        match http::prepare(req, auth, &self.scopes) {
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
        ex
    }

    /// Send a request stored in a collection (auth is inherited from its parents).
    pub fn send_item(&mut self, collection: &Collection, item_id: &str) -> Exchange {
        let Some(item) = collection.find(item_id) else {
            return Exchange { id: crate::model::new_id(), error: Some(format!("item `{item_id}` not found")), ..Default::default() };
        };
        let Some(req) = item.request.as_ref() else {
            return Exchange { id: crate::model::new_id(), name: item.name.clone(), error: Some("item is a folder".into()), ..Default::default() };
        };
        let auth = collection.effective_auth(item_id);
        let mut ex = self.send(&item.name, req, auth.as_ref());
        ex.item_id = Some(item_id.to_string());
        ex
    }
}

/// Record an exchange in the workspace history.
pub fn record(ws: &Workspace, settings: &Settings, ex: &Exchange, source: Option<&Request>, collection_id: Option<&str>) -> Result<()> {
    ws.append_history(
        HistoryEntry {
            id: ex.id.clone(),
            timestamp: chrono::Utc::now(),
            name: if ex.name.is_empty() { None } else { Some(ex.name.clone()) },
            collection_id: collection_id.map(|s| s.to_string()),
            item_id: ex.item_id.clone(),
            source: source.cloned(),
            request: ex.request.clone(),
            response: ex.response.clone(),
            error: ex.error.clone(),
        },
        settings,
    )
}
