//! Bounded compilation cache for repeated include scripts. Only V8 bytecode is
//! reused: JavaScript execution, globals, callbacks and native state stay local.
use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    hash::{Hash, Hasher},
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::Result;
use deno_core::{v8, JsRuntime};

const MAX_ENTRIES: usize = 128;
const MAX_BYTES: usize = 8 * 1024 * 1024;

struct CompiledScript {
    name: Box<str>,
    source: Box<str>,
    bytecode: Box<[u8]>,
}

impl CompiledScript {
    fn size(&self) -> usize {
        self.name.len() + self.source.len() + self.bytecode.len()
    }
}

struct Entry {
    script: Arc<CompiledScript>,
    last_used: u64,
}

#[derive(Default)]
struct CompilationCache {
    entries: HashMap<u64, Entry>,
    bytes: usize,
    clock: u64,
}

impl CompilationCache {
    fn get(&mut self, key: u64, name: &str, source: &str) -> Option<Arc<CompiledScript>> {
        let entry = self.entries.get_mut(&key)?;
        // A hash collision is a miss, never permission to execute other code.
        if entry.script.name.as_ref() != name || entry.script.source.as_ref() != source {
            return None;
        }
        self.clock = self.clock.wrapping_add(1);
        entry.last_used = self.clock;
        Some(entry.script.clone())
    }

    fn insert(&mut self, key: u64, script: CompiledScript) {
        let size = script.size();
        if size > MAX_BYTES {
            return;
        }
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes -= previous.script.size();
        }
        while self.entries.len() >= MAX_ENTRIES || self.bytes + size > MAX_BYTES {
            let oldest = self.entries.iter().min_by_key(|(_, entry)| entry.last_used)
                .map(|(&key, _)| key).expect("nonempty compilation cache");
            self.bytes -= self.entries.remove(&oldest).unwrap().script.size();
        }
        self.clock = self.clock.wrapping_add(1);
        self.entries.insert(key, Entry { script: Arc::new(script), last_used: self.clock });
        self.bytes += size;
    }
}

fn cache() -> &'static Mutex<CompilationCache> {
    static CACHE: OnceLock<Mutex<CompilationCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(CompilationCache::default()))
}

fn source_key(name: &str, source: &str) -> u64 {
    let mut hash = DefaultHasher::new();
    name.hash(&mut hash);
    source.hash(&mut hash);
    hash.finish()
}

pub(crate) fn execute(runtime: &mut JsRuntime, name: &str, source: &str) -> Result<()> {
    let can_cache = name.len().saturating_add(source.len()) < MAX_BYTES;
    let key = can_cache.then(|| source_key(name, source));
    let cached = key.and_then(|key| cache().lock().ok()?.get(key, name, source));
    // Release the mutex before entering V8. An infinite script cannot block
    // compilation or execution in other workers, and watchdogs remain effective.
    let generated = compile_and_run(runtime, name, source,
        cached.as_ref().map(|script| script.bytecode.as_ref()), can_cache)?;
    if let (Some(key), Some(bytecode)) = (key, generated) {
        let script = CompiledScript { name: name.into(), source: source.into(), bytecode };
        if let Ok(mut cache) = cache().lock() {
            cache.insert(key, script);
        }
    }
    Ok(())
}

