//! Local workspace storage. Everything is plain JSON under one directory:
//!
//! ```text
//! <root>/collections/<id>.json     Postman Collection v2.1
//! <root>/environments/<id>.json    Postman environment
//! <root>/globals.json              Postman globals
//! <root>/history.jsonl             one exchange per line
//! <root>/cookies.json  settings.json  state.json
//! ```

use crate::cookies::CookieJar;
use crate::http::{Prepared, Response, Settings};
use crate::model::{Collection, Environment, Request};
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct State {
    /// Id of the active environment.
    #[serde(default)]
    pub active_environment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub collection_id: Option<String>,
    #[serde(default)]
    pub item_id: Option<String>,
    /// The editable (unresolved) request, used to re-open or replay the entry.
    #[serde(default)]
    pub source: Option<Request>,
    /// What was actually sent.
    #[serde(default)]
    pub request: Option<Prepared>,
    #[serde(default)]
    pub response: Option<Response>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
}

fn safe(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(serde_json::from_str(&s).with_context(|| format!("corrupt file {}", path.display()))?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

/// Restrict a file to its owner (the workspace holds credentials). No-op outside Unix.
pub fn make_private(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if path.is_dir() { 0o700 } else { 0o600 };
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Create a directory (and parents) that only the owner can enter. Existing directories keep
/// their permissions – the user may have chosen them on purpose.
pub fn create_private_dir(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(path)?;
    make_private(path);
    Ok(())
}

/// Atomic, owner-only write (temp file + rename).
pub fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).with_context(|| format!("cannot write {}", tmp.display()))?;
        f.write_all(data).with_context(|| format!("cannot write {}", tmp.display()))?;
    }
    make_private(&tmp);
    std::fs::rename(&tmp, path).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

fn write_file(path: &Path, text: &str) -> Result<()> {
    write_private(path, text.as_bytes())
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<()> {
    write_file(path, &serde_json::to_string_pretty(v)?)
}

impl Workspace {
    /// Open (and create) a workspace. Order: explicit path, `$RSPOSTMAN_HOME`, `~/.rspostman`.
    pub fn open(root: Option<PathBuf>) -> Result<Self> {
        let root = root
            .or_else(|| std::env::var_os("RSPOSTMAN_HOME").map(PathBuf::from))
            .or_else(|| dirs::home_dir().map(|h| h.join(".rspostman")))
            .ok_or_else(|| anyhow!("cannot determine workspace directory; pass --workspace"))?;
        create_private_dir(&root).with_context(|| format!("cannot create workspace at {}", root.display()))?;
        for d in ["collections", "environments"] {
            create_private_dir(&root.join(d)).with_context(|| format!("cannot create workspace at {}", root.display()))?;
        }
        Ok(Workspace { root })
    }

    fn list<T: DeserializeOwned>(&self, dir: &str) -> Result<Vec<T>> {
        let mut out = vec![];
        for e in std::fs::read_dir(self.root.join(dir))? {
            let p = e?.path();
            if p.extension().and_then(|x| x.to_str()) == Some("json") {
                if let Some(v) = read_json(&p)? {
                    out.push(v);
                }
            }
        }
        Ok(out)
    }

    // ---------------------------------------------------------------- collections

    pub fn collections(&self) -> Result<Vec<Collection>> {
        let mut v: Vec<Collection> = self.list("collections")?;
        v.sort_by(|a, b| a.info.name.to_lowercase().cmp(&b.info.name.to_lowercase()));
        Ok(v)
    }

    pub fn collection(&self, id: &str) -> Result<Option<Collection>> {
        read_json(&self.root.join("collections").join(format!("{}.json", safe(id))))
    }

    pub fn save_collection(&self, c: &Collection) -> Result<()> {
        write_json(&self.root.join("collections").join(format!("{}.json", safe(c.id()))), c)
    }

    pub fn delete_collection(&self, id: &str) -> Result<bool> {
        let p = self.root.join("collections").join(format!("{}.json", safe(id)));
        Ok(std::fs::remove_file(p).is_ok())
    }

    /// Find a collection by id, name (case-insensitive) or path to an exported file.
    pub fn find_collection(&self, reference: &str) -> Result<Collection> {
        if let Some(c) = self.collection(reference)? {
            return Ok(c);
        }
        let all = self.collections()?;
        let lower = reference.to_lowercase();
        let hits: Vec<&Collection> = all.iter().filter(|c| c.info.name.to_lowercase() == lower).collect();
        match hits.len() {
            1 => return Ok(hits[0].clone()),
            n if n > 1 => bail!("{n} collections are named `{reference}`; use the id instead"),
            _ => {}
        }
        let p = Path::new(reference);
        if p.is_file() {
            let mut c = Collection::from_json(&std::fs::read_to_string(p)?).with_context(|| format!("cannot load collection file {reference}"))?;
            // A collection loaded from a file is transient: it must never be mistaken for (and
            // persisted over) a stored collection that happens to share its id.
            c.info.id = crate::model::new_id();
            return Ok(c);
        }
        bail!("collection `{reference}` not found (not an id, a name, or a file)")
    }

    /// Import a collection, generating a fresh id when one with the same id exists.
    pub fn import_collection(&self, mut c: Collection) -> Result<Collection> {
        if self.collection(c.id())?.is_some() {
            c.info.id = crate::model::new_id();
        }
        self.save_collection(&c)?;
        Ok(c)
    }

    // ---------------------------------------------------------------- environments

    pub fn environments(&self) -> Result<Vec<Environment>> {
        let mut v: Vec<Environment> = self.list("environments")?;
        v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(v)
    }

    pub fn environment(&self, id: &str) -> Result<Option<Environment>> {
        read_json(&self.root.join("environments").join(format!("{}.json", safe(id))))
    }

    pub fn save_environment(&self, e: &Environment) -> Result<()> {
        write_json(&self.root.join("environments").join(format!("{}.json", safe(&e.id))), e)
    }

    pub fn delete_environment(&self, id: &str) -> Result<bool> {
        let p = self.root.join("environments").join(format!("{}.json", safe(id)));
        let ok = std::fs::remove_file(p).is_ok();
        let mut st = self.state()?;
        if st.active_environment.as_deref() == Some(id) {
            st.active_environment = None;
            self.save_state(&st)?;
        }
        Ok(ok)
    }

    pub fn find_environment(&self, reference: &str) -> Result<Environment> {
        if let Some(e) = self.environment(reference)? {
            return Ok(e);
        }
        let lower = reference.to_lowercase();
        if let Some(e) = self.environments()?.into_iter().find(|e| e.name.to_lowercase() == lower) {
            return Ok(e);
        }
        let p = Path::new(reference);
        if p.is_file() {
            let mut e = Environment::from_json(&std::fs::read_to_string(p)?).with_context(|| format!("cannot load environment file {reference}"))?;
            e.id = crate::model::new_id();
            return Ok(e);
        }
        bail!("environment `{reference}` not found (not an id, a name, or a file)")
    }

    pub fn active_environment(&self) -> Result<Option<Environment>> {
        match self.state()?.active_environment {
            Some(id) => self.environment(&id),
            None => Ok(None),
        }
    }

    pub fn globals(&self) -> Result<Environment> {
        Ok(read_json(&self.root.join("globals.json"))?.unwrap_or_else(|| Environment {
            id: crate::model::new_id(),
            name: "Globals".into(),
            values: vec![],
            scope: Some("globals".into()),
        }))
    }

    pub fn save_globals(&self, g: &Environment) -> Result<()> {
        write_json(&self.root.join("globals.json"), g)
    }

    // ---------------------------------------------------------------- misc state

    pub fn state(&self) -> Result<State> {
        Ok(read_json(&self.root.join("state.json"))?.unwrap_or_default())
    }
    pub fn save_state(&self, s: &State) -> Result<()> {
        write_json(&self.root.join("state.json"), s)
    }
    pub fn settings(&self) -> Result<Settings> {
        Ok(read_json(&self.root.join("settings.json"))?.unwrap_or_default())
    }
    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        write_json(&self.root.join("settings.json"), s)
    }
    pub fn cookies(&self) -> Result<CookieJar> {
        let mut jar: CookieJar = read_json(&self.root.join("cookies.json"))?.unwrap_or_default();
        jar.purge_expired();
        Ok(jar)
    }
    pub fn save_cookies(&self, jar: &CookieJar) -> Result<()> {
        write_json(&self.root.join("cookies.json"), jar)
    }

    // ---------------------------------------------------------------- history

    fn history_path(&self) -> PathBuf {
        self.root.join("history.jsonl")
    }

    /// Oldest first.
    pub fn history(&self) -> Result<Vec<HistoryEntry>> {
        let text = match std::fs::read_to_string(self.history_path()) {
            Ok(t) => t,
            Err(_) => return Ok(vec![]),
        };
        Ok(text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
    }

    pub fn history_entry(&self, reference: &str) -> Result<HistoryEntry> {
        let all = self.history()?;
        // 1-based index counted from the most recent entry, or (prefix of) an id
        if let Ok(n) = reference.parse::<usize>() {
            if n >= 1 && n <= all.len() {
                return Ok(all[all.len() - n].clone());
            }
        }
        all.into_iter().rev().find(|e| e.id.starts_with(reference)).ok_or_else(|| anyhow!("history entry `{reference}` not found"))
    }

    pub fn append_history(&self, mut entry: HistoryEntry, settings: &Settings) -> Result<()> {
        if let Some(r) = entry.response.as_mut() {
            if r.body.len() > settings.history_body_limit {
                r.body.truncate(settings.history_body_limit);
            }
        }
        let line = serde_json::to_string(&entry)?;
        let fresh = !self.history_path().exists();
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.history_path())?;
        if fresh {
            make_private(&self.history_path());
        }
        writeln!(f, "{line}")?;
        drop(f);
        let all = self.history()?;
        if settings.history_limit > 0 && all.len() > settings.history_limit + 20 {
            let keep = &all[all.len() - settings.history_limit..];
            let text: String = keep.iter().filter_map(|e| serde_json::to_string(e).ok()).map(|l| l + "\n").collect();
            write_file(&self.history_path(), &text)?;
        }
        Ok(())
    }

    pub fn delete_history(&self, id: &str) -> Result<bool> {
        let all = self.history()?;
        let keep: Vec<&HistoryEntry> = all.iter().filter(|e| e.id != id).collect();
        let text: String = keep.iter().filter_map(|e| serde_json::to_string(e).ok()).map(|l| l + "\n").collect();
        write_file(&self.history_path(), &text)?;
        Ok(keep.len() != all.len())
    }

    pub fn clear_history(&self) -> Result<()> {
        let _ = std::fs::remove_file(self.history_path());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Item, Request};

    pub fn temp_ws() -> Workspace {
        let dir = std::env::temp_dir().join(format!("rspostman-test-{}", crate::model::new_id()));
        Workspace::open(Some(dir)).unwrap()
    }

    #[test]
    fn collections_and_environments_crud() {
        let ws = temp_ws();
        let mut c = Collection::new("My API");
        c.add_item(None, Item::request("ping", Request::new("GET", "http://x/ping"))).unwrap();
        ws.save_collection(&c).unwrap();
        assert_eq!(ws.find_collection("my api").unwrap(), c);
        assert_eq!(ws.find_collection(c.id()).unwrap(), c);
        let dup = ws.import_collection(c.clone()).unwrap();
        assert_ne!(dup.id(), c.id());
        assert!(ws.find_collection("My API").is_err(), "ambiguous name must be rejected");
        assert!(ws.delete_collection(dup.id()).unwrap());
        // a file with the same id as a stored collection is loaded detached
        let file = ws.root.join("export.json");
        std::fs::write(&file, c.to_json()).unwrap();
        let from_file = ws.find_collection(file.to_str().unwrap()).unwrap();
        assert_ne!(from_file.id(), c.id());
        assert!(ws.collection(from_file.id()).unwrap().is_none());
        assert_eq!(ws.collections().unwrap().len(), 1);

        let mut e = Environment::new("dev");
        e.set("host", "localhost");
        ws.save_environment(&e).unwrap();
        ws.save_state(&State { active_environment: Some(e.id.clone()) }).unwrap();
        assert_eq!(ws.active_environment().unwrap().unwrap().get("host"), Some("localhost"));
        ws.delete_environment(&e.id).unwrap();
        assert!(ws.active_environment().unwrap().is_none());
        std::fs::remove_dir_all(&ws.root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn workspace_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let ws = temp_ws();
        let mode = |p: std::path::PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        ws.save_collection(&Collection::new("c")).unwrap();
        ws.save_globals(&ws.globals().unwrap()).unwrap();
        ws.save_cookies(&CookieJar::default()).unwrap();
        ws.append_history(
            HistoryEntry { id: "h".into(), timestamp: Utc::now(), name: None, collection_id: None, item_id: None, source: None, request: None, response: None, error: None },
            &Settings::default(),
        )
        .unwrap();
        assert_eq!(mode(ws.root.clone()), 0o700);
        assert_eq!(mode(ws.root.join("collections")), 0o700);
        for f in ["globals.json", "cookies.json", "history.jsonl"] {
            assert_eq!(mode(ws.root.join(f)), 0o600, "{f}");
        }
        let c = std::fs::read_dir(ws.root.join("collections")).unwrap().next().unwrap().unwrap().path();
        assert_eq!(mode(c), 0o600);
        // an existing directory chosen by the user is left alone
        let shared = std::env::temp_dir().join(format!("rsp-shared-{}", crate::model::new_id()));
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
        Workspace::open(Some(shared.clone())).unwrap();
        assert_eq!(mode(shared.clone()), 0o755);
        std::fs::remove_dir_all(&ws.root).ok();
        std::fs::remove_dir_all(&shared).ok();
    }

    #[test]
    fn history_is_bounded() {
        let ws = temp_ws();
        let st = Settings { history_limit: 5, history_body_limit: 4, ..Default::default() };
        for i in 0..40 {
            let e = HistoryEntry {
                id: format!("id{i}"),
                timestamp: Utc::now(),
                name: None,
                collection_id: None,
                item_id: None,
                source: None,
                request: None,
                response: Some(Response { body: b"0123456789".to_vec(), ..Default::default() }),
                error: None,
            };
            ws.append_history(e, &st).unwrap();
        }
        let h = ws.history().unwrap();
        assert!(h.len() <= 25 && h.len() >= 5);
        assert_eq!(h.last().unwrap().id, "id39");
        assert_eq!(h.last().unwrap().response.as_ref().unwrap().body, b"0123");
        assert_eq!(ws.history_entry("1").unwrap().id, "id39");
        assert_eq!(ws.history_entry("id38").unwrap().id, "id38");
        assert!(ws.delete_history("id39").unwrap());
        ws.clear_history().unwrap();
        assert!(ws.history().unwrap().is_empty());
        std::fs::remove_dir_all(&ws.root).ok();
    }
}
