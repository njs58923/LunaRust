use lazy_static::lazy_static;
use std::collections::HashMap;

pub struct VirtualRoutes {
    routes: HashMap<String, RouteHandler>,
}

enum RouteHandler {
    Static(&'static str),        // HSML/JS estáticos
    Dynamic(fn(&str) -> String), // Contenido generado (ej: cache stats)
}

lazy_static! {
    pub static ref VIRTUAL_ROUTES: VirtualRoutes = VirtualRoutes::init();
}

impl VirtualRoutes {
    fn init() -> Self {
        let mut routes = HashMap::new();

        // Registrar rutas estáticas
        routes.insert("root".to_string(), RouteHandler::Static(LUNA_ROOT));
        routes.insert("home".to_string(), RouteHandler::Static(LUNA_HOME));
        routes.insert("demos".to_string(), RouteHandler::Static(LUNA_DEMOS));
        routes.insert("settings".to_string(), RouteHandler::Static(LUNA_SETTINGS));
        routes.insert("about".to_string(), RouteHandler::Static(LUNA_ABOUT));
        routes.insert("error/404".to_string(), RouteHandler::Static(LUNA_404));

        // Registrar rutas dinámicas
        routes.insert(
            "cache-stats".to_string(),
            RouteHandler::Dynamic(generate_cache_stats),
        );

        // Registrar scripts internos
        routes.insert(
            "internal/home_navigation.js".to_string(),
            RouteHandler::Static(SCRIPT_HOME_NAV),
        );
        routes.insert(
            "internal/root_api.js".to_string(),
            RouteHandler::Static(SCRIPT_ROOT_API),
        );
        routes.insert(
            "internal/controller_toque.js".to_string(),
            RouteHandler::Static(SCRIPT_CONTROLLER_TOQUE),
        );

        // UX and Controller routes
        routes.insert("ux_desktop".to_string(), RouteHandler::Static(LUNA_UX_DESKTOP));
        routes.insert("ux_vr".to_string(), RouteHandler::Static(LUNA_UX_VR));
        routes.insert("controller_desktop".to_string(), RouteHandler::Static(LUNA_CONTROLLER_DESKTOP));
        routes.insert("controller_vr".to_string(), RouteHandler::Static(LUNA_CONTROLLER_VR));
        Self { routes }
    }

    pub fn resolve(&self, url: &str) -> Option<String> {
        let route_path = url.strip_prefix("luna://")?.trim_start_matches('/');

        println!(
            "[VirtualRoutes] Resolving: '{}' -> route_path: '{}'",
            url, route_path
        );

        if let Some(handler) = self.routes.get(route_path) {
            let content = match handler {
                RouteHandler::Static(content) => content.to_string(),
                RouteHandler::Dynamic(generator) => generator(route_path),
            };
            println!(
                "[VirtualRoutes] ✓ Found route, content length: {}",
                content.len()
            );
            return Some(content);
        }

        // No match - return 404
        println!("[VirtualRoutes] ✗ Route not found, returning 404");
        Some(LUNA_404.to_string())
    }

    pub fn is_virtual_url(url: &str) -> bool {
        url.starts_with("luna://")
    }
}

// ============================================================================
// DOCUMENTOS HSML ESTÁTICOS
// ============================================================================

const LUNA_ROOT: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Luna Root</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space id="luna_root" system-space="root" resources="root">
  </space>
</hsml>"##;

const LUNA_HOME: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Luna Home</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space resources="navigate_self">
    <text x="0" y="1.5" z="-2" value="Luna Browser - Home" size="0.3" />
    <text x="0" y="1.2" z="-2" value="Welcome to Luna 3D Browser" size="0.15" />

    <box x="-0.8" y="0.6" z="-2" sx="0.4" sy="0.4" sz="0.1" color="#4CAF50" id="btn_demos" />
    <text x="-0.8" y="0.6" z="-1.9" value="Demos" size="0.1" />

    <box x="0" y="0.6" z="-2" sx="0.4" sy="0.4" sz="0.1" color="#2196F3" id="btn_settings" />
    <text x="0" y="0.6" z="-1.9" value="Settings" size="0.1" />

