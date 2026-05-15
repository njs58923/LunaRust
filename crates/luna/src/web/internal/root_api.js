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

      if (kind === 'spatial') {
        // Cerrar todas las spatial existentes ANTES de crear la nueva, para
        // evitar que la nueva entre transitoria al barrido si fuera ya child.
        const toUnmount = [];
        for (const [id, entry] of registry) {
          if (id === this._uxSpaceId) continue;
          // Las managed-by=dimension.luna que no son el shell son tabs spatiales
          // o apps que abrimos nosotros — distinguimos por entry.kind.
          if ((entry.kind || 'spatial') === 'spatial') {
            toUnmount.push(id);
          }
        }
        if (toUnmount.length) {
          console.log('[root] mountSpace spatial', url, '— closing', toUnmount.length, 'previous spatial(s)');
          for (const id of toUnmount) this.unmountSpace(id);
        }
      }

      // Default grants por kind si el caller no especificó.
      let grants = options.grants;
      if (!Array.isArray(grants) || !grants.length) {
        if (kind === 'spatial') {
          grants = ['navigate_self', 'read_pose_stream', 'skybox'];
        } else if (kind === 'app') {
          grants = ['navigate_self'];
        } else if (kind === 'app-embedded') {
          grants = ['navigate_self', 'ux_embed'];
        }
      }

      const space = root.createElement('space');
      const publicId = registerSpace(space, kind);
      const entry = registry.get(publicId);
      if (!entry) return -1;

      space.setAttribute('visible', options.visible === false ? 'false' : 'true');
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
      const grants = ['navigate_self', 'read_pose_stream'];

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
      if (mode !== 'desktop' && mode !== 'vr') {
        console.error('[root] Invalid mode:', mode);
        return false;
      }
      if (mode === this._currentMode) return true;

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

      const uxUrl = mode === 'vr' ? 'luna://ux_vr' : 'luna://ux_desktop';
      const grants = mode === 'vr'
        ? ['navigate_self', 'vr_locomotion', 'read_system_input', 'manage_tabs', 'read_hmd_pose']
        : ['navigate_self', 'desktop_camera_control', 'read_system_input', 'manage_tabs', 'read_hmd_pose'];
      this._uxSpaceId = this.mountSpace(uxUrl, { visible: true, grants, systemShell: true });
      this._currentMode = mode;
      console.log('[root] ux mounted id=', this._uxSpaceId, 'url=', uxUrl);
      return true;
    },
  };

  // Auto-mount desktop UX on startup
  dimension.luna.switchMode('desktop');

  console.log('[luna://root] dimension.luna ready');
})(globalThis);
