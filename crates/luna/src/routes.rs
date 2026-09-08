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
        routes.insert("environment".to_string(), RouteHandler::Static(include_str!("web/environment.hsml")));
        routes.insert("demos".to_string(), RouteHandler::Static(LUNA_DEMOS));
        routes.insert("scale_demo".to_string(), RouteHandler::Static(LUNA_SCALE_DEMO));
        routes.insert("mesh_demo".to_string(), RouteHandler::Static(include_str!("web/mesh_demo.hsml")));
        routes.insert("fire_demo".to_string(), RouteHandler::Static(LUNA_FIRE_DEMO));
        routes.insert("target_demo".to_string(), RouteHandler::Static(LUNA_TARGET_DEMO));
        routes.insert("range_demo".to_string(), RouteHandler::Static(LUNA_RANGE_DEMO));
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
            "internal/tabs_api.js".to_string(),
            RouteHandler::Static(SCRIPT_TABS_API),
        );
        routes.insert(
            "internal/viewer_pose_api.js".to_string(),
            RouteHandler::Static(SCRIPT_VIEWER_POSE_API),
        );
        routes.insert(
            "internal/embedded_api.js".to_string(),
            RouteHandler::Static(SCRIPT_EMBEDDED_API),
        );

        // UX routes
        routes.insert("ux_desktop".to_string(), RouteHandler::Static(LUNA_UX_DESKTOP));
        routes.insert("ux_vr".to_string(), RouteHandler::Static(LUNA_UX_VR));

        // Apps embedded
        routes.insert("demo_embedded".to_string(), RouteHandler::Static(LUNA_APP_DEMO_EMBEDDED));

        // Compatibility aliases: MCP activation now lives in Settings.
        routes.insert("agent".to_string(), RouteHandler::Static(LUNA_SETTINGS));
        routes.insert("agent_app".to_string(), RouteHandler::Static(LUNA_SETTINGS));
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

const LUNA_HOME: &str = include_str!("web/home.hsml");

const LUNA_DEMOS: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Luna Demos Menu</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space resources="navigate_self">


    <group y="1.1">
    <plane x="0" y="0.6" z="-3.15" sx="3.0" sy="2.6" sz="1" color="#0D1B2A" id="menu_panel" />

    <text x="0" y="1.65" z="-3.0" value="Demos" size="0.26" color="#64B5F6" />
    <text x="0" y="1.32" z="-3.0" value="Pick a demo to launch" size="0.10" color="#90A4AE" />

    <!-- Demo buttons -->
    <box x="0" y="0.95" z="-3.0" sx="2.20" sy="0.26" sz="0.05" color="#7E57C2" id="btn_scale_demo" touchable="true"/>
    <text x="0" y="0.95" z="-2.94" value="Scale Demo  ·  spawn / animate / stress" size="0.085" color="#FFFFFF" />

    <box x="0" y="0.62" z="-3.0" sx="2.20" sy="0.26" sz="0.05" color="#EF5350" id="btn_fire_demo" touchable="true"/>
    <text x="0" y="0.62" z="-2.94" value="Fire Demo  ·  free shooting" size="0.085" color="#FFFFFF" />

    <box x="0" y="0.29" z="-3.0" sx="2.20" sy="0.26" sz="0.05" color="#FF1744" id="btn_target_demo" touchable="true"/>
    <text x="0" y="0.29" z="-2.94" value="Target Demo  ·  hit moving targets" size="0.085" color="#FFFFFF" />

    <box x="0" y="-0.04" z="-3.0" sx="2.20" sy="0.26" sz="0.05" color="#D50000" id="btn_range_demo" touchable="true"/>
    <text x="0" y="-0.04" z="-2.94" value="Range Demo  ·  formal levels" size="0.085" color="#FFFFFF" />

    <!-- Bottom row: navigation -->
    <box x="-0.78" y="-0.45" z="-3.0" sx="0.62" sy="0.22" sz="0.05" color="#4CAF50" id="btn_home" touchable="true"/>
    <text x="-0.78" y="-0.45" z="-2.94" value="HOME" size="0.085" color="#FFFFFF" />

    <box x="0.00" y="-0.45" z="-3.0" sx="0.62" sy="0.22" sz="0.05" color="#FF9800" id="btn_about" touchable="true"/>
    <text x="0.00" y="-0.45" z="-2.94" value="ABOUT" size="0.085" color="#FFFFFF" />

    <box x="0.78" y="-0.45" z="-3.0" sx="0.62" sy="0.22" sz="0.05" color="#9C27B0" id="btn_settings" touchable="true"/>
    <text x="0.78" y="-0.45" z="-2.94" value="SETTINGS" size="0.085" color="#FFFFFF" />

    <text x="0" y="-0.78" z="-3.0" value="" size="0.075" id="menu_status" color="#B0BEC5" />

    </group>

    <script>
      const root = hiperspace.dimention;
      const byId = (id) => root.getElementById(id);
      const setStatus = (msg) => {
        const el = byId('menu_status');
        if (el) el.setAttribute('value', msg || '');
      };
      const goto = (url) => () => { setStatus('Loading ' + url + '…'); location.href = url; };

      const links = {
        btn_scale_demo:  'luna://scale_demo',
        btn_fire_demo:   'luna://fire_demo',
        btn_target_demo: 'luna://target_demo',
        btn_range_demo:  'luna://range_demo',
        btn_home:        'luna://home',
        btn_about:       'luna://about',
        btn_settings:    'luna://settings',
      };
      Object.keys(links).forEach((id) => {
        const el = byId(id);
        if (el) el.addEventListener('toque', goto(links[id]));
      });

      console.log('[luna://demos] Menu ready');
    </script>
  </space>
</hsml>"##;

