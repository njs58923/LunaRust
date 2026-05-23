# HTTP Cache & Content Security Policy - Implementation

## ✅ Implementación Completada en js_runtime

### 1. **HTTP Cache** (`debs/js_runtime/src/cache.rs`)

#### Estructuras Principales:

**CachedScript:**
```rust
pub struct CachedScript {
    pub code: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub expires: Option<SystemTime>,
    pub cached_at: SystemTime,
}
```

- Soporta Cache-Control con `max-age`
- Validación con `is_valid()` basada en `expires` o 1 hora por defecto
- Revalidación con ETags y Last-Modified

**ScriptCache (LRU):**
```rust
pub struct ScriptCache {
    entries: HashMap<String, CachedScript>,
    lru: VecDeque<String>,
    max_entries: usize,
}
```

- LRU eviction cuando se alcanza `max_entries`
- Métodos: `get()`, `insert()`, `get_for_revalidation()`, `clear()`, `stats()`
- Default: 100 scripts máximo

**FetchCache (LRU):**
```rust
pub struct FetchCache {
    entries: HashMap<String, CachedResponse>,
    lru: VecDeque<String>,
    max_entries: usize,
}
```

- Similar a ScriptCache pero para respuestas HTTP generales
- Default: 200 responses máximo
- Cache duration: 5 minutos por defecto si no hay Cache-Control

### 2. **Content Security Policy** (`debs/js_runtime/src/csp.rs`)

#### Estructura Principal:

**ContentSecurityPolicy:**
```rust
pub struct ContentSecurityPolicy {
    pub script_src: HashSet<String>,
    pub connect_src: HashSet<String>,
    pub allow_self_scripts: bool,
    pub allow_self_connect: bool,
    pub allow_unsafe_eval: bool,
}
```

**Métodos:**
- `from_header(header: &str)` - Parse CSP header (e.g., "script-src 'self' https://cdn.example.com")
- `allows_script(url, document_origin)` - Valida si un script puede cargarse
- `allows_connect(url, document_origin)` - Valida si un fetch puede realizarse
- `permissive()` - CSP que permite todo (default)
- `restrictive()` - CSP que bloquea todo por defecto

**CORS Validator:**
```rust
pub struct CorsValidator;
impl CorsValidator {
    pub fn validate_request(request_url, document_origin) -> CorsValidation;
    pub fn validate_response(request_origin, access_control_allow_origin) -> bool;
}
```

### 3. **Integration en Engine** (`debs/js_runtime/src/lib.rs`)

#### Nuevos Campos en Engine:

```rust
pub struct Engine {
    // ... existing fields ...

    // HTTP Cache & Security
    script_cache: Arc<Mutex<ScriptCache>>,
    fetch_cache: Arc<Mutex<FetchCache>>,
    csp: Arc<Mutex<ContentSecurityPolicy>>,
    document_origin: Arc<Mutex<String>>,
}
```

#### Nuevos Métodos Públicos:

**Document Origin:**
- `set_document_origin(origin: String)`
- `get_document_origin() -> String`

**CSP:**
- `set_csp(csp: ContentSecurityPolicy)`
- `get_csp() -> ContentSecurityPolicy`
- `csp_allows_script(url: &str) -> bool`
- `csp_allows_fetch(url: &str) -> bool`

**Script Cache:**
- `get_cached_script(url: &str) -> Option<CachedScript>`
- `get_script_for_revalidation(url: &str) -> Option<CachedScript>`
- `cache_script(url: String, script: CachedScript)`
- `get_cache_stats() -> CacheStats`

**Fetch Cache:**
- `get_cached_response(url: &str) -> Option<CachedResponse>`
- `cache_response(url: String, response: CachedResponse)`

**Utility:**
- `clear_caches()` - Clear both script and fetch caches

---

## 📋 Pendiente: Integración en luna.rs

Para completar la integración, se necesita:

### 1. **Configurar Document Origin**

En `reload_xml_system` o al inicio de cada navegación:

```rust
// Extract origin from URL
let origin = extract_origin_from_url(&current_url.0);
runtime.set_document_origin(origin);
```

### 2. **Configurar CSP** (Opcional)

Si el documento HSML incluye CSP en metadata:

```rust
if let Some(csp_header) = parse_csp_from_document() {
    let csp = ContentSecurityPolicy::from_header(&csp_header);
    runtime.set_csp(csp);
}
```

### 3. **Usar Script Cache en dom_sync_system**

Modificar la sección de descarga de scripts (línea ~1188):