    <box x="0.8" y="0.6" z="-2" sx="0.4" sy="0.4" sz="0.1" color="#FF9800" id="btn_about" />
    <text x="0.8" y="0.6" z="-1.9" value="About" size="0.1" />
    <script src="luna://internal/home_navigation.js" />
  </space>
</hsml>"##;

const LUNA_DEMOS: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Luna Demos</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space resources="navigate_self">
    <text x="0" y="1.5" z="-2" value="Luna Demos" size="0.3" />
    <text x="0" y="1.2" z="-2" value="Interactive 3D Demonstrations" size="0.15" />

    <box x="-1.2" y="0.6" z="-2" sx="0.4" sy="0.4" sz="0.1" color="#9C27B0" id="demo_cube" />
    <text x="-1.2" y="0.6" z="-1.9" value="Demo 1" size="0.1" />

    <box x="0" y="0.6" z="-2" sx="0.4" sy="0.4" sz="0.1" color="#00BCD4" id="demo_colors" />
    <text x="0" y="0.6" z="-1.9" value="Demo 2" size="0.1" />

    <box x="1.2" y="0.6" z="-2" sx="0.4" sy="0.4" sz="0.1" color="#FF5722" id="demo_buttons" />
    <text x="1.2" y="0.6" z="-1.9" value="Demo 3" size="0.1" />

    <box x="0" y="-0.2" z="-2" sx="0.6" sy="0.2" sz="0.05" color="#4CAF50" id="btn_home" />
    <text x="0" y="-0.2" z="-1.95" value="Back to Home" size="0.08" />

  <script>
    const btnHome = hiperspace.dimention.getElementById('btn_home');
    if (btnHome) btnHome.addEventListener('toque', () => { location.href = 'luna://home'; });

    // Demo placeholders - implement actual demo logic later
    const demos = ['demo_cube', 'demo_colors', 'demo_buttons'];
    demos.forEach(id => {
      const elem = hiperspace.dimention.getElementById(id);
      if (elem) elem.addEventListener('toque', () => {
        console.log('Demo clicked:', id, '- Implementation pending');
      });
    });

    console.log('[luna://demos] Page loaded');
  </script>
  </space>
</hsml>"##;

const LUNA_SETTINGS: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Luna Settings</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space resources="navigate_self">
  <text x="0" y="3.5" z="-5" value="Luna Settings" size="0.4" />
  <text x="0" y="3" z="-5" value="Browser Configuration" size="0.18" />

  <text x="-3" y="2.2" z="-5" value="Appearance" size="0.15" color="#4CAF50" />
  <text x="-3" y="1.8" z="-5" value="- Theme: Default" size="0.1" />
  <text x="-3" y="1.5" z="-5" value="- Font Size: Medium" size="0.1" />

  <text x="0" y="2.2" z="-5" value="Performance" size="0.15" color="#2196F3" />
  <text x="0" y="1.8" z="-5" value="- Render Quality: High" size="0.1" />
  <text x="0" y="1.5" z="-5" value="- Cache Enabled: Yes" size="0.1" />

  <text x="3" y="2.2" z="-5" value="Network" size="0.15" color="#FF9800" />
  <text x="3" y="1.8" z="-5" value="- Proxy: None" size="0.1" />
  <text x="3" y="1.5" z="-5" value="- Timeout: 30s" size="0.1" />

  <box x="-1.5" y="0" z="-4" sx="1.2" sy="0.3" sz="0.05" color="#9C27B0" id="btn_cache" />
  <text x="-1.5" y="0" z="-3.95" value="Cache Stats" size="0.1" />

  <box x="1.5" y="0" z="-4" sx="1.2" sy="0.3" sz="0.05" color="#4CAF50" id="btn_home" />
  <text x="1.5" y="0" z="-3.95" value="Back to Home" size="0.1" />

  <text x="0" y="-0.8" z="-5" value="Note: Settings UI is read-only" size="0.09" color="#999999" />

  <script>
    const btnHome = hiperspace.dimention.getElementById('btn_home');
    const btnCache = hiperspace.dimention.getElementById('btn_cache');

    if (btnHome) btnHome.addEventListener('toque', () => { location.href = 'luna://home'; });
    if (btnCache) btnCache.addEventListener('toque', () => { location.href = 'luna://cache-stats'; });