const LUNA_SCALE_DEMO: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Luna Scale Demo</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space resources="navigate_self">

    <!-- ═══════════════════════════════════════════════════════════════════════
         ENVIRONMENT
         ═══════════════════════════════════════════════════════════════════════ -->

    <!-- Ground extending 80m radius -->

    <!-- ═══════════════════════════════════════════════════════════════════════
         SCALE RING — 8 anchors, each at a different distance and direction.
         Scale rule: size = distance × 0.1  (4 m → 0.4 m  …  35 m → 3.5 m)
         All appear roughly the same angular size from the origin.
         ═══════════════════════════════════════════════════════════════════════ -->

    <group id="demo_ground">
    <!--  4 m  straight ahead  — tiny orange cube (0.4 m) -->
    <box x="0"      y="0.20" z="-4"     sx="0.40" sy="0.40" sz="0.40" color="#FF6F00" id="scale_4m" />
    <text x="0"      y="0.50" z="-4"     value="4 m · 0.4 m" size="0.050" color="#FFD54F" />

    <!--  6 m  front-right  45°  — yellow-green (0.6 m) -->
    <box x="4.24"  y="0.30" z="-4.24"  sx="0.60" sy="0.60" sz="0.60" color="#AEEA00" id="scale_6m" />
    <text x="4.24"  y="0.75" z="-4.24"  value="6 m · 0.6 m" size="0.075" color="#F9A825" />

    <!--  9 m  right  90°  — green (0.9 m) -->
    <box x="9"     y="0.45" z="0"      sx="0.90" sy="0.90" sz="0.90" color="#00C853" id="scale_9m" />
    <text x="9"     y="1.05" z="0"      value="9 m · 0.9 m" size="0.113" color="#B9F6CA" />

    <!-- 12 m  back-right  135°  — cyan (1.2 m) -->
    <box x="8.49"  y="0.60" z="8.49"   sx="1.20" sy="1.20" sz="1.20" color="#00BCD4" id="scale_12m" />
    <text x="8.49"  y="1.40" z="8.49"   value="12 m · 1.2 m" size="0.150" color="#B2EBF2" />

    <!-- 16 m  directly behind  180°  — blue (1.6 m) -->
    <box x="0"     y="0.80" z="16"     sx="1.60" sy="1.60" sz="1.60" color="#2979FF" id="scale_16m" />
    <text x="0"     y="1.75" z="16"     value="16 m · 1.6 m" size="0.200" color="#82B1FF" />

    <!-- 20 m  back-left  225°  — purple (2.0 m) -->
    <box x="-14.14" y="1.00" z="14.14" sx="2.00" sy="2.00" sz="2.00" color="#AA00FF" id="scale_20m" />
    <text x="-14.14" y="2.15" z="14.14" value="20 m · 2.0 m" size="0.250" color="#E040FB" />

    <!-- 26 m  left  270°  — pink (2.6 m) -->
    <box x="-26"   y="1.30" z="0"      sx="2.60" sy="2.60" sz="2.60" color="#F50057" id="scale_26m" />
    <text x="-26"   y="2.75" z="0"      value="26 m · 2.6 m" size="0.325" color="#FF80AB" />

    <!-- 35 m  front-left  315°  — red-orange (3.5 m — imposing!) -->
    <box x="-24.75" y="1.75" z="-24.75" sx="3.50" sy="3.50" sz="3.50" color="#DD2C00" id="scale_35m" />
    <text x="-24.75" y="3.55" z="-24.75" value="35 m · 3.5 m" size="0.438" color="#FFAB40" />

    </group>
    <!-- ═══════════════════════════════════════════════════════════════════════
         CONTROL PANEL  (at z = -3.1, eye level)
         ═══════════════════════════════════════════════════════════════════════ -->

    <group id="demo_controls" y="1.4" sx="0.75" sy="0.65">
    <plane x="0" y="0.20" z="-3.15" sx="5.10" sy="3.40" sz="1" color="#0D1B2A" id="demo_panel" />

    <text x="0" y="1.68" z="-3.0" value="Luna Scale Demo" size="0.22" color="#64B5F6" />
    <text x="0" y="1.38" z="-3.0" value="size = distance × 0.1  (all same angular size)" size="0.088" color="#546E7A" />

    <!-- Column headers -->
    <text x="-1.70" y="1.07" z="-3.0" value="Spawn" size="0.10" color="#8BC34A" />
    <text x="0.00"  y="1.07" z="-3.0" value="Modify" size="0.10" color="#03A9F4" />
    <text x="1.70"  y="1.07" z="-3.0" value="Inspect" size="0.10" color="#FFB74D" />

    <!-- ── Spawn column ── -->
    <box x="-1.70" y="0.74" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#4CAF50" id="demo_spawn_box" />
    <text x="-1.70" y="0.74" z="-2.94" value="Spawn Box" size="0.07" />

    <box x="-1.70" y="0.45" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#8BC34A" id="demo_spawn_sphere" />
    <text x="-1.70" y="0.45" z="-2.94" value="Spawn Sphere" size="0.07" />

    <box x="-1.70" y="0.16" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#009688" id="demo_spawn_cylinder" />
    <text x="-1.70" y="0.16" z="-2.94" value="Spawn Cylinder" size="0.07" />

    <box x="-1.70" y="-0.13" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#607D8B" id="demo_spawn_plane" />
    <text x="-1.70" y="-0.13" z="-2.94" value="Spawn Plane" size="0.07" />

    <box x="-1.70" y="-0.42" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#7E57C2" id="demo_stress" />
    <text x="-1.70" y="-0.42" z="-2.94" value="Stress ×25" size="0.07" />

    <!-- ── Modify column ── -->
    <box x="0.00" y="0.74" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#03A9F4" id="demo_recolor" />
    <text x="0.00" y="0.74" z="-2.94" value="Recolor" size="0.07" />

    <box x="0.00" y="0.45" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#00BCD4" id="demo_toggle_anim" />
    <text x="0.00" y="0.45" z="-2.94" value="Toggle Anim" size="0.07" />

    <box x="0.00" y="0.16" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#2196F3" id="demo_move_banner" />
    <text x="0.00" y="0.16" z="-2.94" value="Move Panel" size="0.07" />

    <box x="0.00" y="-0.13" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#1976D2" id="demo_toggle_floor" />
    <text x="0.00" y="-0.13" z="-2.94" value="Toggle Guides" size="0.07" />

    <box x="0.00" y="-0.42" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#E53935" id="demo_clear" />
    <text x="0.00" y="-0.42" z="-2.94" value="Clear All" size="0.07" />

    <!-- ── Inspect column (read-only / diagnostic actions) ── -->
    <box x="1.70" y="0.74" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#795548" id="demo_query" />
    <text x="1.70" y="0.74" z="-2.94" value="Query Test" size="0.07" />

    <box x="1.70" y="0.45" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#546E7A" id="demo_wave" />
    <text x="1.70" y="0.45" z="-2.94" value="Ring Arrange" size="0.07" />

    <box x="1.70" y="0.16" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#455A64" id="demo_direct_mode" />
    <text x="1.70" y="0.16" z="-2.94" value="Direct Only" size="0.06" />

    <!-- Status bar -->
    <text x="0" y="-0.70" z="-2.96" value="Status: ready" size="0.085" id="demo_status" color="#FFFFFF" />
    <text x="0" y="-0.88" z="-2.96" value="Dynamic nodes: 0" size="0.075" id="demo_count" color="#B0BEC5" />
    <text x="0" y="-1.04" z="-2.96" value="Animation: off" size="0.075" id="demo_anim" color="#B0BEC5" />
    <text x="0" y="-1.20" z="-2.96" value="Objects scale 1:10 of distance — spawns appear around you" size="0.063" id="demo_hint" color="#546E7A" />

    <!-- Bottom navigation row -->
    <box x="0.00" y="-1.42" z="-3.0" sx="0.62" sy="0.22" sz="0.05" color="#7E57C2" id="btn_demos" touchable="true"/>
    <text x="0.00" y="-1.42" z="-2.94" value="DEMOS" size="0.085" color="#FFFFFF" />

    </group>

    <script>
    const root = hiperspace.dimention;
    const __tmpRot = { x: 0, y: 0, z: 0 };
    const __tmpPos = { x: 0, y: 0, z: 0 };

    // Each entry: { el, bx, by, bz, off }
    const dynamicNodes = [];
    let nextId = 1;
    let animating = false;
    let bannerMoved = false;
    let floorVisible = true;

    const transformMode = 'direct';

    // FPS meter (script-side, approximate)
    let fpsFrames = 0;
    let fpsLastTs = 0;
    let fpsValue = 0;

    // ── helpers ──────────────────────────────────────────────────────────────

    // These controls live for the lifetime of this document. Avoid walking
    // thousands of dynamic children for every status/counter update.
    const controlsById = new Map();
    function byId(id) {
      let el = controlsById.get(id);
      if (!el) {
        el = root.getElementById(id);
        if (el) controlsById.set(id, el);
      }
      return el;
    }

    function setText(id, val) {
      const el = byId(id);
      if (el) el.setAttribute('value', val);
    }

    function setStatus(msg) {
      setText('demo_status', 'Status: ' + msg);
      console.log('[luna://demos]', msg);
    }

    function updateCounters() {
      setText('demo_count', 'Dynamic nodes: ' + dynamicNodes.length);
      setText('demo_anim',  'Animation: ' + (animating ? 'on' : 'off') + ' · mode: ' + transformMode);
      setText('demo_hint',  'Mode: ' + transformMode + ' · FPS: ' + fpsValue + ' · Nodes: ' + dynamicNodes.length);
    }

    function updateFps(ts) {
      fpsFrames++;
      if (!fpsLastTs) fpsLastTs = ts;
      const dt = ts - fpsLastTs;
      if (dt >= 500) {
        fpsValue = Math.round((fpsFrames * 1000) / dt);
        fpsFrames = 0;
        fpsLastTs = ts;
        updateCounters();
      }
    }

    function randomColor() {
      const palette = [
        '#F44336','#E91E63','#9C27B0','#673AB7',
        '#3F51B5','#2196F3','#00BCD4','#009688',
        '#4CAF50','#CDDC39','#FF9800','#FF5722',
      ];
      return palette[Math.floor(Math.random() * palette.length)];
    }

    function applyTransformDirect(el, px, py, pz, rx, ry, rz) {
      __tmpRot.x = rx; __tmpRot.y = ry; __tmpRot.z = rz;
      el.rotation = __tmpRot;

      __tmpPos.x = px; __tmpPos.y = py; __tmpPos.z = pz;
      el.position = __tmpPos;
    }

    // ── scale-aware spawn ─────────────────────────────────────────────────────

    function spawnAround(tag) {
      const el = root.createElement(tag);

      const angle = Math.random() * Math.PI * 2;
      const dist  = 3.5 + Math.random() * 250;
      const scale = dist * 0.1;

      const bx = Math.sin(angle) * dist;
      const bz = -Math.cos(angle) * dist;
      const off = nextId * 0.38;

      // Conservative bounds keep tilted primitives above the common y=0 floor.
      const groundY = scale * (tag === 'plane' ? 1.42 : tag === 'sphere' ? 0.5 : 1.1);
      const lift = Math.random() < 0.4
        ? 0
        : Math.random() * (1.0 + dist * 0.25);
      const by = groundY + lift;

      el.id = 'demo_dyn_' + nextId++;
      el.className = 'demo-dynamic';
      el.setAttribute('color', randomColor());

      let rx, ry, rz;

      if (tag === 'plane') {
        el.setAttribute('sx', String(scale * 2.0));
        el.setAttribute('sy', String(scale * 2.0));
        el.setAttribute('sz', '1');
        rx = -1.5708;
        ry = Math.random() * Math.PI * 2;
        rz = 0;
      } else if (tag === 'cylinder') {
        el.setAttribute('sx', String(scale));
        el.setAttribute('sy', String(scale * 1.6));
        el.setAttribute('sz', String(scale));
        rx = 0;
        ry = Math.random() * Math.PI * 2;
        rz = 0;
      } else {
        el.setAttribute('sx', String(scale));
        el.setAttribute('sy', String(scale));
        el.setAttribute('sz', String(scale));
        rx = Math.random() * 0.4;
        ry = Math.random() * Math.PI * 2;
        rz = Math.random() * 0.4;
      }

      applyTransformDirect(el, bx, by, bz, rx, ry, rz);

      root.appendChild(el);
      dynamicNodes.push({ el, bx, by, bz, off, groundY });

    }

    function spawn(tag, amount) {
      for (let i = 0; i < amount; i++) spawnAround(tag);
      updateCounters();
      setStatus('spawned ' + amount + ' ' + tag + ' objects');
    }

    // ── actions ───────────────────────────────────────────────────────────────

    function clearDynamic() {
      while (dynamicNodes.length) {
        const item = dynamicNodes.pop();
        if (item && item.el) item.el.remove();
      }
      updateCounters();
      setStatus('cleared all dynamic nodes');
    }

    function recolorDynamic() {
      dynamicNodes.forEach(({ el }) => el.setAttribute('color', randomColor()));
      setStatus('recolored ' + dynamicNodes.length + ' nodes');
    }

    function animateFrame(ts) {
      if (!animating) return;

      updateFps(ts);
      const t = ts / 1000;

      for (let i = 0; i < dynamicNodes.length; i++) {
        const item = dynamicNodes[i];
        const off = item.off;
        applyTransformDirect(
          item.el,
          item.bx,
          Math.max(item.groundY, item.by + Math.sin(t * 1.4 + off) * (item.by * 0.25)),
          item.bz,
          Math.sin(t * 0.5 + off) * 0.12,
          t * 0.9 + off,
          Math.sin(t * 0.7 + off) * 0.12
        );
      }

      requestAnimationFrame(animateFrame);
    }

    function toggleAnimation() {
      animating = !animating;
      fpsFrames = 0;
      fpsLastTs = 0;
      fpsValue = 0;
      updateCounters();
      setStatus(animating ? 'animation on' : 'animation off');
      if (animating) requestAnimationFrame(animateFrame);
    }

    function moveBanner() {
      const panel = byId('demo_panel');
      if (!panel) return;
      bannerMoved = !bannerMoved;

      if (bannerMoved) {
        applyTransformDirect(panel, 0.45, 0.70, -3.15, 0, 0.12, 0);
      } else {
        applyTransformDirect(panel, 0.00, 0.50, -3.15, 0, 0.00, 0);
      }

      setStatus(bannerMoved ? 'panel nudged' : 'panel reset');
    }

    function toggleFloor() {
      const floor = byId('demo_ground');
      if (!floor) return;
      floorVisible = !floorVisible;
      floor.setAttribute('visible', floorVisible ? 'true' : 'false');
      setStatus(floorVisible ? 'guides visible' : 'guides hidden');
    }

    function queryTest() {
      const all = root.getElementsByClass('demo-dynamic');
      const first = all.length ? all[0].id : 'none';
      setText('demo_hint', 'Mode: ' + transformMode + ' · FPS: ' + fpsValue + ' · Found ' + all.length + ' · first: ' + first);
      setStatus('query complete');
    }

    function ringArrange() {
      const n = dynamicNodes.length;
      if (!n) {
        setStatus('nothing to arrange');
        return;
      }

      dynamicNodes.forEach((item, i) => {
        const angle = (i / n) * Math.PI * 2;
        const ring  = i % 5;
        const dist  = 5 + ring * 5;
        const scale = dist * 0.1;
        const nx = Math.sin(angle) * dist;
        const nz = -Math.cos(angle) * dist;
        const ny = Math.max(item.groundY, scale * 0.5 + ring * 1.5);

        item.bx = nx;
        item.by = ny;
        item.bz = nz;

        applyTransformDirect(item.el, nx, ny, nz, 0, angle + Math.PI, 0);
      });

      setStatus('arranged ' + n + ' nodes in scale ring');
    }

    // ── bindings ──────────────────────────────────────────────────────────────

    const bindings = {
      demo_spawn_box:      () => spawn('box', 100),
      demo_spawn_sphere:   () => spawn('sphere', 25),
      demo_spawn_cylinder: () => spawn('cylinder', 25),
      demo_spawn_plane:    () => spawn('plane', 25),
      demo_stress:         () => spawn('box', 25),
      demo_recolor:        recolorDynamic,
      demo_toggle_anim:    toggleAnimation,
      demo_move_banner:    moveBanner,
      demo_toggle_floor:   toggleFloor,
      demo_clear:          clearDynamic,
      demo_query:          queryTest,
      demo_wave:           ringArrange,
      btn_demos:           () => { location.href = 'luna://demos'; },
      demo_direct_mode:    () => setStatus('direct transform mode only'),
    };

    Object.keys(bindings).forEach((id) => {
      const el = byId(id);
      if (el) {
        el.addEventListener('toque', bindings[id]);
        el.setAttribute("touchable", "true");
      }
    });

    updateCounters();
    setStatus('ready — mode=' + transformMode);
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
  <space resources="navigate_self,ux_embed">
  <group id="settings_content">
  <text x="0.000" y="2.550" z="-3.5" value="Luna Settings" size="0.240" />
  <text x="0.000" y="2.317" z="-3.5" value="Browser Configuration" size="0.108" />

  <text x="-1.950" y="1.945" z="-3.5" value="Appearance" size="0.090" color="#4CAF50" />
  <text x="-1.950" y="1.759" z="-3.5" value="- Theme: Default" size="0.060" />
  <text x="-1.950" y="1.620" z="-3.5" value="- Font Size: Medium" size="0.060" />

  <text x="0.000" y="1.945" z="-3.5" value="Performance" size="0.090" color="#2196F3" />
  <text x="0.000" y="1.759" z="-3.5" value="- Render Quality: High" size="0.060" />
  <text x="0.000" y="1.620" z="-3.5" value="- Cache Enabled: Yes" size="0.060" />

  <text x="1.950" y="1.945" z="-3.5" value="Network" size="0.090" color="#FF9800" />
  <text x="1.950" y="1.759" z="-3.5" value="- Proxy: None" size="0.060" />
  <text x="1.950" y="1.620" z="-3.5" value="- Timeout: 30s" size="0.060" />

  <group id="settings_navigation">
  <box x="-0.975" y="0.922" z="-3.5" sx="1.2" sy="0.3" sz="0.05" color="#9C27B0" id="btn_cache" touchable="true"/>
  <text x="-0.975" y="0.922" z="-3.45" value="Cache Stats" size="0.060" />

  <box x="0.975" y="0.922" z="-3.5" sx="1.2" sy="0.3" sz="0.05" color="#4CAF50" id="btn_home" touchable="true"/>
  <text x="0.975" y="0.922" z="-3.45" value="Back to Home" size="0.060" />

  </group>
  <box id="mcp_start" x="-0.975" y="1.28" z="-3.5" sx="1.2" sy="0.24" sz="0.05" color="#356F58" touchable="true"/>
  <text x="-0.975" y="1.28" z="-3.45" value="Iniciar MCP local" size="0.06"/>
  <box id="mcp_stop" x="0.975" y="1.28" z="-3.5" sx="1.2" sy="0.24" sz="0.05" color="#714B45" touchable="true"/>
  <text x="0.975" y="1.28" z="-3.45" value="Detener MCP" size="0.06"/>
  <text id="mcp_status" x="0" y="0.550" z="-3.5" value="MCP local" size="0.054" color="#B8CCC0"/>

  </group>
  <script>
    const btnHome = hiperspace.dimention.getElementById('btn_home');
    const btnCache = hiperspace.dimention.getElementById('btn_cache');

    if (btnHome) btnHome.addEventListener('toque', () => { location.href = 'luna://home'; });
    if (btnCache) btnCache.addEventListener('toque', () => { location.href = 'luna://cache-stats'; });

    const root = hiperspace.dimention;
    const start = root.getElementById('mcp_start');
    const stop = root.getElementById('mcp_stop');
    if (start) start.addEventListener('toque', () => root.setMcpEnabled(true));
    if (stop) stop.addEventListener('toque', () => root.setMcpEnabled(false));
    // One Settings document for spatial Home and the menu's window-local panel.
    function attachPanel(attempt) {
      const embedded = dimention.embedded;
      if (!embedded) {
        if (attempt < 50) setTimeout(() => attachPanel(attempt + 1), 5);
        return;
      }
      const navigation = root.getElementById('settings_navigation');
      if (navigation) navigation.setAttribute('visible', 'false');
      embedded.on('slot', slot => {
        const content = root.getElementById('settings_content');
        if (!content || slot.coordinateSpace !== 'window-local') return;
        const scale = Math.min(slot.size.x / 5.4, slot.size.y / 2.8);
        content.scale = { x: scale, y: scale, z: scale };
        content.position = { x: 0, y: -1.6 * scale, z: 3.58 * scale };
      });
      embedded.requestSlot({title:'Ajustes', minSize:{x:1.2,y:0.7,z:0.1}, preferredSize:{x:1.5,y:0.85,z:0.1}});
    }
    attachPanel(0);
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
  <plane y="1.6" z="-3.58" sx="5.2" sy="2.6" color="#304B43" touchable="false"/>
  <text x="0.000" y="2.550" z="-3.5" value="About Luna Browser" size="0.240" />
  <text x="0.000" y="2.310" z="-3.5" value="Spatial 3D Web Browser" size="0.108" />

  <text x="0.000" y="2.070" z="-3.5" value="Version: 0.1.0-alpha" size="0.084" />
  <text x="0.000" y="1.950" z="-3.5" value="Built with Bevy 0.14 + OpenXR" size="0.072" />

  <text x="0.000" y="1.710" z="-3.5" value="Features:" size="0.090" color="#4CAF50" />
  <text x="0.000" y="1.590" z="-3.5" value="- HSML (Spatial HTML) Parsing" size="0.060" />
  <text x="0.000" y="1.470" z="-3.5" value="- JavaScript Runtime (V8 via deno_core)" size="0.060" />
  <text x="0.000" y="1.350" z="-3.5" value="- Virtual Protocol Handler (luna://)" size="0.060" />
  <text x="0.000" y="1.230" z="-3.5" value="- HTTP Caching System" size="0.060" />
  <text x="0.000" y="1.110" z="-3.5" value="- DevTools with Console" size="0.060" />

  <text x="0.000" y="0.870" z="-3.5" value="Project: Luna Browser by Sam" size="0.066" color="#2196F3" />

  <box x="0.000" y="0.550" z="-3.5" sx="1.2" sy="0.3" sz="0.05" color="#4CAF50" id="btn_home" touchable="true"/>
  <text x="0.000" y="0.550" z="-3.45" value="Back to Home" size="0.060" />

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
  <plane y="1.6" z="-3.58" sx="5.2" sy="2.6" color="#304B43" touchable="false"/>
  <text x="0.000" y="2.550" z="-3.5" value="404 - Page Not Found" size="0.210" color="#F44336" />
  <text x="0.000" y="2.337" z="-3.5" value="The requested luna:// page does not exist" size="0.090" />

  <text x="0.000" y="2.082" z="-3.5" value="Available Pages:" size="0.096" color="#4CAF50" />
  <text x="0.000" y="1.933" z="-3.5" value="- luna://home" size="0.066" />
  <text x="0.000" y="1.827" z="-3.5" value="- luna://demos" size="0.066" />
  <text x="0.000" y="1.720" z="-3.5" value="- luna://settings" size="0.066" />
  <text x="0.000" y="1.614" z="-3.5" value="- luna://about" size="0.066" />
  <text x="0.000" y="1.507" z="-3.5" value="- luna://cache-stats" size="0.066" />
  <text x="0.000" y="1.401" z="-3.5" value="- luna://scale_demo" size="0.066" />
  <text x="0.000" y="1.295" z="-3.5" value="- luna://fire_demo" size="0.066" />
  <text x="0.000" y="1.188" z="-3.5" value="- luna://target_demo" size="0.066" />
  <text x="0.000" y="1.082" z="-3.5" value="- luna://range_demo" size="0.066" />

  <box x="-0.455" y="0.550" z="-3.5" sx="1.2" sy="0.3" sz="0.05" color="#4CAF50" id="btn_home" touchable="true"/>
  <text x="-0.455" y="0.550" z="-3.45" value="Go Home" size="0.060" />

  <box x="0.455" y="0.550" z="-3.5" sx="1.2" sy="0.3" sz="0.05" color="#7E57C2" id="btn_demos" touchable="true"/>
  <text x="0.455" y="0.550" z="-3.45" value="Go to Demos" size="0.060" />

  <script>
    const btnHome = hiperspace.dimention.getElementById('btn_home');
    const btnDemos = hiperspace.dimention.getElementById('btn_demos');
    if (btnHome) btnHome.addEventListener('toque', () => { location.href = 'luna://home'; });
    if (btnDemos) btnDemos.addEventListener('toque', () => { location.href = 'luna://demos'; });
    console.log('[luna://404] Error page loaded');
  </script>
  </space>