```rust
"script" => {
    if let Some(script_comp) = scripts_storage.get(*node) {
        if let Some(ref src) = script_comp.src {
            if let Some(final_url) = resolve_remote_path(&current_url.0, src) {
                // TODO: Validate CSP first
                // if !runtime.csp_allows_script(&final_url) {
                //     log_panel.push_error(format!("CSP blocked script: {}", final_url));
                //     continue;
                // }

                // TODO: Check cache first
                // if let Some(cached) = runtime.get_cached_script(&final_url) {
                //     pending_scripts.0.push((final_url, cached.code));
                //     continue;
                // }

                // Download script
                match tokio_rt.0.block_on(async {
                    let resp = reqwest::get(&final_url).await?;

                    // Extract cache headers
                    let etag = resp.headers()
                        .get("etag")
                        .and_then(|v| v.to_str().ok())
                        .map(String::from);
                    let last_modified = resp.headers()
                        .get("last-modified")
                        .and_then(|v| v.to_str().ok())
                        .map(String::from);
                    let cache_control = resp.headers()
                        .get("cache-control")
                        .and_then(|v| v.to_str().ok());

                    let code = resp.text().await?;

                    Ok((code, etag, last_modified, cache_control.map(String::from)))
                }) {
                    Ok((code, etag, last_modified, cache_control)) => {
                        // TODO: Cache the script
                        // let cached_script = CachedScript::with_headers(
                        //     code.clone(),
                        //     etag,
                        //     last_modified,
                        //     cache_control.as_deref(),
                        // );
                        // runtime.cache_script(final_url.clone(), cached_script);

                        pending_scripts.0.push((final_url, code));
                    }
                    Err(e) => {
                        log_panel.push_error(format!("Error downloading script {}: {}", final_url, e));
                    }
                }
            }
        }
    }
    // ... rest of code
}
```

### 4. **Usar Fetch Cache en js_tick_system**

Modificar Block 7 (fetch requests, línea ~1986):

```rust
// Block 7: Process fetch requests
if !fetch_queue.is_empty() {
    // Check cache first
    let (cached_responses, uncached_requests) = {
        let mut cached = Vec::new();
        let mut uncached = Vec::new();

        for (request_id, url) in &fetch_queue {
            // TODO: Check CSP
            // if !runtime.csp_allows_fetch(url) {
            //     cached.push((request_id, Err("CSP blocked request".to_string())));
            //     continue;
            // }

            // TODO: Check cache
            // if let Some(cached_response) = runtime.get_cached_response(url) {
            //     cached.push((request_id, Ok(cached_response.body)));
            // } else {
            //     uncached.push((request_id, url));
            // }

            uncached.push((request_id.clone(), url.clone()));
        }
        (cached, uncached)
    };

    // Perform uncached fetches
    let fetch_results = {
        let Some(tokio_rt) = world.get_resource::<TokioRuntime>() else {
            return;
        };

        let mut results = cached_responses;

        for (request_id, url) in uncached_requests {
            let result = tokio_rt.0.block_on(async {
                match reqwest::get(&url).await {
                    Ok(resp) => {
                        // TODO: Extract cache headers
                        let text = resp.text().await?;

                        // TODO: Cache response
                        // let cached_response = CachedResponse::new(text.clone(), 200, HashMap::new());
                        // runtime.cache_response(url, cached_response);

                        Ok(text)
                    },
                    Err(e) => Err(format!("HTTP error: {}", e)),
                }
            });

            results.push((request_id, result));
        }

        results
    };

    // Rest of processing...
}
```

---

## 🔍 Testing

### Test Script Cache:

```rust
// In luna.rs or test file
let cache_stats = runtime.get_cache_stats();
log_panel.push_info(format!(
    "Script cache: {}/{} entries ({} valid)",
    cache_stats.total_entries,
    cache_stats.max_entries,
    cache_stats.valid_entries
));

// Clear cache if needed
runtime.clear_caches();
```

### Test CSP:

```rust
// Set up CSP
let csp = ContentSecurityPolicy::from_header("script-src 'self' https://cdn.example.com");
runtime.set_csp(csp);

// Validate scripts
if !runtime.csp_allows_script("https://evil.com/malware.js") {
    log_panel.push_error("CSP blocked malicious script");
}
```

### Test Cache Headers:

Create a test script at server_hsml with proper cache headers:

```javascript
// server_hsml/src/index.ts
app.get('/cached-script.js', (req, res) => {
  res.setHeader('Cache-Control', 'max-age=3600'); // 1 hour
  res.setHeader('ETag', '"v1.0.0"');
  res.send('console.log("Cached script loaded");');
});
```

---

## 📊 Benefits

1. **Performance**: Scripts and responses are cached, reducing network requests
2. **Bandwidth**: ETags and Last-Modified enable conditional requests (304 Not Modified)
3. **Security**: CSP prevents loading of unauthorized scripts
4. **CORS**: Validates cross-origin requests properly
5. **Memory Management**: LRU eviction prevents unbounded memory growth

---

## 🚀 Next Steps

1. ✅ Complete cache.rs (DONE)
2. ✅ Complete csp.rs (DONE)
3. ✅ Integrate into Engine (DONE)
4. ⏳ Integrate cache into luna.rs dom_sync_system (PENDING)
5. ⏳ Integrate cache into luna.rs js_tick_system (PENDING)
6. ⏳ Add CSP validation (PENDING)
7. ⏳ Add conditional HTTP requests with ETags (PENDING)
8. ⏳ Testing with real scripts (PENDING)

---

## 📝 Notes

- Cache is currently permissive by default - tighten CSP for production
- Default cache sizes are conservative (100 scripts, 200 responses) - tune based on usage
- ETags and Last-Modified require server support - ensure server_hsml sends these headers
- CORS validation is simplified - may need enhancement for complex scenarios