    console.log('[luna://settings] Settings page loaded');
  </script>
  </space>
</hsml>"##;

const LUNA_ABOUT: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Luna About</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space resources="navigate_self">
  <text x="0" y="3.5" z="-5" value="About Luna Browser" size="0.4" />
  <text x="0" y="2.9" z="-5" value="Spatial 3D Web Browser" size="0.18" />

  <text x="0" y="2.3" z="-5" value="Version: 0.1.0-alpha" size="0.14" />
  <text x="0" y="2" z="-5" value="Built with Bevy 0.14 + OpenXR" size="0.12" />

  <text x="0" y="1.4" z="-5" value="Features:" size="0.15" color="#4CAF50" />
  <text x="0" y="1.1" z="-5" value="- HSML (Spatial HTML) Parsing" size="0.1" />
  <text x="0" y="0.8" z="-5" value="- JavaScript Runtime (V8 via deno_core)" size="0.1" />
  <text x="0" y="0.5" z="-5" value="- Virtual Protocol Handler (luna://)" size="0.1" />
  <text x="0" y="0.2" z="-5" value="- HTTP Caching System" size="0.1" />
  <text x="0" y="-0.1" z="-5" value="- DevTools with Console" size="0.1" />

  <text x="0" y="-0.7" z="-5" value="Project: Luna Browser by Sam" size="0.11" color="#2196F3" />

  <box x="0" y="-1.5" z="-4" sx="1.2" sy="0.3" sz="0.05" color="#4CAF50" id="btn_home" />
  <text x="0" y="-1.5" z="-3.95" value="Back to Home" size="0.1" />

  <script>
    const btnHome = hiperspace.dimention.getElementById('btn_home');
    if (btnHome) btnHome.addEventListener('toque', () => { location.href = 'luna://home'; });

    console.log('[luna://about] About page loaded');
  </script>
  </space>
</hsml>"##;

const LUNA_404: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>404 Not Found</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space resources="navigate_self">
  <text x="0" y="3" z="-5" value="404 - Page Not Found" size="0.35" color="#F44336" />
  <text x="0" y="2.5" z="-5" value="The requested luna:// page does not exist" size="0.15" />

  <text x="0" y="1.8" z="-5" value="Available Pages:" size="0.16" color="#4CAF50" />
  <text x="0" y="1.4" z="-5" value="- luna://home" size="0.11" />
  <text x="0" y="1.1" z="-5" value="- luna://demos" size="0.11" />
  <text x="0" y="0.8" z="-5" value="- luna://settings" size="0.11" />
  <text x="0" y="0.5" z="-5" value="- luna://about" size="0.11" />
  <text x="0" y="0.2" z="-5" value="- luna://cache-stats" size="0.11" />

  <box x="0" y="-1.5" z="-4" sx="1.2" sy="0.3" sz="0.05" color="#4CAF50" id="btn_home" />
  <text x="0" y="-1.5" z="-3.95" value="Go Home" size="0.1" />