</hsml>"##;

// ============================================================================
// SCRIPTS INTERNOS
// ============================================================================

// Scripts internos UX viven en src/web/internal/*.js
const SCRIPT_HOME_NAV: &str = include_str!("web/internal/home_navigation.js");
const SCRIPT_ROOT_API: &str = include_str!("web/internal/root_api.js");
const SCRIPT_TABS_API: &str = include_str!("web/internal/tabs_api.js");
const SCRIPT_VIEWER_POSE_API: &str = include_str!("web/internal/viewer_pose_api.js");
const SCRIPT_EMBEDDED_API: &str = include_str!("web/internal/embedded_api.js");

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
  <plane y="1.6" z="-3.58" sx="5.2" sy="2.6" color="#304B43" touchable="false"/>
  <text x="0.000" y="2.550" z="-3.5" value="HTTP Cache Statistics" size="0.210" />
  <text x="0.000" y="2.250" z="-3.5" value="Real-time cache monitoring" size="0.096" />

  <text x="0.000" y="1.950" z="-3.5" value="Cache Entries: [Placeholder]" size="0.078" />
  <text x="0.000" y="1.800" z="-3.5" value="Total Size: [Placeholder]" size="0.078" />
  <text x="0.000" y="1.650" z="-3.5" value="Hit Rate: [Placeholder]" size="0.078" />

  <text x="0.000" y="1.300" z="-3.5" value="Note: Dynamic stats require runtime integration" size="0.066" color="#FFC107" />
  <text x="0.000" y="1.150" z="-3.5" value="Future: Pass cache resource to generate_cache_stats()" size="0.054" />

  <box x="0.000" y="0.550" z="-3.5" sx="1.2" sy="0.3" sz="0.05" color="#4CAF50" id="btn_back" touchable="true"/>
  <text x="0.000" y="0.550" z="-3.45" value="Back to Settings" size="0.060" />

  <script>
    const btn = hiperspace.dimention.getElementById('btn_back');
    if (btn) btn.addEventListener('toque', () => { location.href = 'luna://settings'; });
    console.log('[luna://cache-stats] Stats page loaded (placeholder)');
  </script>
  </space>
</hsml>"##.to_string()
}

// ============================================================================
// UX DOCUMENTS — los HSML viven en src/web/ux/*.hsml
// ============================================================================

const LUNA_UX_DESKTOP: &str = include_str!("web/ux/ux_desktop.hsml");
const LUNA_UX_VR: &str = include_str!("web/ux/ux_vr.hsml");
const LUNA_APP_DEMO_EMBEDDED: &str = include_str!("web/apps/demo_embedded.hsml");
// Legacy agent URLs resolve to Settings; the bridge now belongs to the host.

const LUNA_FIRE_DEMO: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Fire Bullet Demo</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space resources="navigate_self,read_pose_stream">
    <!-- Invisible volume that enables posemove while the controller is inside.
         Make it generous so the whole play area is covered. -->
    <posezone id="gun_zone" x="0" y="1.0" z="0" sx="40" sy="12" sz="40" visible="false" />

    <!-- Ground -->

    <!-- Control Panel (side-mounted left to clear forward firing arc) -->
    <group y="1.15" sy="0.8">
    <plane x="-2.5" y="0.50" z="-3.15" sx="2.50" sy="2.40" sz="1" color="#1a1a2e" id="control_panel" />

    <text x="-2.5" y="1.20" z="-3.0" value="Fire Bullet Demo" size="0.20" color="#FF6B6B" />
    <text x="-2.5" y="0.85" z="-3.0" value="Right trigger fires from the right controller" size="0.10" color="#888888" />
    <text x="-2.5" y="0.68" z="-3.0" value="FIRE button stays as manual fallback" size="0.08" color="#666666" />

    <!-- Action row -->
    <box x="-3.1" y="0.30" z="-3.0" sx="0.6" sy="0.25" sz="0.05" color="#FF6B6B" id="btn_fire" touchable="true"/>
    <text x="-3.1" y="0.30" z="-2.94" value="FIRE!" size="0.10" color="#FFFFFF" />

    <box x="-1.9" y="0.30" z="-3.0" sx="0.6" sy="0.25" sz="0.05" color="#4ECDC4" id="btn_clear" touchable="true"/>
    <text x="-1.9" y="0.30" z="-2.94" value="CLEAR" size="0.10" color="#FFFFFF" />

    <!-- Status -->
    <text x="-2.5" y="-0.05" z="-3.0" value="Bullets: 0" size="0.10" id="bullet_count" color="#FFD700" />
    <text x="-2.5" y="-0.22" z="-3.0" value="Ready to fire!" size="0.08" id="status_text" color="#FFFFFF" />

    <!-- Cross-demo nav row -->
    <box x="-2.5" y="-0.55" z="-3.0" sx="0.62" sy="0.22" sz="0.05" color="#7E57C2" id="btn_demos" touchable="true"/>
    <text x="-2.5" y="-0.55" z="-2.94" value="DEMOS" size="0.085" color="#FFFFFF" />

    </group>

    <script>
    const root = hiperspace.dimention;
    const bullets = [];
    let bulletCount = 0;
    const BULLET_SPEED = 100; // units per second
    const BULLET_SIZE = 0.16;
    const BULLET_MAX_DISTANCE = 120;
    const GRAVITY = 9.8; // simple gravity
    let lastFireTime = Number.NEGATIVE_INFINITY;
    let loopRunning = false;
    let lastFrameTs = 0;
    let latestRightPose = null;

    function byId(id) { return root.getElementById(id); }

    function setText(id, val) {
      const el = byId(id);
      if (el) el.setAttribute('value', val);
    }

    function setStatus(msg) {
      setText('status_text', msg);
      console.log('[fire_demo]', msg);
    }

    function updateBulletCount() {
      setText('bullet_count', 'Bullets: ' + bullets.length);
    }

    function normalizePose(evt) {
      const dx = Number(evt.dx ?? 0);
      const dy = Number(evt.dy ?? 0);
      const dz = Number(evt.dz ?? -1);
      const len = Math.hypot(dx, dy, dz) || 1;
      return {
        hand: String(evt.hand || ''),
        px: Number(evt.px ?? 0),
        py: Number(evt.py ?? 0),
        pz: Number(evt.pz ?? -2),
        dx: dx / len,
        dy: dy / len,
        dz: dz / len,
        trigger: Number(evt.trigger ?? 0),
        grip: Number(evt.grip ?? 0),
      };
    }

    function spawnBulletAt(startX, startY, startZ, dirX, dirY, dirZ, label) {
      const now = performance.now();
      if (now - lastFireTime < 100) return;
      lastFireTime = now;

      const bullet = root.createElement('sphere');
      bullet.id = 'bullet_' + bulletCount++;
      bullet.className = 'bullet';
      bullet.setAttribute('color', '#FFD700');
      bullet.setAttribute('sx', String(BULLET_SIZE));
      bullet.setAttribute('sy', String(BULLET_SIZE));
      bullet.setAttribute('sz', String(BULLET_SIZE));

      bullet.position = { x: startX, y: startY, z: startZ };
      bullet.rotation = { x: 0, y: 0, z: 0 };

      root.appendChild(bullet);

      bullets.push({
        el: bullet,
        x: startX,
        y: startY,
        z: startZ,
        vx: dirX * BULLET_SPEED,
        vy: dirY * BULLET_SPEED,
        vz: dirZ * BULLET_SPEED,
        startTime: now,
        maxDistance: BULLET_MAX_DISTANCE
      });

      // Start the loop only if it's not already running
      if (!loopRunning) {
        loopRunning = true;
        lastFrameTs = 0;
        requestAnimationFrame(animateBullets);
      }

      updateBulletCount();
      setStatus(label + ' (' + bullets.length + ' active)');
    }

    function fireBulletFromPose(pose) {
      const muzzle = 0.18;
      spawnBulletAt(
        pose.px + pose.dx * muzzle,
        pose.py + pose.dy * muzzle,
        pose.pz + pose.dz * muzzle,
        pose.dx,
        pose.dy,
        pose.dz,
        'Bullet fired from ' + pose.hand + ' controller!'
      );
    }

    function fireBullet() {
      if (latestRightPose) {
        fireBulletFromPose(latestRightPose);
        return;
      }
      spawnBulletAt(
        0, 0.3, -2,
        0, 0.02, -1,
        'Bullet fired from fallback origin!'
      );
    }

    function clearBullets() {
      bullets.forEach(b => {
        if (b.el) b.el.remove();
      });
      bullets.length = 0;
      bulletCount = 0;
      updateBulletCount();
      setStatus('All bullets cleared!');
    }

    function animateBullets(ts) {
      if (bullets.length === 0) {
        loopRunning = false;
        lastFrameTs = 0;
        return;
      }

      // Real delta time in seconds, clamped to avoid huge jumps
      // (first frame after pause, tab-switch, breakpoint, etc.)
      if (!lastFrameTs) lastFrameTs = ts;
      let dt = (ts - lastFrameTs) / 1000;
      lastFrameTs = ts;
      if (dt > 0.1) dt = 0.1;
      if (dt <= 0) dt = 1 / 60;

      for (let i = bullets.length - 1; i >= 0; i--) {
        const b = bullets[i];

        // Apply gravity
        b.vy -= GRAVITY * dt;

        // Update position
        b.x += b.vx * dt;
        b.y += b.vy * dt;
        b.z += b.vz * dt;

        // Remove if out of bounds
        if (Math.abs(b.x) > b.maxDistance ||
            Math.abs(b.z) > b.maxDistance ||
            b.y < BULLET_SIZE * 0.5) {
          b.el.remove();
          bullets.splice(i, 1);
          continue;
        }

        // Update element position
        if (b.el) {
          b.el.position = { x: b.x, y: b.y, z: b.z };
        }
      }

      updateBulletCount();
      requestAnimationFrame(animateBullets);
    }

    // Bindings
    const fireBtn = byId('btn_fire');
    if (fireBtn) fireBtn.addEventListener('toque', fireBullet);

    const gunZone = byId('gun_zone');
    if (gunZone) {
      gunZone.addEventListener('posemove', (evt) => {
        if (evt.hand !== 'right') return;
        latestRightPose = normalizePose(evt);

        // Hold trigger to shoot with a simple rate limit.
        if (latestRightPose.trigger > 0.75) {
          fireBulletFromPose(latestRightPose);
        }
      });
    }

    const clearBtn = byId('btn_clear');
    if (clearBtn) clearBtn.addEventListener('toque', clearBullets);

    const navLinks = {
      btn_demos: 'luna://demos',
    };
    Object.keys(navLinks).forEach((id) => {
      const el = byId(id);
      if (el) el.addEventListener('toque', () => { location.href = navLinks[id]; });
    });

    updateBulletCount();
    setStatus('Ready — right trigger uses posemove when available');
    </script>
  </space>