fn compile_and_run(
    runtime: &mut JsRuntime,
    name: &str,
    code: &str,
    cached: Option<&[u8]>,
    generate_cache: bool,
) -> Result<Option<Box<[u8]>>> {
    use v8::script_compiler::{self, CompileOptions, NoCacheReason, Source};
    let scope = &mut runtime.handle_scope();
    let source = v8::String::new(scope, code)
        .ok_or_else(|| anyhow::anyhow!("could not allocate script source"))?;
    let resource_name = v8::String::new(scope, name)
        .ok_or_else(|| anyhow::anyhow!("could not allocate script name"))?;
    let source_map_url = v8::String::empty(scope);
    let origin = v8::ScriptOrigin::new(scope, resource_name.into(), 0, 0, false,
        123, source_map_url.into(), true, false, false);
    let (source, options) = match cached {
        Some(bytes) => (Source::new_with_cached_data(source, Some(&origin), v8::CachedData::new(bytes)),
            CompileOptions::ConsumeCodeCache),
        None => (Source::new(source, Some(&origin)), CompileOptions::NoCompileOptions),
    };
    let scope = &mut v8::TryCatch::new(scope);
    let script = match script_compiler::compile(scope, source, options, NoCacheReason::NoReason) {
        Some(script) => script,
        None => return Err(exception(scope)),
    };
    if script.run(scope).is_none() {
        return Err(exception(scope));
    }
    if generate_cache && cached.is_none() {
        if let Some(bytes) = script.get_unbound_script(scope).create_code_cache() {
            if bytes.len().saturating_add(name.len()).saturating_add(code.len()) <= MAX_BYTES {
                return Ok(Some(bytes.to_vec().into_boxed_slice()));
            }
        }
    }
    Ok(None)
}