  <script>
    const btn = hiperspace.dimention.getElementById('btn_home');
    if (btn) btn.addEventListener('toque', () => { location.href = 'luna://home'; });
    console.log('[luna://404] Error page loaded');
  </script>
  </space>
</hsml>"##;

// ============================================================================
// SCRIPTS INTERNOS
// ============================================================================

const SCRIPT_HOME_NAV: &str = r##"
const btnDemos = hiperspace.dimention.getElementById('btn_demos');
const btnSettings = hiperspace.dimention.getElementById('btn_settings');
const btnAbout = hiperspace.dimention.getElementById('btn_about');

if (btnDemos) btnDemos.addEventListener('toque', () => { location.href = 'luna://demos'; });
if (btnSettings) btnSettings.addEventListener('toque', () => { location.href = 'luna://settings'; });
if (btnAbout) btnAbout.addEventListener('toque', () => { location.href = 'luna://about'; });

console.log('[luna://home] Navigation ready');
"##;

const SCRIPT_ROOT_API: &str = r##"
(function (global) {
  const root = global.hiperspace && global.hiperspace.dimention;
  if (!root) {
    console.error('[luna://root] Missing root space');
    return;
  }

  const dimension = global.dimension || (global.dimension = {});
  const registry = new Map();
  let nextPublicId = 1;

  function normalizeTag(tagName) {
    return String(tagName || '').toLowerCase();
  }

  function isPending(space) {
    return typeof space.nodeId === 'number' && space.nodeId < 0;
  }

  function isDirectRootChild(space) {
    if (isPending(space)) {
      return true;
    }
    const parent = space.parent;
    return !!parent && parent.nodeId === root.nodeId;
  }

  function cloneVec3(vec, fallback) {
    if (!vec || typeof vec !== 'object') return { ...fallback };
    return {
      x: Number(vec.x ?? fallback.x),
      y: Number(vec.y ?? fallback.y),
      z: Number(vec.z ?? fallback.z),
    };
  }

  function findPrimaryInclude(space) {
    for (const child of space.children) {
      if (normalizeTag(child.tagName) === 'include') {
        return child;
      }
    }
    return null;
  }

  function registerSpace(space) {
    for (const [publicId, entry] of registry) {
      if (entry.space.nodeId === space.nodeId) {
        if (!entry.include) {
          entry.include = findPrimaryInclude(space);
        }
        return publicId;
      }
    }

    const publicId = nextPublicId++;
    registry.set(publicId, {
      space,
      include: findPrimaryInclude(space),
    });
    return publicId;
  }

  function discoverDirectSpaces() {
    for (const child of root.children) {
      if (normalizeTag(child.tagName) === 'space') {
        registerSpace(child);
      }
    }
  }

  function cleanupRegistry() {
    for (const [publicId, entry] of [...registry.entries()]) {
      if (!isDirectRootChild(entry.space)) {
        registry.delete(publicId);
      }
    }
  }

  function getMountedEntry(publicId) {
    discoverDirectSpaces();
    cleanupRegistry();
    return registry.get(publicId) || null;
  }

  function ensureInclude(entry) {
    if (entry.include) return entry.include;
    const include = root.createElement('include');
    entry.space.appendChild(include);
    entry.include = include;
    return include;
  }

  function applySpaceOptions(entry, options) {
    const opts = options || {};
    const space = entry.space;

    if (opts.visible != null) {
      space.setAttribute('visible', opts.visible ? 'true' : 'false');
    }

    if (opts.position) {
      space.position = cloneVec3(opts.position, { x: 0, y: 0, z: 0 });
    }

    if (opts.rotation) {
      space.rotation = cloneVec3(opts.rotation, { x: 0, y: 0, z: 0 });
    }

    if (opts.scale != null) {
      if (typeof opts.scale === 'number') {
        space.scale = Number(opts.scale);
      } else {
        space.scale = cloneVec3(opts.scale, { x: 1, y: 1, z: 1 });
      }
    }

    if (opts.title != null) {
      space.setAttribute('title', String(opts.title));
    }

    if (opts.url != null) {
      const include = ensureInclude(entry);
      include.setAttribute('src', String(opts.url));
    }
  }

  function describeSpace(publicId, entry) {
    const include = entry.include || findPrimaryInclude(entry.space);
    entry.include = include;
    return {
      id: publicId,
      nodeId: entry.space.nodeId,
      url: include ? include.getAttribute('src') : '',
      visible: entry.space.getAttribute('visible') !== 'false',
      loaded: include ? include.children.length > 0 : entry.space.children.length > 0,
      position: cloneVec3(entry.space.position, { x: 0, y: 0, z: 0 }),
      rotation: cloneVec3(entry.space.rotation, { x: 0, y: 0, z: 0 }),
      scale: cloneVec3(entry.space.scale, { x: 1, y: 1, z: 1 }),
      title: entry.space.getAttribute('title') || '',
    };
  }

  dimension.luna = {
    _uxSpaceId: null,
    _currentMode: null,

    mountSpace(url, options = {}) {
      discoverDirectSpaces();
      cleanupRegistry();

      const space = root.createElement('space');
      const publicId = registerSpace(space);
      const entry = registry.get(publicId);
      if (!entry) return -1;

      space.setAttribute('visible', options.visible === false ? 'false' : 'true');
      space.setAttribute('managed-by', 'dimension.luna');

      if (Array.isArray(options.grants) && options.grants.length) {
        space.setAttribute('resources', options.grants.join(','));
      }

      root.appendChild(space);
      console.log('[root] mountSpace url=', url, 'grants=', Array.isArray(options.grants) ? options.grants.join(',') : '(none)');
      const include = ensureInclude(entry);
      if (Array.isArray(options.grants)) {
        include.setAttribute('resources', options.grants.join(','));
      }
      applySpaceOptions(entry, { ...options, url });
      return publicId;
    },

    updateSpace(id, options = {}) {
      const entry = getMountedEntry(id);
      if (!entry) return false;
      applySpaceOptions(entry, options);
      return true;
    },

    setSpaceVisible(id, visible) {
      return this.updateSpace(id, { visible });
    },

    unmountSpace(id) {
      const entry = getMountedEntry(id);
      if (!entry) return false;
      console.log('[root] unmountSpace id=', id);
      registry.delete(id);
      entry.space.remove();
      return true;
    },

    listMountedSpaces() {
      discoverDirectSpaces();
      cleanupRegistry();
      const mounted = [];
      for (const [publicId, entry] of registry) {
        if (!isDirectRootChild(entry.space)) continue;
        mounted.push(describeSpace(publicId, entry));
      }
      return mounted;
    },

    regrantMountedSpaces(mode) {
      const grants = ['navigate_self'];

      discoverDirectSpaces();
      cleanupRegistry();

      for (const [publicId, entry] of registry) {
        if (publicId === this._uxSpaceId) continue;
        if (!isDirectRootChild(entry.space)) continue;
        const include = ensureInclude(entry);
        include.setAttribute('resources', grants.join(','));
      }

      return true;
    },

    switchMode(mode) {
      console.log('[root] switchMode ->', mode);
      if (mode !== 'desktop' && mode !== 'vr') {
        console.error('[root] Invalid mode:', mode);
        return false;
      }
      if (mode === this._currentMode) return true;

      if (this._uxSpaceId != null) {
        this.unmountSpace(this._uxSpaceId);
        this._uxSpaceId = null;
      }

      const uxUrl = mode === 'vr' ? 'luna://ux_vr' : 'luna://ux_desktop';
      const grants = mode === 'vr'
        ? ['navigate_self', 'vr_locomotion']
        : ['navigate_self', 'desktop_camera_control'];
      this._uxSpaceId = this.mountSpace(uxUrl, { visible: true, grants });
      this._currentMode = mode;
      console.log('[root] ux mounted id=', this._uxSpaceId, 'url=', uxUrl);
      return true;
    },
  };

  // Auto-mount desktop UX on startup
  dimension.luna.switchMode('desktop');

  console.log('[luna://root] dimension.luna ready');
})(globalThis);
"##;

// ============================================================================
// GENERADORES DE CONTENIDO DINÁMICO
// ============================================================================

fn generate_cache_stats(_path: &str) -> String {
    r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Cache Statistics</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space resources="navigate_self">
  <text x="0" y="3.5" z="-5" value="HTTP Cache Statistics" size="0.35" />
  <text x="0" y="2.9" z="-5" value="Real-time cache monitoring" size="0.16" />

  <text x="0" y="2.3" z="-5" value="Cache Entries: [Placeholder]" size="0.13" />
  <text x="0" y="2" z="-5" value="Total Size: [Placeholder]" size="0.13" />
  <text x="0" y="1.7" z="-5" value="Hit Rate: [Placeholder]" size="0.13" />

  <text x="0" y="1" z="-5" value="Note: Dynamic stats require runtime integration" size="0.11" color="#FFC107" />
  <text x="0" y="0.7" z="-5" value="Future: Pass cache resource to generate_cache_stats()" size="0.09" />

  <box x="0" y="-0.5" z="-4" sx="1.2" sy="0.3" sz="0.05" color="#4CAF50" id="btn_back" />
  <text x="0" y="-0.5" z="-3.95" value="Back to Settings" size="0.1" />

  <script>
    const btn = hiperspace.dimention.getElementById('btn_back');
    if (btn) btn.addEventListener('toque', () => { location.href = 'luna://settings'; });
    console.log('[luna://cache-stats] Stats page loaded (placeholder)');
  </script>
  </space>
</hsml>"##.to_string()
}

// ============================================================================
// UX DOCUMENTS
// ============================================================================

const LUNA_UX_DESKTOP: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>UX Desktop</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space id="ux_desktop" resources="desktop_camera_control">
    <text x="0" y="2.0" z="-3" value="UX Desktop Mounted" size="0.15" color="#00FF00" />
    <script>
      console.log('[ux_desktop] Desktop UX loaded');
    </script>
  </space>
</hsml>"##;

const LUNA_UX_VR: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>UX VR</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space id="ux_vr" resources="vr_locomotion">
    <text x="0" y="2.0" z="-3" value="UX VR Mounted" size="0.15" color="#00FF00" />
    <script>
      console.log('[ux_vr] VR UX loaded');
    </script>
  </space>
</hsml>"##;

// ============================================================================
// CONTROLLER DOCUMENTS
// ============================================================================

const LUNA_CONTROLLER_DESKTOP: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Controller Desktop</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space id="controller_desktop">
    <script>
      Deno.core.ops.op_register_controller('desktop');