</hsml>"##;

const LUNA_TARGET_DEMO: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Target Practice Demo</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space resources="navigate_self,read_pose_stream">
    <posezone id="gun_zone" x="0" y="1.0" z="0" sx="40" sy="12" sz="40" visible="false" />


    <group y="1.15" sy="0.8">
    <plane x="-2.5" y="0.50" z="-3.15" sx="2.50" sy="2.40" sz="1" color="#1a1a2e" id="control_panel" />

    <text x="-2.5" y="1.20" z="-3.0" value="Target Practice" size="0.20" color="#FF1744" />
    <text x="-2.5" y="0.85" z="-3.0" value="Right trigger fires. Hit moving targets to score." size="0.10" color="#888888" />

    <box x="-3.1" y="0.50" z="-3.0" sx="0.6" sy="0.25" sz="0.05" color="#FF6B6B" id="btn_fire" touchable="true"/>
    <text x="-3.1" y="0.50" z="-2.94" value="FIRE!" size="0.10" color="#FFFFFF" />

    <box x="-1.9" y="0.50" z="-3.0" sx="0.6" sy="0.25" sz="0.05" color="#4ECDC4" id="btn_reset" touchable="true"/>
    <text x="-1.9" y="0.50" z="-2.94" value="RESET" size="0.10" color="#FFFFFF" />

    <text x="-2.5" y="0.20" z="-3.0" value="Score: 0" size="0.12" id="score_text" color="#FFD700" />
    <text x="-2.5" y="0.02" z="-3.0" value="Bullets: 0  Targets: 0" size="0.09" id="hud_text" color="#B0BEC5" />
    <text x="-2.5" y="-0.16" z="-3.0" value="Ready" size="0.08" id="status_text" color="#FFFFFF" />

    <!-- Cross-demo nav row -->
    <box x="-2.5" y="-0.55" z="-3.0" sx="0.62" sy="0.22" sz="0.05" color="#7E57C2" id="btn_demos" touchable="true"/>
    <text x="-2.5" y="-0.55" z="-2.94" value="DEMOS" size="0.085" color="#FFFFFF" />

    </group>

    <script>
    const root = hiperspace.dimention;
    const bullets = [];
    const targets = [];
    let bulletCount = 0;
    let targetCount = 0;
    let score = 0;
    const BULLET_SPEED = 80;
    const BULLET_SIZE = 0.16;
    const BULLET_MAX_DISTANCE = 120;
    const GRAVITY = 4.0;
    const TARGET_RADIUS = 0.6;
    const HIT_RADIUS = TARGET_RADIUS + BULLET_SIZE * 0.5;
    const NUM_TARGETS = 5;
    let lastFireTime = Number.NEGATIVE_INFINITY;
    let loopRunning = false;
    let lastFrameTs = 0;
    let latestRightPose = null;
    let triggerHeld = false;
    const TRIGGER_DOWN = 0.75;
    const TRIGGER_UP = 0.4;

    function byId(id) { return root.getElementById(id); }

    function setText(id, val) {
      const el = byId(id);
      if (el) el.setAttribute('value', val);
    }

    function setStatus(msg) {
      setText('status_text', msg);
      console.log('[target_demo]', msg);
    }

    function updateHud() {
      setText('score_text', 'Score: ' + score);
      setText('hud_text', 'Bullets: ' + bullets.length + '  Targets: ' + targets.length);
    }

    function spawnTarget() {
      const angle = Math.random() * Math.PI * 2;
      const dist = 6 + Math.random() * 10;
      const baseX = Math.sin(angle) * dist;
      const baseZ = -Math.abs(Math.cos(angle) * dist) - 3;
      const baseY = 1.0 + Math.random() * 1.8;

      const el = root.createElement('sphere');
      el.id = 'target_' + targetCount++;
      el.className = 'target';
      el.setAttribute('color', '#FF1744');
      el.setAttribute('sx', String(TARGET_RADIUS * 2));
      el.setAttribute('sy', String(TARGET_RADIUS * 2));
      el.setAttribute('sz', String(TARGET_RADIUS * 2));
      el.position = { x: baseX, y: baseY, z: baseZ };

      root.appendChild(el);

      targets.push({
        el,
        bx: baseX,
        by: baseY,
        bz: baseZ,
        x: baseX,
        y: baseY,
        z: baseZ,
        ampX: 1.5 + Math.random() * 2.5,
        ampY: 0.3 + Math.random() * 0.6,
        ampZ: 0.5 + Math.random() * 1.5,
        freqX: 0.5 + Math.random() * 0.8,
        freqY: 0.8 + Math.random() * 1.2,
        freqZ: 0.3 + Math.random() * 0.6,
        phase: Math.random() * Math.PI * 2,
      });
    }

    function spawnInitialTargets() {
      for (let i = 0; i < NUM_TARGETS; i++) spawnTarget();
      updateHud();
    }

    function resetGame() {
      targets.forEach(t => { if (t.el) t.el.remove(); });
      targets.length = 0;
      bullets.forEach(b => { if (b.el) b.el.remove(); });
      bullets.length = 0;
      score = 0;
      spawnInitialTargets();
      setStatus('Reset');
      ensureLoop();
    }

    function ensureLoop() {
      if (!loopRunning) {
        loopRunning = true;
        lastFrameTs = 0;
        requestAnimationFrame(animate);
      }
    }

    function normalizePose(evt) {
      const dx = Number(evt.dx ?? 0);
      const dy = Number(evt.dy ?? 0);
      const dz = Number(evt.dz ?? -1);
      const len = Math.hypot(dx, dy, dz) || 1;
      return {
        hand: String(evt.hand || ''),
        px: Number(evt.px ?? 0),
        py: Number(evt.py ?? 0),
        pz: Number(evt.pz ?? -2),
        dx: dx / len,
        dy: dy / len,
        dz: dz / len,
        trigger: Number(evt.trigger ?? 0),
        grip: Number(evt.grip ?? 0),
      };
    }

    function spawnBulletAt(startX, startY, startZ, dirX, dirY, dirZ, label) {
      const now = performance.now();
      if (now - lastFireTime < 100) return;
      lastFireTime = now;

      const bullet = root.createElement('sphere');
      bullet.id = 'bullet_' + bulletCount++;
      bullet.className = 'bullet';
      bullet.setAttribute('color', '#FFD700');
      bullet.setAttribute('sx', String(BULLET_SIZE));
      bullet.setAttribute('sy', String(BULLET_SIZE));
      bullet.setAttribute('sz', String(BULLET_SIZE));
      bullet.position = { x: startX, y: startY, z: startZ };

      root.appendChild(bullet);

      bullets.push({
        el: bullet,
        x: startX, y: startY, z: startZ,
        vx: dirX * BULLET_SPEED,
        vy: dirY * BULLET_SPEED,
        vz: dirZ * BULLET_SPEED,
      });

      ensureLoop();
      updateHud();
      setStatus(label);
    }

    function fireBulletFromPose(pose) {
      const muzzle = 0.18;
      spawnBulletAt(
        pose.px + pose.dx * muzzle,
        pose.py + pose.dy * muzzle,
        pose.pz + pose.dz * muzzle,
        pose.dx, pose.dy, pose.dz,
        'Shot from ' + pose.hand
      );
    }

    function fireBullet() {
      if (latestRightPose) { fireBulletFromPose(latestRightPose); return; }
      spawnBulletAt(0, 0.3, -2, 0, 0.02, -1, 'Shot from fallback');
    }

    function checkHits() {
      for (let i = bullets.length - 1; i >= 0; i--) {
        const b = bullets[i];
        for (let j = targets.length - 1; j >= 0; j--) {
          const t = targets[j];
          const dx = b.x - t.x;
          const dy = b.y - t.y;
          const dz = b.z - t.z;
          if (dx*dx + dy*dy + dz*dz <= HIT_RADIUS * HIT_RADIUS) {
            if (b.el) b.el.remove();
            if (t.el) t.el.remove();
            bullets.splice(i, 1);
            targets.splice(j, 1);
            score++;
            spawnTarget();
            setStatus('HIT! +1');
            break;
          }
        }
      }
    }

    function animate(ts) {
      if (bullets.length === 0 && targets.length === 0) {
        loopRunning = false;
        lastFrameTs = 0;
        return;
      }

      if (!lastFrameTs) lastFrameTs = ts;
      let dt = (ts - lastFrameTs) / 1000;
      lastFrameTs = ts;
      if (dt > 0.1) dt = 0.1;
      if (dt <= 0) dt = 1 / 60;

      const t = ts / 1000;

      for (let i = 0; i < targets.length; i++) {
        const tg = targets[i];
        tg.x = tg.bx + Math.sin(t * tg.freqX + tg.phase) * tg.ampX;
        tg.y = Math.max(TARGET_RADIUS, tg.by + Math.sin(t * tg.freqY + tg.phase) * tg.ampY);
        tg.z = tg.bz + Math.cos(t * tg.freqZ + tg.phase) * tg.ampZ;
        if (tg.el) tg.el.position = { x: tg.x, y: tg.y, z: tg.z };
      }

      for (let i = bullets.length - 1; i >= 0; i--) {
        const b = bullets[i];
        b.vy -= GRAVITY * dt;
        b.x += b.vx * dt;
        b.y += b.vy * dt;
        b.z += b.vz * dt;

        if (Math.abs(b.x) > BULLET_MAX_DISTANCE ||
            Math.abs(b.z) > BULLET_MAX_DISTANCE ||
            b.y < BULLET_SIZE * 0.5) {
          if (b.el) b.el.remove();
          bullets.splice(i, 1);
          continue;
        }
        if (b.el) b.el.position = { x: b.x, y: b.y, z: b.z };
      }

      checkHits();
      updateHud();
      requestAnimationFrame(animate);
    }

    const fireBtn = byId('btn_fire');
    if (fireBtn) fireBtn.addEventListener('toque', fireBullet);

    const resetBtn = byId('btn_reset');
    if (resetBtn) resetBtn.addEventListener('toque', resetGame);

    const navLinks = {
      btn_demos: 'luna://demos',
    };
    Object.keys(navLinks).forEach((id) => {
      const el = byId(id);
      if (el) el.addEventListener('toque', () => { location.href = navLinks[id]; });
    });

    const gunZone = byId('gun_zone');
    if (gunZone) {
      gunZone.addEventListener('posemove', (evt) => {
        if (evt.hand !== 'right') return;
        latestRightPose = normalizePose(evt);
        const t = latestRightPose.trigger;
        if (!triggerHeld && t > TRIGGER_DOWN) {
          triggerHeld = true;
          fireBulletFromPose(latestRightPose);
        } else if (triggerHeld && t < TRIGGER_UP) {
          triggerHeld = false;
        }
      });
    }

    spawnInitialTargets();
    ensureLoop();
    setStatus('Targets up — shoot them');
    </script>
  </space>
