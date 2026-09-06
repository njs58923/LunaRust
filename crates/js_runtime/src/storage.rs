//! Persistent Web Storage. Document identity is supplied only by the host.
use deno_core::{op2, OpState};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};

pub const QUOTA: usize = 5 * 1024 * 1024;

pub fn origin_for_url(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    match url.scheme() {
        "http" | "https" => Some(url.origin().ascii_serialization()),
        // luna://home and luna://settings are routes, not independent hosts.
        "luna" => Some("luna://internal".into()),
        _ => None,
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Change {
    key: Option<String>,
    old_value: Option<String>,
    new_value: Option<String>,
    url: String,
}
struct Subscriber {
    path: PathBuf,
    origin: String,
    events: Weak<Mutex<VecDeque<Change>>>,
}
static SUBSCRIBERS: OnceLock<Mutex<Vec<Subscriber>>> = OnceLock::new();

#[derive(Default)]
pub struct StorageContext {
    path: PathBuf,
    origin: Option<String>,
    url: String,
    connection: Option<Connection>,
    events: Option<Arc<Mutex<VecDeque<Change>>>>,
}

impl StorageContext {
    pub fn new(path: PathBuf, url: String) -> Self {
        Self {
            origin: origin_for_url(&url),
            path,
            url,
            ..Self::default()
        }
    }
    fn connection(&mut self) -> Result<&mut Connection, String> {
        if self.origin.is_none() {
            return Err("SecurityError".into());
        }
        if self.connection.is_none() {
            let init = || -> anyhow::Result<Connection> {
                if let Some(parent) = self.path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let conn = Connection::open(&self.path)?;
                conn.busy_timeout(Duration::from_millis(500))?;
                conn.execute_batch(
                    "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
                    CREATE TABLE IF NOT EXISTS local_storage (
                        origin TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
                        bytes INTEGER NOT NULL, PRIMARY KEY(origin, key));",
                )?;
                Ok(conn)
            };
            self.connection = Some(init().map_err(|_| "InvalidStateError")?);
        }
        Ok(self.connection.as_mut().unwrap())
    }
    fn broadcast(&self, change: Change) {
        let mut subscribers = SUBSCRIBERS.get_or_init(Default::default).lock().unwrap();
        subscribers.retain(|s| s.events.strong_count() > 0);
        for s in subscribers.iter() {
            if s.path != self.path || Some(&s.origin) != self.origin.as_ref() {
                continue;
            }
            if let Some(queue) = s.events.upgrade() {
                if self
                    .events
                    .as_ref()
                    .is_some_and(|mine| Arc::ptr_eq(mine, &queue))
                {
                    continue;
                }
                queue.lock().unwrap().push_back(change.clone());
            }
        }
    }
    fn watch(&mut self, enabled: bool) {
        if !enabled {
            self.events = None;
            return;
        }
        if self.events.is_some() {
            return;
        }
        if let Some(origin) = &self.origin {
            let events = Arc::new(Mutex::new(VecDeque::new()));
            let mut subscribers = SUBSCRIBERS.get_or_init(Default::default).lock().unwrap();
            subscribers.retain(|s| s.events.strong_count() > 0);
            subscribers.push(Subscriber {
                path: self.path.clone(),
                origin: origin.clone(),
                events: Arc::downgrade(&events),
            });
            self.events = Some(events);
        }
    }
    fn request(
        &mut self,
        action: &str,
        key: &str,
        value: &str,
    ) -> Result<serde_json::Value, String> {
        if action == "watch" {
            self.watch(value == "true");
            return Ok(serde_json::Value::Null);
        }
        if action == "events" {
            let changes: Vec<_> = self
                .events
                .as_ref()
                .map(|q| q.lock().unwrap().drain(..).collect())
                .unwrap_or_default();
            return Ok(serde_json::to_value(changes).unwrap());
        }
        let origin = self.origin.clone().ok_or("SecurityError")?;
        if action == "check" {
            self.connection()?;
            return Ok(serde_json::Value::Null);
        }
        // All SQL uses parameters; JS cannot select a different origin or file.
        let url = self.url.clone();
        let conn = self.connection()?;
        let result = (|| -> Result<(serde_json::Value, Option<Change>), rusqlite::Error> {
            if action == "get" {
                let v: Option<String> = conn.query_row("SELECT value FROM local_storage WHERE origin=?1 AND key=?2",params![origin,key],|r|r.get(0)).optional()?;
                return Ok((serde_json::json!(v),None));
            }
            if action == "keys" {
                let mut stmt = conn.prepare("SELECT key FROM local_storage WHERE origin=?1 ORDER BY rowid")?;
                let keys = stmt.query_map([&origin],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
                return Ok((serde_json::json!(keys),None));
            }
            if action == "length" {
                let count: i64 = conn.query_row("SELECT COUNT(*) FROM local_storage WHERE origin=?1",[&origin],|r|r.get(0))?;
                return Ok((serde_json::json!(count),None));
            }
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let old: Option<String> = tx.query_row("SELECT value FROM local_storage WHERE origin=?1 AND key=?2",params![origin,key],|r|r.get(0)).optional()?;
            let change = match action {
                "set" if old.as_deref() != Some(value) => {
                    let used: i64 = tx.query_row("SELECT COALESCE(SUM(bytes),0) FROM local_storage WHERE origin=?1 AND key<>?2",params![origin,key],|r|r.get(0))?;
                    if used as usize + key.len() + value.len() > QUOTA {
                        return Ok((serde_json::json!({"quota":true}),None));
                    }
                    tx.execute("INSERT INTO local_storage(origin,key,value,bytes) VALUES(?1,?2,?3,?4)
                        ON CONFLICT(origin,key) DO UPDATE SET value=excluded.value,bytes=excluded.bytes",
                        params![origin,key,value,(key.len()+value.len()) as i64])?;
                    Some(Change{key:Some(key.into()),old_value:old,new_value:Some(value.into()),url})
                },
                "remove" if old.is_some() => {
                    tx.execute("DELETE FROM local_storage WHERE origin=?1 AND key=?2",params![origin,key])?;
                    Some(Change{key:Some(key.into()),old_value:old,new_value:None,url})
                },
                "clear" => {
                    let removed = tx.execute("DELETE FROM local_storage WHERE origin=?1",[&origin])?;
                    (removed > 0).then(||Change{key:None,old_value:None,new_value:None,url})
                },
                _ => None,
            };
            tx.commit()?;
            Ok((serde_json::Value::Null,change))
        })().map_err(|_|"InvalidStateError".to_string())?;
        if result.0.get("quota").is_some() {
            return Err("QuotaExceededError".into());
        }
        if let Some(change) = result.1 {
            self.broadcast(change);
        }
        Ok(result.0)
    }
}

#[op2]
#[serde]
pub fn op_local_storage(
    state: &mut OpState,
    #[string] action: &str,
    #[string] key: &str,
    #[string] value: &str,
) -> serde_json::Value {
    match state
        .borrow_mut::<StorageContext>()
        .request(action, key, value)
    {
        Ok(value) => serde_json::json!({"value":value}),
        Err(error) => serde_json::json!({"error":error}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn javascript_storage_contract_and_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage.sqlite");
        let mut engine = crate::Engine::new();
        engine.configure_local_storage(path.clone(), "https://example.test/a".into());
        engine.eval(r#"
            function check(ok,msg) { if (!ok) throw new Error(msg); }
            const s = localStorage;
            check(s === window.localStorage && s instanceof Storage, 'identity');
            check(s.getItem('missing') === null && s.missing === undefined, 'missing value');
            check(s.setItem('number', 42) === undefined && s.number === '42', 'string coercion');
            s.flag = false; s['unicode'] = '\ud800🍃';
            check(s.unicode === '\ud800🍃', 'UTF-16 round trip');
            check(s.length === 3 && s.key(0) === 'number' && s.key(99) === null, 'length/key');
            check(Object.keys(s).join(',') === 'number,flag,unicode', 'enumeration');
            s.setItem('__proto__','safe');
            check(s.getItem('__proto__') === 'safe' && Object.getPrototypeOf(s) === Storage.prototype, 'prototype');
            delete s.flag; check(s.getItem('flag') === null, 'delete');
            s.removeItem('missing');
            let missingArgument=false;
            try { s.setItem('key'); } catch (e) { missingArgument = e instanceof TypeError; }
            check(missingArgument, 'missing argument');
            let quota=false;
            try { s.setItem('number','x'.repeat(5*1024*1024)); } catch(e) { quota=e.name==='QuotaExceededError'; }
            check(quota && s.number === '42', 'atomic quota failure');
            s.clear(); s.setItem('saved', JSON.stringify({theme:'green'}));
            globalThis.location = {origin:'https://other.test',href:'https://other.test/'};
            check(s.saved === '{"theme":"green"}', 'mutable JS origin changed storage');
        "#).unwrap();
        drop(engine);
        let mut engine = crate::Engine::new();
        engine.configure_local_storage(path.clone(), "https://example.test/b".into());
        engine.eval("if (JSON.parse(localStorage.saved).theme !== 'green') throw new Error('not persisted');").unwrap();
        drop(engine);
        let mut denied = crate::Engine::new();
        denied.configure_local_storage(path, "data:text/plain,opaque".into());
        denied.eval("let denied=false; try { localStorage; } catch(e) { denied=e.name==='SecurityError'; } if (!denied) throw new Error('opaque origin allowed');").unwrap();
    }

    #[test]
    fn javascript_storage_events_reach_other_isolates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage.sqlite");
        let mut a = crate::Engine::new();
        a.configure_local_storage(path.clone(), "https://a.test/writer".into());
        let mut b = crate::Engine::new();
        b.configure_local_storage(path, "https://a.test/listener".into());
        b.eval(
            r#"
            globalThis.events=[];
            onstorage=e=>events.push(e);
        "#,
        )
        .unwrap();
        a.eval("localStorage.setItem('theme','green');").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        b.fire_raf(40.0);
        b.eval(
            r#"
            if (events.length!==1 || events[0].key!=='theme' || events[0].oldValue!==null ||
                events[0].newValue!=='green' || events[0].url!=='https://a.test/writer' ||
                events[0].storageArea!==localStorage || !(events[0] instanceof StorageEvent))
                throw new Error('incorrect storage event');
            onstorage=null;
            let once=0;
            addEventListener('storage',()=>once++,{once:true});
            dispatchEvent(new StorageEvent('storage')); dispatchEvent(new StorageEvent('storage'));
            if (once!==1) throw new Error('once listener');
        "#,
        )
        .unwrap();
        assert!(
            !b.needs_continuous_ticks(),
            "storage listener left a polling timer alive"
        );
    }

    #[test]
    fn concurrent_connections_do_not_lose_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage.sqlite");
        let mut seed = StorageContext::new(path.clone(), "https://a.test".into());
        seed.request("check", "", "").unwrap();
        let threads: Vec<_> = (0..4)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let mut storage = StorageContext::new(path, "https://a.test".into());
                    for j in 0..20 {
                        storage.request("set", &format!("{i}-{j}"), "v").unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(seed.request("length", "", "").unwrap(), 80);
    }
    #[test]
    fn origins_are_normalized_and_opaque_urls_are_denied() {
        assert_eq!(
            origin_for_url("https://EXAMPLE.com:443/a"),
            origin_for_url("https://example.com/b")
        );
        assert_ne!(
            origin_for_url("http://example.com"),
            origin_for_url("https://example.com")
        );
        assert_ne!(
            origin_for_url("https://example.com:444"),
            origin_for_url("https://example.com")
        );
        assert_ne!(
            origin_for_url("https://sub.example.com"),
            origin_for_url("https://example.com")
        );
        assert_eq!(
            origin_for_url("luna://home"),
            origin_for_url("luna://settings")
        );
        for raw in ["file:///secret", "data:text/plain,test", "", "not a URL"] {
            assert!(origin_for_url(raw).is_none());
        }
    }
    #[test]
    fn persistence_isolation_quota_and_cross_connection_visibility() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage.sqlite");
        let mut a = StorageContext::new(path.clone(), "https://a.test/one".into());
        let mut b = StorageContext::new(path.clone(), "https://a.test/two".into());
        let mut other = StorageContext::new(path.clone(), "https://b.test/".into());
        a.request("set", "k", "old").unwrap();
        assert_eq!(b.request("get", "k", "").unwrap(), "old");
        assert_eq!(
            other.request("get", "k", "").unwrap(),
            serde_json::Value::Null
        );
        assert_eq!(
            a.request("set", "k", &"x".repeat(QUOTA)).unwrap_err(),
            "QuotaExceededError"
        );
        assert_eq!(b.request("get", "k", "").unwrap(), "old");
        b.request("set", "second", "v").unwrap();
        drop(a);
        drop(b);
        let mut reopened = StorageContext::new(path, "https://a.test/three".into());
        assert_eq!(reopened.request("length", "", "").unwrap(), 2);
        other.request("clear", "", "").unwrap();
        assert_eq!(reopened.request("get", "k", "").unwrap(), "old");
        reopened.request("remove", "k", "").unwrap();
        assert_eq!(
            reopened.request("keys", "", "").unwrap(),
            serde_json::json!(["second"])
        );
    }
    #[test]
    fn notifications_exclude_writer_other_origins_and_noops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("storage.sqlite");
        let mut a = StorageContext::new(path.clone(), "https://a.test/one".into());
        let mut b = StorageContext::new(path.clone(), "https://a.test/two".into());
        let mut c = StorageContext::new(path, "https://other.test/".into());
        a.watch(true);
        b.watch(true);
        c.watch(true);
        a.request("set", "k", "v").unwrap();
        a.request("set", "k", "v").unwrap();
        a.request("remove", "missing", "").unwrap();
        assert_eq!(a.request("events", "", "").unwrap(), serde_json::json!([]));
        assert_eq!(c.request("events", "", "").unwrap(), serde_json::json!([]));
        let changes = b.request("events", "", "").unwrap();
        assert_eq!(changes.as_array().unwrap().len(), 1);
        assert_eq!(changes[0]["url"], "https://a.test/one");
        a.request("clear", "", "").unwrap();
        assert_eq!(
            b.request("events", "", "").unwrap()[0]["key"],
            serde_json::Value::Null
        );
    }
}