      function pollTouch() {
        var events = Deno.core.ops.op_poll_touch_events();
        for (var i = 0; i &lt; events.length; i++) {
          var evt = events[i];
          var root = globalThis.hiperspace &amp;&amp; globalThis.hiperspace.dimention;
          if (!root) continue;
          function findNode(el, targetId) {
            if (el.nodeId === targetId) return el;
            var ch = el.children;
            for (var j = 0; j &lt; ch.length; j++) {
              var found = findNode(ch[j], targetId);
              if (found) return found;
            }
            return null;
          }
          var target = findNode(root, evt.nodeId);
          if (target) {
            target.dispatchEvent({ type: 'touch', nodeId: evt.nodeId, x: evt.x, y: evt.y, z: evt.z });
            target.dispatchEvent({ type: 'click' });
          }
        }
        requestAnimationFrame(pollTouch);
      }
      requestAnimationFrame(pollTouch);

      console.log('[controller_desktop] Desktop controller active');
    </script>
  </space>
</hsml>"##;

const LUNA_CONTROLLER_VR: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Controller VR</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space id="controller_vr">
    <script>
      Deno.core.ops.op_register_controller('vr');

      function pollTouch() {
        var events = Deno.core.ops.op_poll_touch_events();
        for (var i = 0; i &lt; events.length; i++) {
          var evt = events[i];
          var root = globalThis.hiperspace &amp;&amp; globalThis.hiperspace.dimention;
          if (!root) continue;
          function findNode(el, targetId) {
            if (el.nodeId === targetId) return el;
            var ch = el.children;
            for (var j = 0; j &lt; ch.length; j++) {
              var found = findNode(ch[j], targetId);
              if (found) return found;
            }
            return null;
          }
          var target = findNode(root, evt.nodeId);
          if (target) {
            target.dispatchEvent({ type: 'touch', nodeId: evt.nodeId, x: evt.x, y: evt.y, z: evt.z });
            target.dispatchEvent({ type: 'click' });
          }
        }
        requestAnimationFrame(pollTouch);
      }
      requestAnimationFrame(pollTouch);