</hsml>"##;

const LUNA_RANGE_DEMO: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Shooting Range</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="scale" x="1" y="1" z="1"/>
    <meta type="rotation" x="0" y="0" z="0"/>
  </head>
  <space resources="navigate_self,read_pose_stream">
    <posezone id="gun_zone" x="0" y="1.0" z="0" sx="40" sy="12" sz="40" visible="false" />


    <!-- Range walls / lane markers -->

    <!-- Control panel (side-mounted left, clear of forward shooting axis) -->
    <group y="1.15" sy="0.8">
    <plane x="-3.5" y="0.50" z="-3.15" sx="2.80" sy="2.40" sz="1" color="#0d1b2a" id="control_panel" />

    <text x="-3.5" y="1.30" z="-3.0" value="Shooting Range" size="0.20" color="#D50000" />
    <text x="-3.5" y="1.00" z="-3.0" value="One pull = one shot. Hit all targets to advance." size="0.085" color="#888888" />

    <!-- Status / HUD -->
    <text x="-4.45" y="0.65" z="-3.0" value="LEVEL" size="0.075" color="#888888" />
    <text x="-4.45" y="0.50" z="-3.0" value="1" size="0.18" id="level_text" color="#FFD700" />

    <text x="-3.50" y="0.65" z="-3.0" value="SCORE" size="0.075" color="#888888" />
    <text x="-3.50" y="0.50" z="-3.0" value="0" size="0.18" id="score_text" color="#FFD700" />

    <text x="-2.55" y="0.65" z="-3.0" value="LEFT" size="0.075" color="#888888" />
    <text x="-2.55" y="0.50" z="-3.0" value="0" size="0.18" id="left_text" color="#00E676" />

    <text x="-3.5" y="0.18" z="-3.0" value="" size="0.085" id="banner_text" color="#4FC3F7" />

    <box x="-4.42" y="-0.15" z="-3.0" sx="0.42" sy="0.22" sz="0.05" color="#D50000" id="btn_start" touchable="true"/>
    <text x="-4.42" y="-0.15" z="-2.94" value="START" size="0.08" color="#FFFFFF" />

    <box x="-3.81" y="-0.15" z="-3.0" sx="0.42" sy="0.22" sz="0.05" color="#FF6B6B" id="btn_reset" touchable="true"/>
    <text x="-3.81" y="-0.15" z="-2.94" value="RESET" size="0.08" color="#FFFFFF" />

    <text x="-3.5" y="-0.40" z="-3.0" value="Idle" size="0.075" id="status_text" color="#FFFFFF" />

    <!-- Cross-demo nav row -->
    <box x="-3.5" y="-0.70" z="-3.0" sx="0.62" sy="0.22" sz="0.05" color="#7E57C2" id="btn_demos" touchable="true"/>
    <text x="-3.5" y="-0.70" z="-2.94" value="DEMOS" size="0.085" color="#FFFFFF" />

    </group>

    <script>
    const root = hiperspace.dimention;

    // ── Tunables ───────────────────────────────────────────────────────────
    const BULLET_SPEED   = 180;
    const BULLET_SIZE    = 0.10;
    const BULLET_MAX_DIST = 200;
    const GRAVITY        = 4.0;
    const FIRE_COOLDOWN  = 90;       // ms between shots
    const TRIGGER_DOWN   = 0.75;
    const TRIGGER_UP     = 0.40;
    // Sub-frame steps for bullet integration + target motion. Targets oscillate
    // on sin/cos paths that the swept-segment test approximates as a straight
    // line within a step — too coarse a step misses the curve.
    const SUBSTEPS       = 6;

    // Level config: progressively smaller, faster, more targets.
    // radius drops; speed scales position oscillation; count = hits to clear.
    const LEVELS = [
      { name: 'L1',  count: 4, radius: 0.30, speed: 0.6, distMin:  6, distMax: 10, color: '#FFAB00' },
      { name: 'L2',  count: 5, radius: 0.22, speed: 0.9, distMin:  7, distMax: 12, color: '#FF6F00' },
      { name: 'L3',  count: 6, radius: 0.16, speed: 1.2, distMin:  8, distMax: 14, color: '#E65100' },
      { name: 'L4',  count: 7, radius: 0.12, speed: 1.6, distMin: 10, distMax: 16, color: '#D50000' },
      { name: 'L5',  count: 8, radius: 0.09, speed: 2.0, distMin: 12, distMax: 20, color: '#B71C1C' },
      { name: 'L6+', count: 9, radius: 0.07, speed: 2.5, distMin: 14, distMax: 24, color: '#880E4F' },
    ];

    // ── State ──────────────────────────────────────────────────────────────
    const bullets = [];
    const targets = [];
    let bulletCount = 0;
    let targetCount = 0;
    let score = 0;
    let levelIdx = 0;
    let hitsThisLevel = 0;
    let running = false;
    let loopRunning = false;
    let lastFrameTs = 0;
    let lastFireTime = Number.NEGATIVE_INFINITY;
    let latestRightPose = null;
    let triggerHeld = false;
    let bannerUntil = 0;

    function byId(id) { return root.getElementById(id); }
    function setText(id, val) {
      const el = byId(id);
      if (el) el.setAttribute('value', val);
    }
    function setColor(id, val) {
      const el = byId(id);
      if (el) el.setAttribute('color', val);
    }

    function levelCfg() {
      return LEVELS[Math.min(levelIdx, LEVELS.length - 1)];
    }

    function setStatus(msg) {
      setText('status_text', msg);
    }

    function setBanner(msg, ms) {
      setText('banner_text', msg);
      bannerUntil = performance.now() + (ms || 1500);
    }

    function updateHud() {
      const cfg = levelCfg();
      setText('level_text', cfg.name);
      setText('score_text', String(score));
      setText('left_text', String(Math.max(0, cfg.count - hitsThisLevel)));
    }

    // ── Targets ────────────────────────────────────────────────────────────
    function spawnTarget() {
      const cfg = levelCfg();
      const angle = (Math.random() * 0.8 + 0.1) * Math.PI - Math.PI / 2; // front-ish arc
      const dist = cfg.distMin + Math.random() * (cfg.distMax - cfg.distMin);
      const baseX = Math.sin(angle) * dist;
      const baseZ = -Math.abs(Math.cos(angle) * dist) - 3;
      const baseY = 0.8 + Math.random() * 1.6;

      const el = root.createElement('sphere');
      el.id = 'rng_target_' + targetCount++;
      el.className = 'rng-target';
      el.setAttribute('color', cfg.color);
      el.setAttribute('sx', String(cfg.radius * 2));
      el.setAttribute('sy', String(cfg.radius * 2));
      el.setAttribute('sz', String(cfg.radius * 2));
      el.position = { x: baseX, y: baseY, z: baseZ };

      root.appendChild(el);

      targets.push({
        el,
        radius: cfg.radius,
        bx: baseX, by: baseY, bz: baseZ,
        x: baseX,  y: baseY,  z: baseZ,
        px: baseX, py: baseY, pz: baseZ,
        ampX: (0.4 + Math.random() * 1.5) * cfg.speed,
        ampY: (0.15 + Math.random() * 0.5) * cfg.speed,
        ampZ: (0.3 + Math.random() * 1.0) * cfg.speed,
        freqX: (0.4 + Math.random() * 0.8) * cfg.speed,
        freqY: (0.7 + Math.random() * 1.2) * cfg.speed,
        freqZ: (0.3 + Math.random() * 0.7) * cfg.speed,
        phase: Math.random() * Math.PI * 2,
      });
    }

    function spawnLevelTargets() {
      const cfg = levelCfg();
      for (let i = 0; i < cfg.count; i++) spawnTarget();
    }

    function clearTargets() {
      targets.forEach(t => { if (t.el) t.el.remove(); });
      targets.length = 0;
    }

    function clearBullets() {
      bullets.forEach(b => { if (b.el) b.el.remove(); });
      bullets.length = 0;
    }

    // ── Game flow ──────────────────────────────────────────────────────────
    function startRun() {
      score = 0;
      levelIdx = 0;
      hitsThisLevel = 0;
      running = true;
      clearBullets();
      clearTargets();
      spawnLevelTargets();
      updateHud();
      setBanner('Level ' + levelCfg().name + ' — go!', 1200);
      setStatus('Running');
      ensureLoop();
    }

    function resetRun() {
      running = false;
      clearBullets();
      clearTargets();
      score = 0;
      levelIdx = 0;
      hitsThisLevel = 0;
      updateHud();
      setBanner('', 0);
      setStatus('Reset — press START');
    }

    function advanceLevel() {
      levelIdx++;
      hitsThisLevel = 0;
      clearTargets();
      spawnLevelTargets();
      updateHud();
      setBanner('Level ' + levelCfg().name, 1500);
    }

    function registerHit() {
      score++;
      hitsThisLevel++;
      const cfg = levelCfg();
      if (hitsThisLevel >= cfg.count) {
        advanceLevel();
      } else {
        spawnTarget();
      }
      updateHud();
    }

    // ── Bullets ────────────────────────────────────────────────────────────
    function ensureLoop() {
      if (!loopRunning) {
        loopRunning = true;
        lastFrameTs = 0;
        requestAnimationFrame(animate);
      }
    }

    function normalizePose(evt) {
      const dx = Number(evt.dx ?? 0);
      const dy = Number(evt.dy ?? 0);
      const dz = Number(evt.dz ?? -1);
      const len = Math.hypot(dx, dy, dz) || 1;
      return {
        hand: String(evt.hand || ''),
        px: Number(evt.px ?? 0),
        py: Number(evt.py ?? 0),
        pz: Number(evt.pz ?? -2),
        dx: dx / len, dy: dy / len, dz: dz / len,
        trigger: Number(evt.trigger ?? 0),
        grip: Number(evt.grip ?? 0),
      };
    }

    function spawnBullet(sx, sy, sz, dx, dy, dz) {
      if (!running) return;
      const now = performance.now();
      if (now - lastFireTime < FIRE_COOLDOWN) return;
      lastFireTime = now;

      const b = root.createElement('sphere');
      b.id = 'rng_bullet_' + bulletCount++;
      b.className = 'rng-bullet';
      b.setAttribute('color', '#FFD700');
      b.setAttribute('sx', String(BULLET_SIZE));
      b.setAttribute('sy', String(BULLET_SIZE));
      b.setAttribute('sz', String(BULLET_SIZE));
      b.position = { x: sx, y: sy, z: sz };
      root.appendChild(b);

      bullets.push({
        el: b,
        x: sx, y: sy, z: sz,
        // Previous-frame position for swept-sphere collision. Without this,
        // at BULLET_SPEED=90 m/s a bullet moves ~1.5m per 16ms frame and
        // tunnels straight past targets smaller than that step.
        px: sx, py: sy, pz: sz,
        vx: dx * BULLET_SPEED,
        vy: dy * BULLET_SPEED,
        vz: dz * BULLET_SPEED,
      });
    }

    // Closest-point-on-segment-to-sphere-center test. Returns true iff the
    // sweep from (a) to (b) intersects sphere centered at (c) with `radius`.
    function segmentHitsSphere(ax, ay, az, bx, by, bz, cx, cy, cz, radius) {
      const dx = bx - ax, dy = by - ay, dz = bz - az;
      const wx = cx - ax, wy = cy - ay, wz = cz - az;
      const len2 = dx * dx + dy * dy + dz * dz;
      let s = len2 > 0 ? (wx * dx + wy * dy + wz * dz) / len2 : 0;
      if (s < 0) s = 0; else if (s > 1) s = 1;
      const px = ax + dx * s;
      const py = ay + dy * s;
      const pz = az + dz * s;
      const ex = px - cx, ey = py - cy, ez = pz - cz;
      return ex * ex + ey * ey + ez * ez <= radius * radius;
    }

    function fireFromPose(pose) {
      const muzzle = 0.18;
      spawnBullet(
        pose.px + pose.dx * muzzle,
        pose.py + pose.dy * muzzle,
        pose.pz + pose.dz * muzzle,
        pose.dx, pose.dy, pose.dz
      );
    }

    // ── Collision ──────────────────────────────────────────────────────────
    // Sweep bullet path in the target's reference frame (subtract target
    // motion) so a moving target is treated as stationary for the segment
    // test. Without this, a bullet that crossed where the target *was* would
    // miss after the target moved on by end of step.
    function checkHits() {
      for (let i = bullets.length - 1; i >= 0; i--) {
        const b = bullets[i];
        for (let j = targets.length - 1; j >= 0; j--) {
          const t = targets[j];
          if (segmentHitsSphere(
            b.px - t.px, b.py - t.py, b.pz - t.pz,
            b.x  - t.x,  b.y  - t.y,  b.z  - t.z,
            0, 0, 0,
            t.radius
          )) {
            if (b.el) b.el.remove();
            if (t.el) t.el.remove();
            bullets.splice(i, 1);
            targets.splice(j, 1);
            registerHit();
            break;
          }
        }
      }
    }

    function animate(ts) {
      if (!running && bullets.length === 0 && targets.length === 0) {
        loopRunning = false;
        lastFrameTs = 0;
        return;
      }

      if (!lastFrameTs) lastFrameTs = ts;
      let dt = (ts - lastFrameTs) / 1000;
      lastFrameTs = ts;
      if (dt > 0.1) dt = 0.1;
      if (dt <= 0) dt = 1 / 60;

      const subDt = dt / SUBSTEPS;
      const t0 = (ts - dt * 1000) / 1000;

      for (let s = 1; s <= SUBSTEPS; s++) {
        const tSub = t0 + subDt * s;

        for (let i = 0; i < targets.length; i++) {
          const tg = targets[i];
          tg.px = tg.x; tg.py = tg.y; tg.pz = tg.z;
          tg.x = tg.bx + Math.sin(tSub * tg.freqX + tg.phase) * tg.ampX;
          tg.y = Math.max(tg.radius, tg.by + Math.sin(tSub * tg.freqY + tg.phase) * tg.ampY);
          tg.z = tg.bz + Math.cos(tSub * tg.freqZ + tg.phase) * tg.ampZ;
        }

        for (let i = bullets.length - 1; i >= 0; i--) {
          const b = bullets[i];
          b.px = b.x; b.py = b.y; b.pz = b.z;
          b.vy -= GRAVITY * subDt;
          b.x += b.vx * subDt;
          b.y += b.vy * subDt;
          b.z += b.vz * subDt;
        }

        if (running) checkHits();

        for (let i = bullets.length - 1; i >= 0; i--) {
          const b = bullets[i];
          if (Math.abs(b.x) > BULLET_MAX_DIST ||
              Math.abs(b.z) > BULLET_MAX_DIST ||
              b.y < BULLET_SIZE * 0.5) {
            if (b.el) b.el.remove();
            bullets.splice(i, 1);
          }
        }
      }

      for (let i = 0; i < targets.length; i++) {
        const tg = targets[i];
        if (tg.el) tg.el.position = { x: tg.x, y: tg.y, z: tg.z };
      }
      for (let i = 0; i < bullets.length; i++) {
        const b = bullets[i];
        if (b.el) b.el.position = { x: b.x, y: b.y, z: b.z };
      }

      // Banner timeout
      if (bannerUntil && ts > bannerUntil) {
        setText('banner_text', '');
        bannerUntil = 0;
      }

      requestAnimationFrame(animate);
    }

    // ── Bindings ───────────────────────────────────────────────────────────
    const startBtn = byId('btn_start');
    if (startBtn) startBtn.addEventListener('toque', startRun);

    const resetBtn = byId('btn_reset');
    if (resetBtn) resetBtn.addEventListener('toque', resetRun);

    const navLinks = {
      btn_demos: 'luna://demos',
    };
    Object.keys(navLinks).forEach((id) => {
      const el = byId(id);
      if (el) el.addEventListener('toque', () => { location.href = navLinks[id]; });
    });

    const gunZone = byId('gun_zone');
    if (gunZone) {
      gunZone.addEventListener('posemove', (evt) => {
        if (evt.hand !== 'right') return;
        latestRightPose = normalizePose(evt);
        const t = latestRightPose.trigger;
        if (!triggerHeld && t > TRIGGER_DOWN) {
          triggerHeld = true;
          if (running) fireFromPose(latestRightPose);
        } else if (triggerHeld && t < TRIGGER_UP) {
          triggerHeld = false;
        }
      });
    }

    updateHud();
    setStatus('Press START to begin');
    setBanner('Welcome to the range', 2000);
    ensureLoop();
    </script>
  </space>
