# Embedded Apps en UX Shell — Phase 2 plan

## Contexto

Phase 1 (implementado) introduce dos tipos de tabs a nivel UX:

- **Spatial**: al abrir cierra otras spatials. Como "moverse a otro lugar". Único activo a la vez (excluyendo el shell).
- **Application**: aditivo, persiste entre spatials. Cada app es un root space hijo de `luna_root`, igual que una spatial a nivel engine.

A nivel **engine** ambas son lo mismo: spaces montados por `dimension.luna.mountSpace`. La distinción vive en el JS del shell (`root_api.js` mantiene el registry con `entry.kind`).

## Lo que queda para Phase 2

Un tercer tipo: **App embedded dentro del UX shell**.

Concepto: apps que renderizan no como root space hermano, sino como **subnodos dentro del space del shell** (ej. `<group id="ux_vr_embedded_apps">`). Esto les da:

- API extendida hacia ux_vr (postMessage, ventanas flotantes, acceso a estado del shell).
- Anclaje al usuario (ux_vr se reposiciona enfrente al toggle — apps embedded se mueven con él).
- Capacidad de renderizar "ventanas" como YouTube client, chat, notificaciones — composables dentro de la metáfora del shell.

## Arquitectura propuesta

### Jerarquía esperada

```
luna_root
├── ux_vr (space, managed-by=dimension.luna)
│   ├── ux_vr_panel        (bookmarks, ya existe)
│   └── ux_vr_embedded     (NUEVO — host de apps embedded)
│       ├── <space role="application-embedded">chat app</space>
│       └── <space role="application-embedded">notifications</space>
├── spatial_tab_home       (managed-by=dimension.luna)
└── application_chat       (managed-by=dimension.luna, opcional si la app es standalone)
```

Una misma URL puede abrirse como standalone (root tab) **o** embedded — diferentes
APIs, diferentes contextos.

### API JS

```js
// Phase 1 — ya implementado:
dimention.tabs.open(url, { kind: 'spatial' });
dimention.tabs.open(url, { kind: 'app' });

// Phase 2 — pendiente:
dimention.tabs.openEmbedded(url, { slot: 'left' | 'right' | 'overlay' });
```

`openEmbedded` sólo callable por UX shell (managed-by=dimension.luna). Otras apps
no pueden montar embedded — sólo el shell decide qué embeber.

### Cap nueva: `ux_embed`

Concedida a apps embedded por el shell al spawnearlas. Otorga:

- **Comunicación bidireccional con el shell**: `dimention.shell.postMessage({...})` + listener `'shellmessage'`.
- **Posicionamiento relativo al shell**: `dimention.shell.requestSlot('overlay-top')` → shell asigna coords.
- **Suspensión cooperativa**: cuando el user cierra el shell, las apps embedded reciben `suspend` event; al reabrir, `resume`.

Apps standalone con `kind='app'` (Phase 1) NO reciben `ux_embed` automáticamente.
Pueden pedirla pero el shell decide si la aceptan.

### Window manager visual

UX shell mantiene un layout para apps embedded:

```
┌─────────────────────────────────────────┐
│  bookmarks panel (centro)               │
├─────────────────────────────────────────┤
│ ┌──── overlay ────┐    ┌─── notifs ───┐ │
│ │ chat window     │    │ notif item 1 │ │
│ │ youtube player  │    │ notif item 2 │ │
│ └─────────────────┘    └──────────────┘ │
└─────────────────────────────────────────┘
```

Slots predefinidos:
- `overlay-left` / `overlay-right`: paneles flotantes al costado del shell.
- `notif-stack`: stack vertical de notificaciones cortas.
- `media-controls`: barra inferior con play/pause/etc.
- `dock`: barra horizontal de iconos de apps embedded.

Apps piden slot; shell asigna posición. Si slot lleno, queue o reemplazo.

### Comunicación shell ↔ embedded

`postMessage`-style. Implementación posible:

```js
// En embedded app
dimention.shell.postMessage({ type: 'request-attention', urgency: 'low' });
dimention.shell.on('message', (msg) => { ... });

// En shell (ux_vr)
dimention.embedded.broadcast({ type: 'theme-changed', value: 'dark' });
for (const app of dimention.embedded.list()) {
  app.sendMessage({ ... });
}
```

Implementación: bridge en el host (luna js.rs) que rutea mensajes entre workers
del shell y de los embedded apps. Permisos: ambos lados validan `ux_embed`.

### Lifecycle

- **Mount**: shell decide cuándo. Puede ser por click en bookmark con `kind='app-embedded'`,
  o por API explícita del usuario (`openEmbedded`).
- **Suspend**: cuando el shell se oculta (menu press → hide), embedded apps reciben suspend.
  Sus loops (`requestAnimationFrame`, polling) deben pausar. Si no lo hacen, host puede
  forzar suspension del worker tras N segundos sin shell visible.
- **Resume**: al reabrir el shell.
- **Unmount**: shell remueve app (o app pide self-unmount).

### Suspension policy (importante)

Apps embedded que persisten cross-spatial podrían generar consumo CPU constante
(animaciones, polling). Para evitar drain:

- Shell pausa apps embedded cuando él mismo no está visible.
- Apps pueden declarar `keep-alive` en su HSML para excluirse del pause (ej. música).
- Limit: max N apps con keep-alive activo simultáneo.

## Integración con Phase 3 (caller + destination reconciliation)

Phase 3 plantea que el destination HSML declare su rol (`<space role="...">`).
Para embedded apps, el destination puede declarar `role="application-embedded"`
para indicar que está diseñada para correr embedded (UI compacta, sin
necesidad de world space propio).

Si un destination `application-embedded` se intenta abrir como `kind='spatial'`,
el shell puede:
- Warnear y abrirla embedded igual.
- O abrirla spatial pero con UI reducida.

Decisión por documento (qué hace si el caller pide algo distinto a su rol natural).

## Riesgos / open questions

1. **Aislamiento de workers**: cada space ya tiene su V8 isolate. Bridge postMessage
   debe ser barato y no requerir serialización pesada.
2. **Z-fighting / oclusión**: embedded apps están en el plano del shell; las
   ventanas tendrán que ofsettearse en Z para no chocar con bookmarks.
3. **Permission prompts**: si un app embedded pide caps elevadas (camera, mic),
   el shell debe poder mostrar prompt al user (mismo gate que para apps root).
4. **Identidad cruzada**: app embedded puede saber que está dentro del shell
   vs standalone? Probablemente sí — `dimention.context.embedded === true`.
5. **Hot-swap**: ¿se puede mover una app embedded → root (popout)? Útil para
   videos full-screen. Requiere preservar estado del worker (no recrear isolate).

## Estimación

~1-2 días de trabajo. Bloqueante: necesita la infra de Phase 1 estable.

## Phase 1 dependencies

- ✅ `dimention.tabs.open(url, { kind })` con kind propagado.
- ✅ `root_api.js` registry incluye `kind` por entry.
- ✅ Spatials se cierran entre sí; apps persisten.
- ⏳ `<space role="...">` declarado en HSML de cada doc luna (caller + destination ready).

Cuando todo lo de arriba esté estable, arrancar con esta fase.
