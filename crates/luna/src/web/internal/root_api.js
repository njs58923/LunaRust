// luna://internal/root_api.js
// API privilegiada del root space — `dimension.luna.*` permite montar tabs
// hijas del luna_root (mountSpace/unmountSpace/updateSpace/listMountedSpaces)
// y conmutar UX shell (switchMode 'vr'|'desktop').
//
// Se auto-inyecta vía el bundle "root" (ver permissions.rs).

(function (global) {
  const root = global.hiperspace && global.hiperspace.dimention;
  if (!root) {
    console.error('[luna://root] Missing root space');
    return;
  }

  const dimension = global.dimension || (global.dimension = {});
  const registry = new Map();
  let nextPublicId = 1;
  const nativePages = new Set([
    'home', 'demos', 'settings', 'about', 'cache-stats', 'error/404',
    'scale_demo', 'fire_demo', 'target_demo', 'range_demo',
  ].map(path => 'luna://' + path));
  let environment = null;
  let environmentVisible = false;

  // A root-owned sibling, never a tab or a child of the current document.
  // Read the include's current URL: self-navigation does not call mountSpace.
  function syncNativeEnvironment() {
    let visible = false;
    for (const [id, entry] of registry) {
      if (id === dimension.luna._uxSpaceId || entry.kind !== 'spatial') continue;
      if (!isDirectRootChild(entry.space)) continue;
      if (entry.space.getAttribute('visible') === 'false') continue;
      const include = entry.include || findPrimaryInclude(entry.space);
      if (include && nativePages.has(include.getAttribute('src'))) {
        visible = true;
        break;
      }
    }
    if (visible && !environment) {
      environment = root.createElement('group');
      environment.id = 'luna_native_environment';
      environment.setAttribute('visible', 'false');
      root.appendChild(environment);
      const include = root.createElement('include');
      include.setAttribute('src', 'luna://environment');
      environment.appendChild(include);
    }
    if (environment && visible !== environmentVisible) {
      environment.setAttribute('visible', visible ? 'inherit' : 'false');
      environmentVisible = visible;
    }
  }

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

  function registerSpace(space, kind) {
    for (const [publicId, entry] of registry) {
      if (entry.space.nodeId === space.nodeId) {
        if (!entry.include) {
          entry.include = findPrimaryInclude(space);
        }
        // Si re-discover sin kind explícito, no pisar lo que ya teníamos.
        if (kind && !entry.kind) {
          entry.kind = kind;
        }
        return publicId;
      }
    }

    const publicId = nextPublicId++;
    registry.set(publicId, {
      space,
      include: findPrimaryInclude(space),
      // kind opaco. Convenciones: "spatial" | "app". Default spatial.
      // Spaces descubiertos via discoverDirectSpaces() (sin pasar por mountSpace)
      // se asumen spatial hasta que algo declare lo contrario.
      kind: kind || 'spatial',
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

    if (opts.tabId != null) {
      space.setAttribute('data-luna-tab-id', String(opts.tabId));
    }

    if (opts.visible != null) {
      space.setAttribute('visible', opts.visible ? 'inherit' : 'false');
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
      tabId: entry.space.getAttribute('data-luna-tab-id') || '',
      url: include ? include.getAttribute('src') : '',
      visible: entry.space.getAttribute('visible') !== 'false',
      loaded: include ? include.children.length > 0 : entry.space.children.length > 0,
      position: cloneVec3(entry.space.position, { x: 0, y: 0, z: 0 }),
      rotation: cloneVec3(entry.space.rotation, { x: 0, y: 0, z: 0 }),
      scale: cloneVec3(entry.space.scale, { x: 1, y: 1, z: 1 }),
      title: entry.space.getAttribute('title') || '',
      kind: entry.kind || 'spatial',
    };
  }

  dimension.luna = {
    _uxSpaceId: null,
    _currentMode: null,

    mountSpace(url, options = {}) {
      discoverDirectSpaces();
      cleanupRegistry();

      // ── Kind & policy ──────────────────────────────────────────────────
      // "spatial" (default): cierra otras spatial antes de montar.
      // "app":               aditiva, no cierra nada.
      // "app-embedded":      aditiva, recibe cap ux_embed para hablar con shell.
      // ───────────────────────────────────────────────────────────────────
      let kind = 'spatial';
      if (options.kind === 'app' || options.kind === 'app-embedded') {
        kind = options.kind;
      }

      // Qué spatial hay ya montadas. Sólo se junta la lista: qué hacer con
      // ellas se decide más abajo, cuando ya se calcularon los grants.
      const spatialesPrevias = [];
      if (kind === 'spatial' && !options.systemShell) {
        for (const [id, entry] of registry) {
          if (id === this._uxSpaceId) continue;
          // Las managed-by=dimension.luna que no son el shell son tabs
          // spatiales o apps que abrimos nosotros — las distingue entry.kind.
          if ((entry.kind || 'spatial') === 'spatial') spatialesPrevias.push(id);
        }
      }

      // Default grants por kind si el caller no especificó.
      let grants = options.grants;
      if (!Array.isArray(grants)) {
        if (kind === 'spatial') {
          // read_camera_pose da la posición del visitante, no su mirada. Va
          // por defecto porque read_pose_stream ya la entrega igual vía la
          // pose de los mandos: negarla sólo rompía escritorio, donde no hay
          // mandos, sin proteger nada.
          grants = ['navigate_self', 'navigate_world', 'read_pose_stream', 'read_camera_pose', 'skybox', 'fetch_text', 'fetch_http', 'spawn', 'audio'];
        } else if (kind === 'app') {
          grants = ['navigate_self', 'fetch_text', 'audio'];
        } else if (kind === 'app-embedded') {
          grants = ['navigate_self', 'ux_embed', 'fetch_text', 'audio'];
        }
      }

      const initialVisible = options.visible !== false && kind !== 'app-embedded';

      // **Una spatial reusa la que ya está, no la reemplaza.**
      //
      // Antes se desmontaba la anterior y se creaba un espacio nuevo: el mundo
      // cambiaba igual, pero cada vez con otra pestaña. Cruzar una puerta del
      // atrio no hace eso — desde la página, `location.href` se resuelve como
      // `SelfNav` y el motor le cambia el `src` al include de esa misma
      // pestaña, así que la escena se reemplaza **en el lugar**. Acá se hace lo
      // mismo a mano, que es todo lo que `SelfNav` hace: cambiarle la URL al
      // include.
      //
      // Se conserva el `tabId` viejo a propósito. El host ya reservó uno nuevo
      // para este pedido y queda sin usar —un hueco en la numeración—, pero eso
      // es más barato que romper la identidad de la pestaña, que es justamente
      // lo que se venía a arreglar.
      if (spatialesPrevias.length) {
        const reusarId = spatialesPrevias[0];
        const previa = registry.get(reusarId);
        // Si por alguna razón hubiera más de una, el resto se cierra: la regla
        // de que hay una sola spatial a la vez sigue valiendo.
        for (let i = 1; i < spatialesPrevias.length; i++) this.unmountSpace(spatialesPrevias[i]);

        if (previa && previa.space) {
          console.log('[root] mountSpace spatial', url, '— reusando tab', reusarId);
          const inc = ensureInclude(previa);
          if (Array.isArray(grants) && grants.length) {
            previa.space.setAttribute('resources', grants.join(','));
            inc.setAttribute('resources', grants.join(','));
          }
          applySpaceOptions(previa, {
            ...options,
            url,
            tabId: null,   // la pestaña es la misma: no se le pisa el id
            visible: initialVisible,
          });
          return reusarId;
        }
        // La entrada estaba rota: se cierra y se sigue por el camino normal.
        this.unmountSpace(reusarId);
      }

      const space = root.createElement('space');
      const publicId = registerSpace(space, kind);
      const entry = registry.get(publicId);
      if (!entry) return -1;

      space.setAttribute('visible', initialVisible ? 'inherit' : 'false');
      space.setAttribute('managed-by', 'dimension.luna');
      space.setAttribute('data-luna-kind', kind);
      if (options.systemShell) {
        // El outer wrapper es el "anchor" del shell — find_system_shell_space
        // lo encuentra y baja al primer space descendiente (el inner del HSML
        // del shell, donde corre el script ux_vr / ux_desktop).
        space.setAttribute('system-shell', 'true');
      }

      if (Array.isArray(grants) && grants.length) {
        space.setAttribute('resources', grants.join(','));
      }

      root.appendChild(space);
      console.log('[root] mountSpace url=', url, 'kind=', kind,
        'grants=', Array.isArray(grants) ? grants.join(',') : '(none)');
      const include = ensureInclude(entry);
      if (Array.isArray(grants) && grants.length) {
        include.setAttribute('resources', grants.join(','));
      }
      applySpaceOptions(entry, { ...options, url, visible: initialVisible });
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

    /// Análogo a `setSpaceVisible` pero usa el `tab_id` (data-luna-tab-id) en
    /// lugar del publicId interno. Lo usa el host bridge cuando llega un
    /// `tabs.setVisible(tabId, ...)` desde el shell.
    setSpaceVisibleByTabId(tabId, visible) {
      discoverDirectSpaces();
      cleanupRegistry();
      const tidStr = String(tabId);
      for (const [pid, entry] of registry) {
        if (entry.space.getAttribute('data-luna-tab-id') === tidStr) {
          entry.space.setAttribute('visible', visible ? 'inherit' : 'false');
          return true;
        }
      }
      console.warn('[root] setSpaceVisibleByTabId: tab_id=' + tabId + ' not found');
      return false;
    },

    /// Setea pose del outer wrapper de una tab. Aplicado directamente acá
    /// (no depende de que la app aplique pose en su isolate — evita race).
    setSpacePoseByTabId(tabId, position, rotation) {
      discoverDirectSpaces();
      cleanupRegistry();
      const tidStr = String(tabId);
      for (const [pid, entry] of registry) {
        if (entry.space.getAttribute('data-luna-tab-id') === tidStr) {
          if (position) entry.space.position = cloneVec3(position, { x: 0, y: 0, z: 0 });
          if (rotation) entry.space.rotation = cloneVec3(rotation, { x: 0, y: 0, z: 0 });
          return true;
        }
      }
      console.warn('[root] setSpacePoseByTabId: tab_id=' + tabId + ' not found');
      return false;
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
      // Tiene que incluir read_camera_pose o cambiar de modo se lo saca a
      // todo lo montado: justo el caso donde más falta hace, porque en
      // escritorio es la única fuente de posición que hay.
      const grants = ['navigate_self', 'read_pose_stream', 'read_camera_pose'];

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

    switchMode(mode, style = this._controllerStyle || 'curved') {
      if (mode !== 'desktop' && mode !== 'vr') {
        console.error('[root] Invalid mode:', mode);
        return false;
      }
      if (style !== 'curved' && style !== 'flat') return false;
      const grants = mode === 'vr'
        ? ['navigate_self', 'vr_locomotion', 'read_system_input', 'manage_tabs', 'read_hmd_pose']
        : ['navigate_self', 'desktop_camera_control', 'read_system_input', 'manage_tabs', 'read_hmd_pose'];
      const existing = registry.get(this._uxSpaceId);
      if (existing && style === this._controllerStyle) {
        if (mode !== this._currentMode) {
          existing.space.setAttribute('resources', grants.join(','));
          ensureInclude(existing).setAttribute('resources', grants.join(','));
        }
        this._currentMode = mode;
        return true;
      }
      // Remove through the registry too: a just-mounted wrapper may not yet
      // appear in the asynchronous root.children snapshot.
      if (existing) this.unmountSpace(this._uxSpaceId);

      // Sweep defensivo en el DOM. Las ops mount/unmount son async, así que
      // un switchMode previo puede dejar wrappers zombi. Busca direct children
      // del root con `system-shell="true"` y los elimina del DOM + registry.
      // (`find_system_shell_space` en Rust también prefiere el más reciente
      // como red de seguridad si este sweep no llegó a tiempo.)
      discoverDirectSpaces();
      cleanupRegistry();
      for (const child of [...root.children]) {
        if (normalizeTag(child.tagName) !== 'space') continue;
        if (child.getAttribute('system-shell') === 'true') {
          for (const [id, entry] of [...registry.entries()]) {
            if (entry.space.nodeId === child.nodeId) {
              registry.delete(id);
            }
          }
          try { child.remove(); } catch (_) {}
        }
      }
      this._uxSpaceId = null;

      const uxUrl = style === 'curved' ? 'luna://ux_vr' : 'luna://ux_desktop';
      this._uxSpaceId = this.mountSpace(uxUrl, { visible: true, grants, systemShell: true });
      this._currentMode = mode;
      this._controllerStyle = style;
      console.log('[root] ux mounted id=', this._uxSpaceId, 'url=', uxUrl);
      return true;
    },
  };

  // The host supplies mode and persisted style together before mounting the shell.

  // Poll only the small tab registry, not the page DOM. No writes while stable.
  // Keeping the same loaded include also preserves the environment across
  // native navigation and desktop/VR shell changes, without touching the viewer.
  function maintainEnvironment() {
    syncNativeEnvironment();
    requestAnimationFrame(maintainEnvironment);
  }
  requestAnimationFrame(maintainEnvironment);

  console.log('[luna://root] dimension.luna ready');
})(globalThis);