</hsml>"##;

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use js_runtime::Engine;
    use std::collections::HashMap;

    #[test]
    fn native_environment_survives_navigation_and_shell_changes() {
        let mut eng = Engine::new();
        eng.eval(r#"
            let nextNode = 1;
            const frames = [];
            globalThis.requestAnimationFrame = fn => frames.push(fn);
            class NativeNode {
                constructor(tag) {
                    this.nodeId = nextNode++; this.tagName = tag;
                    this.children = []; this.parent = null; this.attrs = {};
                    this.position = {x:0,y:0,z:0};
                    this.rotation = {x:0,y:0,z:0}; this.scale = {x:1,y:1,z:1};
                }
                setAttribute(k,v) { this.attrs[k] = String(v); }
                getAttribute(k) { return this.attrs[k] || ''; }
                appendChild(n) { this.children.push(n); n.parent = this; }
                remove() {
                    if (this.parent) this.parent.children = this.parent.children.filter(n => n !== this);
                    this.parent = null;
                }
                createElement(tag) { return new NativeNode(tag); }
            }
            globalThis.__nativeTestRoot = new NativeNode('space');
            function frame() { const batch = frames.splice(0); batch.forEach(fn => fn()); }
            function check(ok,msg) { if (!ok) throw new Error(msg); }
        "#).unwrap();
        eng.eval(&SCRIPT_ROOT_API.replace(
            "global.hiperspace && global.hiperspace.dimention", "global.__nativeTestRoot",
        )).unwrap();
        eng.eval(r#"
            const api = dimension.luna;
            const home = api.mountSpace('luna://home'); frame();
            const root = __nativeTestRoot;
            const env = root.children.find(n => n.id === 'luna_native_environment');
            check(env && env.getAttribute('visible') === 'true', 'home has no environment');
            const include = env.children[0];
            check(include.getAttribute('src') === 'luna://environment', 'wrong environment route');
            api.mountSpace('luna://about'); frame();
            check(root.children.includes(env) && env.children[0] === include, 'environment was remounted');
            api.switchMode('vr'); frame();
            check(api.listMountedSpaces().some(n => n.url === 'luna://about'), 'mode switch closed page');
            check(env.getAttribute('visible') === 'true', 'mode switch hid environment');
            const about = root.children.find(n => n.tagName === 'space' &&
                n.children.some(c => c.getAttribute('src') === 'luna://about'));
            about.children[0].setAttribute('src','https://example.test/'); frame();
            check(env.getAttribute('visible') === 'false', 'self-navigation leaked environment externally');
            about.children[0].setAttribute('src','luna://scale_demo'); frame();
            check(env.getAttribute('visible') === 'true', 'self-navigation did not restore environment');
            const external = api.mountSpace('https://example.test/');
            api.mountSpace('luna://settings', {kind:'app-embedded'}); frame();
            check(env.getAttribute('visible') === 'false', 'embedded app overrode external scene');
            api.unmountSpace(external); frame();
            check(env.getAttribute('visible') === 'false', 'empty browser retained environment');
            api.mountSpace('luna://home'); frame();
            check(root.children.filter(n => n.id === 'luna_native_environment').length === 1,
                'duplicate environment');
            check(env.children[0] === include, 'cached environment replaced');
        "#).unwrap();
    }

    #[test]
    fn scale_demo_spawn_batch_has_distinct_initial_positions_without_animation() {
        let mut eng = Engine::new();
        eng.update_tag_snapshot(HashMap::from([(0, "space".into())]));
        let script = LUNA_SCALE_DEMO.split_once("<script>").unwrap().1
            .split_once("</script>").unwrap().0;
        eng.eval(&format!("(function() {{ {script}\nspawn('box', 100); }})();")).unwrap();
        let creates = eng.drain_element_creation_queue();
        assert_eq!(creates.len(), 100);
        for (i, (request, _)) in creates.iter().enumerate() {
            eng.push_element_creation_result(*request, i as i32 + 1);
        }
        eng.fire_raf(1.0);
        let positions = eng.drain_transform_position_updates();
        assert_eq!(positions.len(), 100);
        let distinct: std::collections::HashSet<_> = positions.iter()
            .map(|(_, p)| (p.x.to_bits(), p.y.to_bits(), p.z.to_bits())).collect();
        assert_eq!(distinct.len(), 100, "spawned objects collapsed onto the last temporary vector");
        assert_eq!(eng.drain_hierarchy_append_queue().len(), 100);
    }

    #[test]
    fn shell_slot_matches_menu_pose_on_first_and_repeated_reposition() {
        let source = include_str!("web/ux/ux_vr.hsml");
        let reposition = source.split_once("function repositionShellAtViewer()").unwrap().1
            .split_once("// ── Toggle shell").unwrap().0;
        let compute = source.split_once("function computeFocusSlotPose()").unwrap().1
            .split_once("// The host follows the actual frame").unwrap().0;
        let mut eng = Engine::new();
        eng.eval(&format!(r#"
            const panel = new HSMLElement(5);
            const bottomBar = new HSMLElement(6);
            const focusZone = new HSMLElement(7);
            const shellState = {{ focusApp: null, anchored: [] }};
            const SHELL_SPAWN_DISTANCE = 2;
            const FOCUS_SLOT_W = 1, FOCUS_SLOT_H = 1, FOCUS_SLOT_D = 0.1;
            function repositionShellAtViewer() {reposition}
            function computeFocusSlotPose() {compute}
            for (const yaw of [0, 1.2, -0.8]) {{
                dimention.readViewerPose = () => ({{px: 4, py: 1.7, pz: -3, yaw}});
                repositionShellAtViewer();
                const slot = computeFocusSlotPose();
                if (Math.abs(slot.position.x - (4 - Math.sin(yaw)*2)) > 1e-6 ||
                    Math.abs(slot.position.z - (-3 - Math.cos(yaw)*2)) > 1e-6 ||
                    slot.rotation.y !== yaw || focusZone.position.x !== slot.position.x)
                    throw new Error('menu and app slot diverged before host commit');
            }}
        "#)).unwrap();
    }

    #[test]
    fn embedded_pose_updates_reuse_content_even_before_creation_resolves() {
        let mut eng = Engine::new();
        eng.update_tag_snapshot(HashMap::from([(0, "space".into()), (1, "group".into())]));
        eng.update_attr_snapshot(HashMap::from([(1, HashMap::from([("id".into(), "demo_root".into())]))]));
        eng.update_hierarchy_snapshot(HashMap::from([(1, 0)]), HashMap::from([(0, vec![1])]));
        eng.eval(r#"
            globalThis.slotHandlers = {};
            hiperspace.dimention.embedded = {
                on(type, cb) { slotHandlers[type] = cb; },
                requestSlot() {}, notifyReady() {}
            };
        "#).unwrap();
        let script = LUNA_APP_DEMO_EMBEDDED.split_once("<script>").unwrap().1
            .split_once("</script>").unwrap().0;
        eng.eval(&format!("(function() {{ {script} }})();")).unwrap();
        eng.eval(r#"
            for (let i = 0; i < 3; i++) slotHandlers.slot({
                position: {x: i, y: 1.5, z: -2}, rotation: {x:0,y:i*0.3,z:0}, size: {x:1,y:0.7}
            });
        "#).unwrap();
        let creates = eng.drain_element_creation_queue();
        assert_eq!(creates.len(), 2, "moving a slot must not destroy/recreate its contents");
        for (i, (request, _)) in creates.iter().enumerate() {
            eng.push_element_creation_result(*request, i as i32 + 10);
        }
        eng.fire_raf(1.0);
        assert!(eng.drain_remove_element_queue().is_empty());
        let positions = eng.drain_transform_position_updates();
        assert_eq!(positions.iter().filter(|(id, _)| *id == 0).last().unwrap().1.x, 2.0);
        assert!(positions.iter().filter(|(id, _)| *id >= 10).all(|(_, p)| p.x == 0.0));
    }

    #[test]
    fn shell_lifecycle_uses_host_identity_and_stable_window_controls() {
        let mut eng = Engine::new();
        eng.eval(r#"
            globalThis.__actions = [];
            let nextNode = 100;
            class UiNode {
                constructor(tag) {
                    this.tag = tag; this._nodeId = nextNode++; this.attrs = {};
                    this.children = []; this.listeners = {};
                    this.position = {x:0,y:0,z:0}; this.rotation = {x:0,y:0,z:0};
                    this.scale = {x:1,y:1,z:1};
                }
                _onResolved(cb) { cb(this._nodeId); }
                setAttribute(k,v) { this.attrs[k] = v; }
                getAttribute(k) { return this.attrs[k]; }
                appendChild(c) { this.children.push(c); c.parent = this; }
                remove() { this.removed = true; }
                addEventListener(k,fn) { (this.listeners[k] ||= []).push(fn); }
                removeEventListener(k,fn) { this.listeners[k] = (this.listeners[k] || []).filter(f => f !== fn); }
                fire(k) { for (const fn of this.listeners[k] || []) fn({}); }
            }
            const uiRoot = new UiNode('space');
            const named = new Map();
            uiRoot.getElementById = id => {
                if (!named.has(id)) named.set(id, new UiNode('group'));
                return named.get(id);
            };
            uiRoot.createElement = tag => new UiNode(tag);
            uiRoot.readViewerPose = () => ({px:4,py:1.7,pz:-2,yaw:0.8});
            uiRoot.tabs = {
                open(url, options) { __actions.push(['open',url]); },
                close(id) { __actions.push(['close',id]); },
                setVisible(id, value) { __actions.push(['visible',id,value]); }
            };
            globalThis.__testRoot = uiRoot;
        "#).unwrap();
        let document = include_str!("web/ux/ux_vr.hsml");
        let script = document.split_once("<script>").unwrap().1.split_once("</script>").unwrap().0
            .replacen("const root = hiperspace.dimention;", "const root = globalThis.__testRoot; const dimention = root;", 1);
        eng.eval(&format!(r#"(function() {{ {script}
            globalThis.shell = {{ state:shellState, open:openEmbeddedApp, message:handleAppMessage,
                anchor:onTapFocusAnchor, close:onTapFocusClose, minimize:onTapFocusMinimize,
                restore:onTapBarFocus, toggle:toggleShell, reposition:repositionShellAtViewer,
                barVisible:shouldShowBottomBar, apply:applyVisibility }};
        }})();"#)).unwrap();
        eng.eval(r#"
            function check(ok, msg) { if (!ok) throw new Error(msg); }
            function hostOpened(id) { shell.message({fromTabId:0,payload:JSON.stringify({type:'tabopened',tabId:id})}); }
            function ready(id) { shell.message({fromTabId:id,payload:JSON.stringify({type:'ready'})}); }
            shell.open({name:'A',url:'luna://demo_embedded'});
            check(shell.barVisible(), 'loading window must retain a cancel control');
            ready(999);
            check(shell.state.focusApp.tabId === 0, 'foreign app claimed pending focus');
            hostOpened(42);
            check(!shell.state.focusApp.ready, 'window revealed before app readiness');
            ready(42);
            check(shell.state.focusApp.ready && shell.barVisible(), 'ready focus must retain taskbar');
            check(__testRoot.getElementById('ux_vr_bottom_bar').attrs.visible === 'true', 'focused taskbar not rendered');
            check(__testRoot.getElementById('ux_vr_panel').attrs.visible === 'false', 'focus must still hide bookmarks');
            shell.minimize();
            check(shell.barVisible(), 'minimized focus must retain restore control');
            shell.restore();
            check(shell.barVisible(), 'restored focus must retain taskbar');
            shell.anchor();
            const first = shell.state.anchored[0];
            shell.open({name:'B',url:'luna://demo_embedded'}); hostOpened(43); ready(43); shell.anchor();
            const second = shell.state.anchored[1];
            const closeButton = second.frame.group.children.find(n => n.attrs.color === '#EF4444');
            const firstClose = first.frame.group.children.find(n => n.attrs.color === '#EF4444');
            firstClose.fire('toque');
            check(shell.state.anchored.length === 1 && shell.state.anchored[0] === second, 'wrong anchor removed');
            closeButton.fire('toque');
            check(shell.state.anchored.length === 0, 'surviving titlebar retained stale array index');
            check(__actions.some(a => a[0] === 'close' && a[1] === 43), 'remaining app not closed');
            shell.open({name:'Slow',url:'luna://slow'});
            shell.open({name:'Replacement',url:'luna://replacement'});
            hostOpened(44);
            check(__actions.some(a => a[0] === 'close' && a[1] === 44), 'superseded pending app leaked');
            check(shell.state.focusApp.tabId === 0 && shell.state.focusApp.pendingUrl === 'luna://replacement', 'replacement lost');
            hostOpened(45); ready(44);
            check(!shell.state.focusApp.ready, 'late closed app revealed replacement');
            ready(45);
            shell.reposition();
        "#).unwrap();
        let slots: Vec<serde_json::Value> = eng.drain_shell_outbox().iter()
            .filter_map(|m| serde_json::from_str::<serde_json::Value>(&m.payload).ok())
            .filter(|p| p["type"] == "slot").collect();
        assert!(!slots.is_empty());
        for slot in slots {
            assert_eq!(slot["coordinateSpace"], "window-local");
            assert_eq!(slot["position"]["x"], 0);
            assert!(slot["anchorNodeId"].as_i64().unwrap() >= 100);
        }
    }

    #[test]
    fn embedded_api_replays_initial_slot_and_reinjection_keeps_one_consumer() {
        let mut eng = Engine::new();
        eng.push_shell_messages(vec![js_runtime::ShellMessage {
            target_tab_id: 0,
            payload: r#"{"type":"slot","coordinateSpace":"window-local","revision":1,"size":{"x":1}}"#.into(),
        }]);
        eng.eval(SCRIPT_EMBEDDED_API).unwrap();
        eng.eval(r#"
            globalThis.receivedSlots = 0;
            const firstApi = dimention.embedded;
            firstApi.on('slot', slot => { if (slot.size.x !== 1) throw Error('wrong slot'); receivedSlots++; });
        "#).unwrap();
        eng.eval(SCRIPT_EMBEDDED_API).unwrap();
        eng.fire_raf(0.0);
        eng.eval("if (receivedSlots !== 1 || dimention.embedded !== firstApi) throw Error('lost slot or duplicate API');").unwrap();
    }

    fn fire_demo_script() -> String {
        let start = LUNA_FIRE_DEMO.find("<script>").unwrap() + "<script>".len();
        let end = LUNA_FIRE_DEMO.find("</script>").unwrap();
        LUNA_FIRE_DEMO[start..end].trim().to_string()
    }

    fn fire_demo_engine() -> Engine {
        let mut eng = Engine::new();

        let mut attrs = HashMap::new();
        attrs.insert(0, HashMap::new());
        attrs.insert(1, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "btn_fire".to_string());
            m
        });
        attrs.insert(2, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "btn_clear".to_string());
            m
        });
        attrs.insert(3, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "btn_home".to_string());
            m
        });
        attrs.insert(4, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "bullet_count".to_string());
            m
        });
        attrs.insert(5, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "status_text".to_string());
            m
        });
        attrs.insert(6, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "gun_zone".to_string());
            m
        });
        eng.update_attr_snapshot(attrs);

        let mut tags = HashMap::new();
        tags.insert(0, "hsml".to_string());
        tags.insert(1, "box".to_string());
        tags.insert(2, "box".to_string());
        tags.insert(3, "box".to_string());
        tags.insert(4, "text".to_string());
        tags.insert(5, "text".to_string());
        tags.insert(6, "posezone".to_string());
        eng.update_tag_snapshot(tags);

        let mut parents = HashMap::new();
        parents.insert(0, -1);
        for node_id in 1..=6 {
            parents.insert(node_id, 0);
        }

        let mut children = HashMap::new();
        children.insert(0, vec![1, 2, 3, 4, 5, 6]);
        for node_id in 1..=6 {
            children.insert(node_id, vec![]);
        }
        eng.update_hierarchy_snapshot(parents, children);

        eng.update_transform_snapshot(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        );

        eng.eval(&fire_demo_script()).unwrap();
        eng.drain_attr_updates();
        eng.drain_logs();
        eng
    }

    fn range_demo_script() -> String {
        let start = LUNA_RANGE_DEMO.find("<script>").unwrap() + "<script>".len();
        let end = LUNA_RANGE_DEMO.find("</script>").unwrap();
        LUNA_RANGE_DEMO[start..end].trim().to_string()
    }

    fn range_demo_engine() -> Engine {
        let mut eng = Engine::new();

        let mut attrs = HashMap::new();
        attrs.insert(0, HashMap::new());
        attrs.insert(1, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "gun_zone".to_string());
            m
        });
        attrs.insert(2, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "btn_start".to_string());
            m
        });
        attrs.insert(3, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "btn_reset".to_string());
            m
        });
        attrs.insert(4, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "status_text".to_string());
            m
        });
        attrs.insert(5, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "level_text".to_string());
            m
        });
        attrs.insert(6, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "score_text".to_string());
            m
        });
        attrs.insert(7, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "left_text".to_string());
            m
        });
        attrs.insert(8, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "banner_text".to_string());
            m
        });
        attrs.insert(9, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "btn_demos".to_string());
            m
        });
        eng.update_attr_snapshot(attrs);

        let mut tags = HashMap::new();
        tags.insert(0, "hsml".to_string());
        tags.insert(1, "posezone".to_string());
        tags.insert(2, "box".to_string());
        tags.insert(3, "box".to_string());
        tags.insert(4, "text".to_string());
        tags.insert(5, "text".to_string());
        tags.insert(6, "text".to_string());
        tags.insert(7, "text".to_string());
        tags.insert(8, "text".to_string());
        tags.insert(9, "box".to_string());
        eng.update_tag_snapshot(tags);

        let mut parents = HashMap::new();
        parents.insert(0, -1);
        for node_id in 1..=9 {
            parents.insert(node_id, 0);
        }
        let mut children = HashMap::new();
        children.insert(0, (1..=9).collect());
        for node_id in 1..=9 {
            children.insert(node_id, vec![]);
        }
        eng.update_hierarchy_snapshot(parents, children);

        let empty = HashMap::new();
        eng.update_transform_snapshot(empty.clone(), empty.clone(), empty.clone(), empty);
        eng.eval(&range_demo_script()).unwrap();
        eng
    }

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
    fn root_api_has_switch_mode() {
        let content = VIRTUAL_ROUTES
            .resolve("luna://internal/root_api.js")
            .unwrap();
        assert!(content.contains("switchMode"));
        assert!(content.contains("luna://ux_desktop"));
        assert!(content.contains("luna://ux_vr"));
    }

    #[test]
    fn root_api_exposes_tab_id_contract_for_managed_spaces() {
        let content = VIRTUAL_ROUTES
            .resolve("luna://internal/root_api.js")
            .unwrap();
        assert!(content.contains("opts.tabId"));
        assert!(content.contains("data-luna-tab-id"));
        assert!(content.contains("tabId: entry.space.getAttribute('data-luna-tab-id')"));
    }

    #[test]
    fn controller_routes_are_gone() {
        let content = VIRTUAL_ROUTES.resolve("luna://controller_desktop").unwrap();
        assert!(content.contains("404"));

        let content = VIRTUAL_ROUTES.resolve("luna://controller_vr").unwrap();
        assert!(content.contains("404"));

        let content = VIRTUAL_ROUTES.resolve("luna://internal/controller_toque.js").unwrap();
        assert!(content.contains("404"));
    }

    #[test]
    fn root_api_no_longer_mentions_controller_resources() {
        let content = VIRTUAL_ROUTES
            .resolve("luna://internal/root_api.js")
            .unwrap();
        assert!(!content.contains("controller_desktop"));
        assert!(!content.contains("controller_vr"));
    }

    #[test]
    fn home_route_requests_only_navigate_self() {
        let content = VIRTUAL_ROUTES.resolve("luna://home").unwrap();
        assert!(content.contains(r#"resources="navigate_self""#));
        assert!(!content.contains("controller_desktop"));
        assert!(!content.contains("controller_vr"));
    }

    #[test]
    fn fire_demo_manual_fire_spawns_bullet() {
        let mut eng = fire_demo_engine();

        eng.push_dom_toque_event(1, 0.0, 0.0, 0.0);
        eng.fire_raf(16.0);

        let created = eng.drain_element_creation_queue();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].1, "sphere");

        let attr_updates = eng.drain_attr_updates();
        assert!(attr_updates.iter().any(|(node_id, key, value)| {
            *node_id == 4 && key == "value" && value.contains("Bullets: 1")
        }));
        assert!(attr_updates.iter().any(|(node_id, key, value)| {
            *node_id == 5 && key == "value" && value.contains("fallback origin")
        }));
    }

    #[test]
    fn fire_demo_posemove_trigger_spawns_bullet() {
        let mut eng = fire_demo_engine();

        // Args: node_id, hand, px, py, pz, dx, dy, dz, trigger, grip, qx, qy, qz, qw.
        eng.push_posemove_event(6, "right", 1.0, 1.5, -0.5, 0.0, 0.0, -1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0);
        eng.fire_raf(16.0);

        let created = eng.drain_element_creation_queue();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].1, "sphere");

        let attr_updates = eng.drain_attr_updates();
        assert!(attr_updates.iter().any(|(node_id, key, value)| {
            *node_id == 4 && key == "value" && value.contains("Bullets: 1")
        }));
        assert!(attr_updates.iter().any(|(node_id, key, value)| {
            *node_id == 5 && key == "value" && value.contains("right controller")
        }));
    }

    #[test]
    fn range_demo_start_then_trigger_spawns_bullet() {
        let mut eng = range_demo_engine();

        eng.push_dom_toque_event(2, 0.0, 0.0, 0.0);
        eng.fire_raf(16.0);

        eng.push_posemove_event(
            1,
            "right",
            0.0,
            1.4,
            -1.0,
            0.0,
            0.0,
            -1.0,
            1.0,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        );
        eng.fire_raf(16.0);

        let created = eng.drain_element_creation_queue();
        let sphere_creates = created
            .iter()
            .filter(|(_, tag)| tag == "sphere")
            .count();
        assert!(
            sphere_creates >= 2,
            "expected at least one target and one bullet sphere after start+trigger, got {:?}",
            created
        );

        let attr_updates = eng.drain_attr_updates();
        assert!(
            attr_updates.iter().any(|(node_id, key, value)| {
                *node_id == 4 && key == "value" && value.contains("Running")
            }),
            "status_text should switch to Running"
        );
    }

    /// Hammer createElement/remove cycles on root and validate JS-side queue
    /// coherence. Repro target: rapid recycle of bullets/targets that has been
    /// causing downstream Bevy entity panics
    /// ("Could not insert a bundle ... because it doesn't exist").
    ///
    /// Invariants checked:
    ///  - every drained create request gets a unique request_id
    ///  - every drained remove id was previously assigned by us
    ///  - no node_id appears in the remove queue twice
    ///  - total creates == total removes after full churn
    ///  - no duplicate (node_id, key, value) attr update for a stale (already-removed) id
    #[test]
    fn dom_stress_create_remove_cycles() {
        use std::collections::HashSet;

        let mut eng = Engine::new();

        let mut tags = HashMap::new();
        tags.insert(0, "hsml".to_string());
        eng.update_tag_snapshot(tags);

        let mut parents = HashMap::new();
        parents.insert(0, -1);
        let mut children = HashMap::new();
        children.insert(0, vec![]);
        eng.update_hierarchy_snapshot(parents, children);

        let mut attrs = HashMap::new();
        attrs.insert(0, HashMap::new());
        eng.update_attr_snapshot(attrs);
        eng.update_transform_snapshot(
            HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new(),
        );

        eng.eval(r#"
            const root = hiperspace.dimention;
            const live = [];
            globalThis.__stress_create = (n) => {
                for (let i = 0; i < n; i++) {
                    const el = root.createElement('sphere');
                    el.setAttribute('color', '#FFD700');
                    el.setAttribute('sx', '0.16');
                    el.position = { x: i * 0.1, y: 0.5, z: -2 };
                    root.appendChild(el);
                    live.push(el);
                }
            };
            globalThis.__stress_remove_all = () => {
                while (live.length) live.pop().remove();
            };
            globalThis.__stress_remove_some = (n) => {
                for (let i = 0; i < n && live.length; i++) live.pop().remove();
            };
            globalThis.__stress_count = () => live.length;
        "#).unwrap();

        eng.drain_element_creation_queue();
        eng.drain_remove_element_queue();
        eng.drain_attr_updates();
        eng.drain_logs();

        let cycles = 40;
        let batch = 25;
        let mut next_real_id: i32 = 100;
        let mut all_assigned: HashSet<i32> = HashSet::new();
        let mut all_removed: HashSet<i32> = HashSet::new();

        for cycle in 0..cycles {
            // Spawn a batch
            eng.eval(&format!("__stress_create({})", batch)).unwrap();
            let creates = eng.drain_element_creation_queue();
            assert_eq!(
                creates.len(),
                batch,
                "cycle {}: expected {} creates, got {}",
                cycle, batch, creates.len()
            );

            let mut req_ids: HashSet<i32> = HashSet::new();
            for (req_id, tag) in &creates {
                assert_eq!(tag, "sphere");
                assert!(req_ids.insert(*req_id), "duplicate request_id {}", req_id);

                let real_id = next_real_id;
                next_real_id += 1;
                eng.push_element_creation_result(*req_id, real_id);
                assert!(
                    all_assigned.insert(real_id),
                    "duplicate real id {} assigned",
                    real_id
                );
            }

            // Pump pending callbacks (resolve queued setAttribute/position/etc).
            eng.fire_raf((cycle * 16) as f64);

            // Tear down half, leave half live across cycles to grow the live set.
            let to_remove = batch / 2;
            eng.eval(&format!("__stress_remove_some({})", to_remove)).unwrap();
            eng.fire_raf((cycle * 16 + 8) as f64);

            let removes = eng.drain_remove_element_queue();
            assert_eq!(
                removes.len(),
                to_remove,
                "cycle {}: expected {} removes, got {}",
                cycle, to_remove, removes.len()
            );
            for id in &removes {
                assert!(
                    all_assigned.contains(id),
                    "remove for never-assigned id {}",
                    id
                );
                assert!(
                    all_removed.insert(*id),
                    "duplicate remove queued for id {}",
                    id
                );
            }
        }

        // Final sweep: remove every survivor.
        eng.eval("__stress_remove_all()").unwrap();
        eng.fire_raf((cycles * 16 + 100) as f64);
        let final_removes = eng.drain_remove_element_queue();
        for id in &final_removes {
            assert!(
                all_assigned.contains(id),
                "final remove for unknown id {}",
                id
            );
            assert!(
                all_removed.insert(*id),
                "duplicate final remove for id {}",
                id
            );
        }

        assert_eq!(
            all_assigned.len(),
            cycles * batch,
            "total creates mismatch"
        );
        assert_eq!(
            all_removed.len(),
            cycles * batch,
            "total removes != total creates ({} created, {} removed)",
            all_assigned.len(),
            all_removed.len()
        );

        // Engine should have logged no errors (e.g. throws from ops on dead refs).
        let logs = eng.drain_logs();
        let errors: Vec<_> = logs.iter().filter(|(level, _)| level == "error").collect();
        assert!(
            errors.is_empty(),
            "stress run produced JS error logs: {:?}",
            errors
        );
    }

    /// Same churn pattern as target_demo: bursts of bullet spawns, then a full
    /// reset that wipes everything. Reset path is exactly what was crashing in
    /// production.
    #[test]
    fn dom_stress_burst_then_reset() {
        use std::collections::HashSet;

        let mut eng = Engine::new();

        let mut tags = HashMap::new();
        tags.insert(0, "hsml".to_string());
        eng.update_tag_snapshot(tags);

        let mut parents = HashMap::new();
        parents.insert(0, -1);
        let mut children = HashMap::new();
        children.insert(0, vec![]);
        eng.update_hierarchy_snapshot(parents, children);

        let mut attrs = HashMap::new();
        attrs.insert(0, HashMap::new());
        eng.update_attr_snapshot(attrs);
        eng.update_transform_snapshot(
            HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new(),
        );

        eng.eval(r#"
            const root = hiperspace.dimention;
            const pool = [];
            globalThis.__burst = (n) => {
                for (let i = 0; i < n; i++) {
                    const el = root.createElement('sphere');
                    el.setAttribute('color', '#FF0000');
                    root.appendChild(el);
                    pool.push(el);
                }
            };
            globalThis.__reset = () => {
                while (pool.length) pool.pop().remove();
            };
        "#).unwrap();

        eng.drain_element_creation_queue();
        eng.drain_remove_element_queue();
        eng.drain_logs();

        let bursts = 15;
        let burst_size = 30;
        let mut next_real_id: i32 = 1000;
        let mut total_assigned = 0usize;
        let mut total_removed = 0usize;
        let mut seen_ids: HashSet<i32> = HashSet::new();

        for cycle in 0..bursts {
            eng.eval(&format!("__burst({})", burst_size)).unwrap();
            let creates = eng.drain_element_creation_queue();
            assert_eq!(creates.len(), burst_size);

            for (req_id, _) in &creates {
                let id = next_real_id;
                next_real_id += 1;
                eng.push_element_creation_result(*req_id, id);
                assert!(seen_ids.insert(id));
                total_assigned += 1;
            }
            eng.fire_raf((cycle * 100) as f64);

            // Full reset.
            eng.eval("__reset()").unwrap();
            eng.fire_raf((cycle * 100 + 50) as f64);

            let removes = eng.drain_remove_element_queue();
            assert_eq!(
                removes.len(),
                burst_size,
                "burst {}: removes != burst_size",
                cycle
            );

            let unique: HashSet<i32> = removes.iter().copied().collect();
            assert_eq!(unique.len(), removes.len(), "duplicate id in remove batch");
            for id in &removes {
                assert!(seen_ids.contains(id), "remove for unknown id {}", id);
            }
            total_removed += removes.len();
        }

        assert_eq!(total_assigned, total_removed);
        assert_eq!(total_assigned, bursts * burst_size);

        let logs = eng.drain_logs();
        let errors: Vec<_> = logs.iter().filter(|(level, _)| level == "error").collect();
        assert!(errors.is_empty(), "burst run produced errors: {:?}", errors);
    }

    /// Adversarial pattern: create then immediately remove BEFORE backend
    /// resolves the request_id. The remove must be queued via `_onResolved`
    /// and then fire as soon as the real id arrives. If the JS layer races
    /// here, downstream we get a stale entity_map entry → Bevy panic on next
    /// command flush.
    #[test]
    fn dom_stress_create_then_remove_before_resolve() {
        let mut eng = Engine::new();

        let mut tags = HashMap::new();
        tags.insert(0, "hsml".to_string());
        eng.update_tag_snapshot(tags);

        let mut parents = HashMap::new();
        parents.insert(0, -1);
        let mut children = HashMap::new();
        children.insert(0, vec![]);
        eng.update_hierarchy_snapshot(parents, children);

        let mut attrs = HashMap::new();
        attrs.insert(0, HashMap::new());
        eng.update_attr_snapshot(attrs);
        eng.update_transform_snapshot(
            HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new(),
        );

        eng.eval(r#"
            const root = hiperspace.dimention;
            globalThis.__churn = (n) => {
                for (let i = 0; i < n; i++) {
                    const el = root.createElement('sphere');
                    el.setAttribute('color', '#0F0');
                    root.appendChild(el);
                    el.remove();
                }
            };
        "#).unwrap();

        eng.drain_element_creation_queue();
        eng.drain_remove_element_queue();
        eng.drain_logs();

        let n = 200;
        eng.eval(&format!("__churn({})", n)).unwrap();

        let creates = eng.drain_element_creation_queue();
        assert_eq!(creates.len(), n);

        // Removes were queued via _onResolved BEFORE we assigned ids.
        // Up to this point the remove queue should still be empty.
        let early_removes = eng.drain_remove_element_queue();
        assert!(
            early_removes.is_empty(),
            "remove fired before id resolution: {:?}",
            early_removes
        );

        let mut assigned: Vec<i32> = Vec::with_capacity(n);
        for (i, (req_id, _)) in creates.iter().enumerate() {
            let id = 5000 + i as i32;
            eng.push_element_creation_result(*req_id, id);
            assigned.push(id);
        }

        eng.fire_raf(16.0);

        // After resolution, the queued remove for each must have fired exactly once.
        let removes = eng.drain_remove_element_queue();
        assert_eq!(removes.len(), n, "post-resolve remove count off");

        let assigned_set: std::collections::HashSet<i32> = assigned.into_iter().collect();
        let mut seen = std::collections::HashSet::new();
        for id in &removes {
            assert!(assigned_set.contains(id), "remove for unknown id {}", id);
            assert!(seen.insert(*id), "duplicate post-resolve remove for {}", id);
        }

        let logs = eng.drain_logs();
        let errors: Vec<_> = logs.iter().filter(|(level, _)| level == "error").collect();
        assert!(errors.is_empty(), "churn produced errors: {:?}", errors);
    }
}
