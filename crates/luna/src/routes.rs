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

        // UX routes
        routes.insert("ux_desktop".to_string(), RouteHandler::Static(LUNA_UX_DESKTOP));
        routes.insert("ux_vr".to_string(), RouteHandler::Static(LUNA_UX_VR));
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

    <!-- ═══════════════════════════════════════════════════════════════════════
         ENVIRONMENT
         ═══════════════════════════════════════════════════════════════════════ -->

    <!-- Ground extending 80m radius -->
    <plane x="0" y="-0.02" z="0" rx="-1.5708" sx="80" sy="80" sz="1" color="#080D12" id="demo_ground" />

    <!-- ═══════════════════════════════════════════════════════════════════════
         SCALE RING — 8 anchors, each at a different distance and direction.
         Scale rule: size = distance × 0.1  (4 m → 0.4 m  …  35 m → 3.5 m)
         All appear roughly the same angular size from the origin.
         ═══════════════════════════════════════════════════════════════════════ -->

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

    <!-- ═══════════════════════════════════════════════════════════════════════
         CONTROL PANEL  (at z = -3.1, eye level)
         ═══════════════════════════════════════════════════════════════════════ -->

    <plane x="0" y="0.50" z="-3.15" sx="5.10" sy="2.50" sz="1" color="#0D1B2A" id="demo_panel" />

    <text x="0" y="1.68" z="-3.0" value="Luna Scale Demo" size="0.22" color="#64B5F6" />
    <text x="0" y="1.38" z="-3.0" value="size = distance × 0.1  (all same angular size)" size="0.088" color="#546E7A" />

    <!-- Column headers -->
    <text x="-1.70" y="1.07" z="-3.0" value="Spawn" size="0.10" color="#8BC34A" />
    <text x="0.00"  y="1.07" z="-3.0" value="Modify" size="0.10" color="#03A9F4" />
    <text x="1.70"  y="1.07" z="-3.0" value="Navigate" size="0.10" color="#FFB74D" />

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
    <text x="0.00" y="-0.13" z="-2.94" value="Toggle Floor" size="0.07" />

    <box x="0.00" y="-0.42" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#E53935" id="demo_clear" />
    <text x="0.00" y="-0.42" z="-2.94" value="Clear All" size="0.07" />

    <!-- ── Navigate column ── -->
    <box x="1.70" y="0.74" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#4CAF50" id="btn_home" />
    <text x="1.70" y="0.74" z="-2.94" value="Home" size="0.07" />

    <box x="1.70" y="0.45" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#FF9800" id="btn_about" />
    <text x="1.70" y="0.45" z="-2.94" value="About" size="0.07" />

    <box x="1.70" y="0.16" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#9C27B0" id="btn_settings" />
    <text x="1.70" y="0.16" z="-2.94" value="Settings" size="0.07" />

    <box x="1.70" y="-0.13" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#795548" id="demo_query" />
    <text x="1.70" y="-0.13" z="-2.94" value="Query Test" size="0.07" />

    <box x="1.70" y="-0.42" z="-3.0" sx="0.62" sy="0.20" sz="0.05" color="#546E7A" id="demo_wave" />
    <text x="1.70" y="-0.42" z="-2.94" value="Ring Arrange" size="0.07" />

    <!-- Status bar -->
    <text x="0" y="-0.78" z="-2.96" value="Status: ready" size="0.085" id="demo_status" color="#FFFFFF" />
    <text x="0" y="-0.96" z="-2.96" value="Dynamic nodes: 0" size="0.075" id="demo_count" color="#B0BEC5" />
    <text x="0" y="-1.12" z="-2.96" value="Animation: off" size="0.075" id="demo_anim" color="#B0BEC5" />
    <text x="0" y="-1.28" z="-2.96" value="Objects scale 1:10 of distance — spawns appear around you" size="0.063" id="demo_hint" color="#546E7A" />

  <script>
    const root = hiperspace.dimention;

    // Each entry: { el, baseX, baseY, baseZ }
    const dynamicNodes = [];
    let nextId = 1;
    let animating = false;
    let bannerMoved = false;
    let floorVisible = true;

    // ── helpers ──────────────────────────────────────────────────────────────

    function byId(id) { return root.getElementById(id); }

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
      setText('demo_anim',  'Animation: ' + (animating ? 'on' : 'off'));
    }

    function randomColor() {
      const palette = [
        '#F44336','#E91E63','#9C27B0','#673AB7',
        '#3F51B5','#2196F3','#00BCD4','#009688',
        '#4CAF50','#CDDC39','#FF9800','#FF5722',
      ];
      return palette[Math.floor(Math.random() * palette.length)];
    }

    // ── scale-aware spawn ─────────────────────────────────────────────────────
    //  Rule: size = distance × 0.1
    //    3.5 m  →  0.35 m  (tiny, near your shins)
    //   10   m  →  1.0  m  (life-sized box)
    //   30   m  →  3.0  m  (huge structure, but far away)

    function spawnAround(tag) {
      const el = root.createElement(tag);

      const angle = Math.random() * Math.PI * 2;          // full 360°
      const dist  = 3.5 + Math.random() * 28.5;           // 3.5 – 32 m
      const scale = dist * 0.1;                            // proportional scale

      const bx = Math.sin(angle) * dist;
      const bz = -Math.cos(angle) * dist;
      // Vertical spread: ~40% stay near ground, ~60% float at varying heights
      // Max height scales with distance so far-away giants can sit high in the sky.
      const groundY = scale * 0.5;
      const lift    = Math.random() &lt; 0.4
        ? 0
        : Math.random() * (1.0 + dist * 0.25);             // up to ~1 m + 25% of distance
      const by = groundY + lift;

      el.id        = 'demo_dyn_' + nextId++;
      el.className = 'demo-dynamic';
      el.setAttribute('color', randomColor());

      if (tag === 'plane') {
        el.setAttribute('sx', String(scale * 2.0));
        el.setAttribute('sy', String(scale * 2.0));
        el.setAttribute('sz', '1');
        el.rotation = { x: -1.5708, y: Math.random() * Math.PI * 2, z: 0 };
      } else if (tag === 'cylinder') {
        el.setAttribute('sx', String(scale));
        el.setAttribute('sy', String(scale * 1.6));
        el.setAttribute('sz', String(scale));
        el.rotation = { x: 0, y: Math.random() * Math.PI * 2, z: 0 };
      } else {
        el.setAttribute('sx', String(scale));
        el.setAttribute('sy', String(scale));
        el.setAttribute('sz', String(scale));
        el.rotation = {
          x: Math.random() * 0.4,
          y: Math.random() * Math.PI * 2,
          z: Math.random() * 0.4,
        };
      }

      el.position = { x: bx, y: by, z: bz };
      root.appendChild(el);
      dynamicNodes.push({ el, bx, by, bz });
      updateCounters();
      setStatus('spawned ' + tag + ' at ' + dist.toFixed(1) + ' m — scale ' + scale.toFixed(2) + ' m — y ' + by.toFixed(2));
    }

    function spawn(tag, amount) {
      for (let i = 0; i < amount; i++) spawnAround(tag);
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
      const t = ts / 1000;
      dynamicNodes.forEach(({ el, bx, by, bz }, i) => {
        const off = i * 0.38;
        el.rotation = {
          x: Math.sin(t * 0.5 + off) * 0.12,
          y: t * 0.9 + off,
          z: Math.sin(t * 0.7 + off) * 0.12,
        };
        el.position = {
          x: bx,
          y: by + Math.sin(t * 1.4 + off) * (by * 0.25),
          z: bz,
        };
      });
      requestAnimationFrame(animateFrame);
    }

    function toggleAnimation() {
      animating = !animating;
      updateCounters();
      setStatus(animating ? 'animation on' : 'animation off');
      if (animating) requestAnimationFrame(animateFrame);
    }

    function moveBanner() {
      const panel = byId('demo_panel');
      if (!panel) return;
      bannerMoved = !bannerMoved;
      panel.position = bannerMoved
        ? { x: 0.45, y: 0.70, z: -3.15 }
        : { x: 0.00, y: 0.50, z: -3.15 };
      panel.rotation = bannerMoved
        ? { x: 0, y: 0.12, z: 0 }
        : { x: 0, y: 0.00, z: 0 };
      setStatus(bannerMoved ? 'panel nudged' : 'panel reset');
    }

    function toggleFloor() {
      const floor = byId('demo_ground');
      if (!floor) return;
      floorVisible = !floorVisible;
      floor.setAttribute('visible', floorVisible ? 'true' : 'false');
      setStatus(floorVisible ? 'floor visible' : 'floor hidden');
    }

    function queryTest() {
      const all   = root.getElementsByClass('demo-dynamic');
      const first = all.length ? all[0].id : 'none';
      setText('demo_hint', 'Found ' + all.length + ' dynamic — first: ' + first);
      setStatus('query complete');
    }

    // Arrange all dynamic nodes into a scale ring (same rule: size ∝ distance)
    function ringArrange() {
      const n = dynamicNodes.length;
      if (!n) { setStatus('nothing to arrange'); return; }
      dynamicNodes.forEach((item, i) => {
        const angle = (i / n) * Math.PI * 2;
        const ring  = i % 5;
        const dist  = 5 + ring * 5;             // rings: 5, 10, 15, 20, 25 m
        const scale = dist * 0.1;
        const nx = Math.sin(angle) * dist;
        const nz = -Math.cos(angle) * dist;
        // Stagger heights per ring so outer rings float higher.
        const ny = scale * 0.5 + ring * 1.5;    // 0, 1.5, 3, 4.5, 6 m above ground
        item.bx = nx; item.by = ny; item.bz = nz;
        item.el.position = { x: nx, y: ny, z: nz };
        item.el.rotation = { x: 0, y: angle + Math.PI, z: 0 };
      });
      setStatus('arranged ' + n + ' nodes in scale ring');
    }

    // ── bindings ──────────────────────────────────────────────────────────────

    const bindings = {
      demo_spawn_box:      () => spawn('box', 1),
      demo_spawn_sphere:   () => spawn('sphere', 1),
      demo_spawn_cylinder: () => spawn('cylinder', 1),
      demo_spawn_plane:    () => spawn('plane', 1),
      demo_stress:         () => spawn('box', 25),
      demo_recolor:        recolorDynamic,
      demo_toggle_anim:    toggleAnimation,
      demo_move_banner:    moveBanner,
      demo_toggle_floor:   toggleFloor,
      demo_clear:          clearDynamic,
      demo_query:          queryTest,
      demo_wave:           ringArrange,
      btn_home:     () => { location.href = 'luna://home'; },
      btn_about:    () => { location.href = 'luna://about'; },
      btn_settings: () => { location.href = 'luna://settings'; },
    };

    Object.keys(bindings).forEach((id) => {
      const el = byId(id);
      if (el) el.addEventListener('toque', bindings[id]);
    });

    updateCounters();
    setStatus('ready — look around to see the scale ring');
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
    fn root_api_has_switch_mode() {
        let content = VIRTUAL_ROUTES
            .resolve("luna://internal/root_api.js")
            .unwrap();
        assert!(content.contains("switchMode"));
        assert!(content.contains("luna://ux_desktop"));
        assert!(content.contains("luna://ux_vr"));
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
}