      console.log('[controller_vr] VR controller active');
    </script>
  </space>
</hsml>"##;

const SCRIPT_CONTROLLER_TOQUE: &str = r##"
(function () {
  function findNode(el, targetId) {
    if (el.nodeId === targetId) return el;
    var ch = el.children;
    for (var i = 0; i < ch.length; i++) {
      var found = findNode(ch[i], targetId);
      if (found) return found;
    }
    return null;
  }

  function pollToqueRaw() {
    // compat temporal con el runtime actual
    var events = Deno.core.ops.op_poll_touch_events();
    var root = globalThis.hiperspace && globalThis.hiperspace.dimention;
    if (root) {
      for (var i = 0; i < events.length; i++) {
        var evt = events[i];
        var target = findNode(root, evt.nodeId);
        if (target) {
          target.dispatchEvent({ type: 'toque', nodeId: evt.nodeId, x: evt.x, y: evt.y, z: evt.z });
        }
      }
    }
    requestAnimationFrame(pollToqueRaw);
  }

  requestAnimationFrame(pollToqueRaw);
  console.log('[controller_toque] active');
})();
"##;

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // is_virtual_url
    #[test]
    fn virtual_url_detected() {
        assert!(VirtualRoutes::is_virtual_url("luna://home"));
        assert!(VirtualRoutes::is_virtual_url("luna://demos"));
        assert!(VirtualRoutes::is_virtual_url(
            "luna://internal/home_navigation.js"
        ));
    }