fn exception(scope: &mut v8::TryCatch<v8::HandleScope>) -> anyhow::Error {
    match scope.exception() {
        Some(exception) => deno_core::error::JsError::from_v8_exception(scope, exception).into(),
        None => anyhow::anyhow!("script execution terminated"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Engine;
    use std::time::Instant;

    fn entry(name: &str, source: &str, bytes: usize) -> CompiledScript {
        CompiledScript { name: name.into(), source: source.into(), bytecode: vec![0; bytes].into() }
    }

    #[test]
    fn cache_bounds_entries_bytes_and_evicts_least_recently_used() {
        let mut cache = CompilationCache::default();
        for key in 0..MAX_ENTRIES as u64 {
            cache.insert(key, entry("script", "source", 1));
        }
        assert!(cache.get(0, "script", "source").is_some());
        cache.insert(MAX_ENTRIES as u64, entry("script", "source", 1));
        assert!(cache.get(0, "script", "source").is_some());
        assert!(cache.get(1, "script", "source").is_none());
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        for key in 1000..1020 {
            cache.insert(key, entry("script", "source", 1024 * 1024));
            assert!(cache.bytes <= MAX_BYTES);
            assert_eq!(cache.bytes, cache.entries.values().map(|entry| entry.script.size()).sum::<usize>());
        }
        let before = cache.bytes;
        cache.insert(2000, entry("script", "source", MAX_BYTES));
        assert_eq!(cache.bytes, before, "oversized scripts must not evict reusable entries");
    }

    #[test]
    fn hash_collision_is_a_cache_miss() {
        let mut cache = CompilationCache::default();
        cache.insert(42, entry("a.js", "code A", 16));
        assert!(cache.get(42, "a.js", "code B").is_none());
        assert!(cache.get(42, "b.js", "code A").is_none());
        assert!(cache.get(42, "a.js", "code A").is_some());
    }

    #[test]
    fn cached_eval_reexecutes_effects_with_independent_globals_and_callbacks() {
        let name = "https://cache.test/repeated-effects.js";
        let source = "globalThis.cachedCounter=(globalThis.cachedCounter||0)+1; setTimeout(()=>console.log('cached:'+cachedCounter),0);";
        {
            let mut engine = Engine::new();
            engine.eval_cached(name, source).unwrap();
            engine.eval_cached(name, source).unwrap();
            engine.eval("if(cachedCounter!==2)throw Error('cached script was not reexecuted');").unwrap();
            engine.fire_raf(1.0);
            assert_eq!(engine.drain_logs().iter().filter(|(_, message)| message == "cached:2").count(), 2);
        }
        assert!(cache().lock().unwrap().get(source_key(name, source), name, source).is_some());
        let mut engine = Engine::new();
        assert!(engine.next_timer_deadline().is_none());
        engine.eval_cached(name, source).unwrap();
        engine.eval("if(cachedCounter!==1)throw Error('cached script shared globals');").unwrap();
        engine.fire_raf(1.0);
        assert_eq!(engine.drain_logs().iter().filter(|(_, message)| message == "cached:1").count(), 1);
    }

    #[test]
    fn same_url_source_updates_and_errors_do_not_reuse_stale_code() {
        let name = "https://cache.test/reloaded.js";
        let mut engine = Engine::new();
        engine.eval_cached(name, "globalThis.version = 1;").unwrap();
        engine.eval_cached(name, "globalThis.version = 2;").unwrap();
        engine.eval("if(version!==2)throw Error('source update lost');").unwrap();
        engine.eval_cached(name, "globalThis.version = 1;").unwrap();
        engine.eval("if(version!==1)throw Error('cached source skipped');").unwrap();
        let invalid = "function (";
        assert!(engine.eval_cached(name, invalid).unwrap_err().to_string().contains("SyntaxError"));
        assert!(cache().lock().unwrap().get(source_key(name, invalid), name, invalid).is_none());
        let throws = "globalThis.failedRuns=(globalThis.failedRuns||0)+1;throw Error('EXPECTED_DYNAMIC_ERROR');";
        for _ in 0..2 {
            assert!(engine.eval_cached(name, throws).unwrap_err().to_string().contains("EXPECTED_DYNAMIC_ERROR"));
        }
        engine.eval("if(failedRuns!==2)throw Error('failure effects changed');").unwrap();
        assert!(cache().lock().unwrap().get(source_key(name, throws), name, throws).is_none());
    }

    #[test]
    fn rejected_bytecode_falls_back_to_source_and_preserves_promise_behavior() {
        let source = "globalThis.scriptRan=true;globalThis.promiseRan=false;Promise.resolve().then(()=>{promiseRan=true;});";
        let mut plain = JsRuntime::new(Default::default());
        plain.execute_script("<eval>", deno_core::FastString::Static(source)).unwrap();
        let plain_result = plain.execute_script("<probe>", deno_core::FastString::Static("promiseRan")).unwrap();
        let plain_promise = {
            let scope = &mut plain.handle_scope();
            let value = v8::Local::new(scope, plain_result);
            value.boolean_value(scope)
        };
        drop(plain);

        let mut cached = JsRuntime::new(Default::default());
        compile_and_run(&mut cached, "<eval>", source, Some(&[0; 128]), false).unwrap();
        let expected = format!("if(!scriptRan||promiseRan!=={plain_promise})throw Error('cached evaluation changed microtasks');");
        cached.execute_script("<probe>", deno_core::FastString::Owned(expected.into_boxed_str())).unwrap();
    }

    #[test]
    #[ignore = "manual repeated real street-script compilation benchmark"]
    fn benchmark_street_script_compilation_cache() {
        let root = std::env::var_os("LUNA_STREET_SOURCE_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../server_expo"));
        let original = std::fs::read_to_string(root.join("public/objetos/_obra.js"))
            .expect("set LUNA_STREET_SOURCE_DIR to the server_expo directory");
        let source = format!("(function(){{\n{original}\n}})();");
        let name = "https://street-benchmark.test/objetos/_obra.js";
        {
            let mut engine = Engine::new();
            engine.eval_cached(name, &source).unwrap();
        }
        let mut cached = Vec::new();
        let mut uncached = Vec::new();
        for iteration in 0..24 {
            for use_cache in if iteration % 2 == 0 { [false, true] } else { [true, false] } {
                let mut engine = Engine::new();
                let started = Instant::now();
                if use_cache {
                    engine.eval_cached(name, &source).unwrap();
                    cached.push(started.elapsed().as_secs_f64() * 1000.0);
                } else {
                    compile_and_run(&mut engine.rt, name, &source, None, false).unwrap();
                    uncached.push(started.elapsed().as_secs_f64() * 1000.0);
                }
                engine.eval("if(typeof Obra !== 'object')throw Error('street API missing');").unwrap();
            }
        }
        cached.sort_by(f64::total_cmp);
        uncached.sort_by(f64::total_cmp);
        let entry = cache().lock().unwrap().get(source_key(name, &source), name, &source).unwrap();
        eprintln!("street_compilation source_bytes={} uncached_median_ms={:.3} cached_median_ms={:.3} cache_entry_bytes={}",
            source.len(), uncached[12], cached[12], entry.size());
    }
}