    #[test]
    fn non_virtual_url_rejected() {
        assert!(!VirtualRoutes::is_virtual_url("http://example.com"));
        assert!(!VirtualRoutes::is_virtual_url("https://example.com"));
        assert!(!VirtualRoutes::is_virtual_url("/relative/path.hsml"));
        assert!(!VirtualRoutes::is_virtual_url(""));
    }

    // resolve — known routes return HSML content
    #[test]
    fn resolve_home_returns_hsml() {
        let content = VIRTUAL_ROUTES.resolve("luna://home").unwrap();
        assert!(content.contains("<hsml>"));
        assert!(content.contains("Luna Home"));
    }

    #[test]
    fn resolve_root_returns_hsml() {
        let content = VIRTUAL_ROUTES.resolve("luna://root").unwrap();
        assert!(content.contains("<hsml>"));
        assert!(content.contains("Luna Root"));
    }

    #[test]
    fn resolve_demos_returns_hsml() {
        let content = VIRTUAL_ROUTES.resolve("luna://demos").unwrap();
        assert!(content.contains("<hsml>"));
    }

    #[test]
    fn resolve_settings_returns_hsml() {
        let content = VIRTUAL_ROUTES.resolve("luna://settings").unwrap();
        assert!(content.contains("<hsml>"));
    }

    #[test]
    fn resolve_about_returns_hsml() {
        let content = VIRTUAL_ROUTES.resolve("luna://about").unwrap();
        assert!(content.contains("<hsml>"));
    }

    #[test]
    fn resolve_internal_script_returns_js() {
        let content = VIRTUAL_ROUTES
            .resolve("luna://internal/home_navigation.js")
            .unwrap();
        assert!(content.contains("luna://demos"));
    }

    #[test]
    fn resolve_internal_root_api_returns_js() {
        let content = VIRTUAL_ROUTES
            .resolve("luna://internal/root_api.js")
            .unwrap();
        assert!(content.contains("dimension.luna"));
        assert!(content.contains("mountSpace"));
    }

    #[test]
    fn resolve_unknown_route_returns_404() {
        let content = VIRTUAL_ROUTES.resolve("luna://does-not-exist").unwrap();
        assert!(content.contains("404"));
    }

    #[test]
    fn resolve_non_virtual_returns_none() {
        assert!(VIRTUAL_ROUTES.resolve("http://example.com").is_none());
        assert!(VIRTUAL_ROUTES.resolve("not-a-luna-url").is_none());
    }

    #[test]
    fn resolve_cache_stats_returns_hsml() {
        let content = VIRTUAL_ROUTES.resolve("luna://cache-stats").unwrap();
        assert!(content.contains("<hsml>"));
    }

    #[test]
    fn resolve_ux_desktop_returns_hsml() {
        let content = VIRTUAL_ROUTES.resolve("luna://ux_desktop").unwrap();
        assert!(content.contains("<hsml>"));
        assert!(content.contains("ux_desktop"));
    }

    #[test]
    fn resolve_ux_vr_returns_hsml() {
        let content = VIRTUAL_ROUTES.resolve("luna://ux_vr").unwrap();
        assert!(content.contains("<hsml>"));
        assert!(content.contains("ux_vr"));
    }

    #[test]
    fn resolve_controller_desktop_returns_hsml() {
        let content = VIRTUAL_ROUTES.resolve("luna://controller_desktop").unwrap();
        assert!(content.contains("<hsml>"));
        assert!(content.contains("controller_desktop"));
    }

    #[test]
    fn resolve_controller_vr_returns_hsml() {
        let content = VIRTUAL_ROUTES.resolve("luna://controller_vr").unwrap();
        assert!(content.contains("<hsml>"));
        assert!(content.contains("controller_vr"));
    }

    #[test]
    fn root_api_has_switch_mode() {
        let content = VIRTUAL_ROUTES
            .resolve("luna://internal/root_api.js")
            .unwrap();
        assert!(content.contains("switchMode"));
        assert!(content.contains("luna://ux_desktop"));
        assert!(content.contains("luna://ux_vr"));
    }
}
