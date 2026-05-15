# Embedded Apps en UX Shell — Diseño completo

## Estado del documento

Plan vivo. Refleja el modelo acordado para el ciclo de vida y la interacción
de las "apps embedded" dentro del shell VR (ux_vr) — análogo a las apps
flotantes de Quest 2 home y a la barra de tareas de Windows.

Implementación dividida en commits separados (ver sección "Roadmap" al final).

---

## Contexto y problema

Phase 1 de tabs ya distingue:

- **Spatial**: al abrir cierra otras spatial — "moverse a otro lugar".
- **App**: aditiva, persiste cross-spatial.

A nivel **engine** ambas son spaces hermanos hijos de `luna_root`. La distinción
vive en `root_api.js` (registry con `entry.kind`).

Lo que falta: **apps que renderizan dentro del shell con UI gestionada por él**
(frame, titlebar, botones, layout). Caso de uso central: una página de
**Settings** estilo Quest con paneles y secciones, donde el shell aporta los
chrome controls (close, minimize, anchor) y la app sólo dibuja contenido.

---

## Modelo conceptual

### Tres ejes ortogonales para una app

| Flag | Significado | Cómo se llega |
|------|-------------|---------------|
| `focused` | Ocupa primer plano. Oculta el menú mientras dura. Máximo 1 en todo el shell. | Default al abrir un bookmark `app-embedded` |
| `anchored` | Flota libre en el aire. Sigue el ciclo de visibilidad del menú. | Drag fuera de focus, o botón "anchor" en titlebar |
| `always_on` | Visible incluso cuando el menú está oculto. Implica anchored. | Toggle explícito en titlebar de anchored |

### Reglas globales

1. **Solo 1 focus** a la vez. Si se abre otra app `app-embedded` mientras hay
   focus, la anterior se **cierra** (unmount). Para preservar: el user debe
   anchored primero.
2. **N anchored** sin límite (la UX puede saturarse, pero no hay cap por ahora).
3. Si `focus.visible == true` → bookmarks y bottom bar ocultos (foco absoluto).
4. Spatial swap **no afecta** apps embedded (apps son extensiones del usuario).
5. Anchored son **siempre interactivas** mientras estén visibles.

---

## Bottom bar (taskbar tipo Windows/Quest)

### Estructura

```
[🏠 Menu]  [📱 Focus]  [🪟 Anc1 ✕]  [🪟 Anc2 ✕]  ...  [🪟 AncN ✕]
```

### Slots

| Slot | Condición | Click body | Click ✕ |
|------|-----------|------------|---------|
| 1: **Menu** | Siempre (si bar visible) | Si focus visible: minimize + bookmarks. Sino: no-op. | — |
| 2: **Focus indicator** | Existe `focus_app` (visible o minimizado) | Restaura focus (minimized=false), oculta bookmarks | — |
| 3..N: **Anchored entries** | Una por cada anchored | Toggle minimized de esa anchored | Unmount tab |

### Visibilidad de la bar

```
bottom_bar.visible = bookmarks_visible
                  && (focus_app.is_some() || !anchored.is_empty())
```

- Sin nada open → solo bookmarks limpios, bar oculta.
- Algo open + bookmarks visibles → bar visible.
- Bookmarks ocultos (focus activo o menu off) → bar oculta.

---

## Titlebar de cada ventana

Cada ventana (focus o anchored) tiene su propia titlebar. Layout:

```
[ Título de la app                  ]  [📌] [📍] [─] [✕]
```

- **Zona drag central**: hold + move para arrastrar la ventana (fase 2 — usa
  raw_input + posemove). Drag dentro de focus zone → focus. Fuera → anchored.
- **📌 Anchor**: solo aparece en focus. Toggle → pasa a anchored.
- **📍 Always_on**: solo aparece en anchored. Toggle del flag `always_on`.
- **─ Minimize**: visible=false, cierra/oculta UI pero mantiene mount.
- **✕ Close**: unmount completo.

### Visibilidad de la titlebar

- Focus: titlebar visible junto con focus.
- Anchored normal: titlebar visible cuando shell visible.
- Anchored `always_on`: titlebar visible **siempre que la app no esté minimizada**.

---

## State machine

### Datos del shell

```rust
ShellState {
  bookmarks_visible: bool,
  focus_app: Option<{
    tab_id,
    minimized: bool,
    position: Vec3,  // pose fija al frente
  }>,
  anchored: Vec<{
    tab_id,
    position: Vec3,
    rotation: Quat,
    minimized: bool,
    always_on: bool,
  }>,
}
```

### Reglas de visibilidad consolidadas

| Elemento | Visible cuando |
|----------|----------------|
| `bookmarks_panel` | `bookmarks_visible && (focus.is_none() \|\| focus.minimized)` |
| `bottom_bar` | `bookmarks_visible && (focus.is_some() \|\| !anchored.is_empty())` |
| `focus_app` content | `focus.is_some() && !focus.minimized` |
| Cada `anchored_i` content | `!anchored_i.minimized && (anchored_i.always_on \|\| bookmarks_visible)` |

### Transiciones principales

| Trigger | Pre | Post |
|---------|-----|------|
| Click bookmark `app-embedded` | bookmarks visible, no focus | focus = new app, focus.minimized=false, bookmarks hidden |
| Click bookmark cuando focus ya existe | bookmarks visible, focus minimizado | focus actual se **cierra (unmount)**, nuevo focus = new app |
| Click ✕ anchored (bottom bar) | (anchored existe) | Unmount tab, anchored removed |
| Click body anchored (bottom bar) | (anchored exists) | Toggle anchored.minimized |
| Click 🏠 Menu (bottom bar) | focus visible | focus.minimized=true, bookmarks visible |
| Click 🏠 Menu (bottom bar) | bookmarks visible | no-op |
| Click 📱 Focus restore (bottom bar) | bookmarks + focus.minimized=true | focus.minimized=false, bookmarks hidden |
| Click ✕ titlebar focus | focus visible | focus removed (unmount), bookmarks visible |
| Click ─ titlebar focus | focus visible | focus.minimized=true, bookmarks visible |
| Click 📌 anchor titlebar focus | focus visible | tab → anchored at current pose, focus=None, bookmarks visible |
| Click ✕ titlebar anchored | anchored visible | unmount tab |
| Click ─ titlebar anchored | anchored visible | anchored.minimized=true |
| Click 📍 always_on titlebar anchored | anchored visible | toggle always_on flag |
| Drag focus → fuera zona (fase 2) | focus visible + dragging | tab → anchored at drop pose, focus=None, bookmarks visible |
| Drag anchored → dentro zona (fase 2) | anchored visible + dragging | tab → focus, removed from anchored |
| Menu press (controller) | bookmarks visible | bookmarks hidden, bar hidden, anchored !always_on ocultos |
| Menu press | shell hidden (algo visible) | bookmarks visible, bar visible, anchored restaurados |
| Menu press | focus visible | focus.minimized=true, bookmarks visible (= mismo que 🏠) |

---

## API de comunicación shell ↔ app

### Bus bidireccional

- **App → shell**: ops Rust `op_shell_request_slot`, `op_shell_send_message`.
  El host valida cap `ux_embed`, encola y rutea al worker del shell.
- **Shell → app**: shell llama ops análogas; host encola al worker target.
- Implementación backend: resource `ShellMessageBus` en Bevy con `Vec<{from, to, payload}>`.
  Dispatcher cada tick.

### API JS expuesta a apps embedded (`web/internal/embedded_api.js`)

Auto-inyectada con cap `ux_embed` (bundle `embedded_app`).

```js
dimention.embedded = {
  context: {
    embedded: true | false,   // siempre true si esta API existe
    tab_id: 42,
    kind: 'app-embedded',
  },

  // Pide al shell un slot. El shell responde con event 'slot'.
  requestSlot({ minSize, preferredSize, title }),

  // Pide cerrar este tab (shell mandará 'close' event, luego unmount).
  closeSelf(),

  // Listener API
  on(eventType, callback),

  // Eventos posibles:
  //   'slot'    — recibe { position, rotation, size } cuando shell asigna/reasigna
  //   'suspend' — shell pidió pausar (loops, polling)
  //   'resume'  — shell pidió retomar
  //   'close'   — shell va a unmount; app debe cleanup en este callback
  //   'focus'   — app pasó a focus (visible en primer plano)
  //   'blur'    — app perdió focus (pasó a anchored o minimized)
};
```

### API JS expuesta al shell (`window_manager.js` interno a ux_vr)

```js
// Internas del shell — no expuestas como API global.
shell.assignSlot(tabId, slotConfig)
shell.sendMessage(tabId, payload)
shell.unmountTab(tabId)
```

---

## Permisos

### Cap `ux_embed` (elevated)

- Otorga acceso a `dimention.embedded.*`.
- Solo concedida automáticamente cuando un tab se monta con
  `kind: 'app-embedded'` desde un space `managed-by="dimension.luna"`.
- Apps no pueden auto-pedirla — el shell decide a quién dársela.

### Cap `window_manage` (elevated, futuro)

- Permite a un space gestionar slots del window manager (ej. agregar layouts
  custom, mover apps entre slots).
- Por ahora exclusiva del shell mismo (managed-by).

---

## Coordenadas del slot

- **Position**: world coords. Calculadas por el shell relativas a su propia
  pose en el momento de asignar. Si el shell se mueve (toggle off → on en otra
  pose), el shell envía un `slot` event actualizado a cada app focus/anchored
  no-`always_on`. `always_on` mantiene su pose original (fija en world).
- **Rotation**: quaternion. Apps focused respetan la rotation del shell;
  anchored mantienen la suya hasta que el user las arrastre (fase 2).
- **Size**: vec3 (ancho, alto, profundidad mínima usable). App puede pedir más
  vía `requestSlot({ size })`; shell otorga el máximo que pueda dar.

### Frame visual

Dibujado por el shell, no por la app:

- Borde sutil (color: `#0D1B2A`, levemente más oscuro que el panel).
- Titlebar arriba con título + botones.
- App dibuja **dentro del rectángulo del viewport**, coords relativas al
  centro recibido. Si la app pone algo fuera, queda sin frame.

---

## Z-order y oclusión

- Focus: al frente del user, plano principal.
- Anchored: pose definida por user (vía drag — fase 2 — o pose default al
  hacer anchor desde titlebar).
- Bottom bar: ligeramente offseteada al frente del bookmarks panel (Z más
  cercano) para no chocar.
- Anchored `always_on` cuando shell está hidden: queda flotando con su
  titlebar visible, sin nada más del shell renderizado.

---

## Persistencia y sesión

- **Cross-spatial**: apps embedded siguen al user al cambiar de spatial. Son
  extensiones del usuario, no del lugar.
- **Cross-mode (VR → desktop)**: si el shell switchea, las apps embedded
  pueden quedar montadas pero el window manager del nuevo modo decide cómo
  renderizarlas (futuro).
- **Persistencia entre sesiones**: no implementada. Cuando exista storage
  API, guardar lista de anchored + always_on.

---

## Widgets toolkit (fase separada)

Para que apps embedded (settings, about, futuras) sean ergonómicas, necesitan
componentes reutilizables. Estos viven en `web/internal/widgets.js` y se
auto-inyectan con un bundle `ui_widgets` (o quizás siempre — son inocuos).

### Lista mínima v1

| Widget | Estado | Estimación |
|--------|--------|------------|
| `widgets.button(opts)` | – | Trivial (ya hay boxes touchables) |
| `widgets.toggle(opts)` | – | Trivial (box + label + flip on click) |
| `widgets.slider(opts)` | – | Medio (drag o press +/− steps) |
| `widgets.section(label)` | – | Trivial (text + separador) |
| `widgets.label(text)` | – | Trivial (alias de text element) |

Diferidos:
- Dropdown
- Text input (depende del teclado virtual)
- Scroll panel (requiere clipping, no trivial)

---

## Roadmap de implementación

### Commit 1 — Infraestructura mínima ✅ (próximo)

- Cap `ux_embed` (elevated) + bundle `embedded_app` con auto-script
  `luna://internal/embedded_api.js`.
- `JsWorkerCommand::PushShellMessage` + dispatcher.
- Op JS `op_shell_request_slot`, `op_shell_send_message` (sin lógica de
  layout aún — solo bus).
- `dimention.embedded.*` API JS skeleton.
- Sin UI todavía. Validable con tests / demo simple.

### Commit 2 — ShellState + bottom bar + reorganización ux_vr

- ShellState struct en `ux_vr.hsml`.
- Bookmarks panel reposicionado arriba.
- Bottom bar render dinámico abajo (sin focus/anchored aún — solo Menu slot).
- Transiciones básicas de bookmarks_visible.

### Commit 3 — Focus app

- Click bookmark `app-embedded` → spawn como focus.
- Frame de focus visible (titlebar + viewport silhouette).
- Botones titlebar: close, minimize, anchor.
- Demo `luna://demo_embedded` que dibuja un box rojo dentro.

### Commit 4 — Anchored

- Click anchor en titlebar focus → tab pasa a anchored.
- Bottom bar muestra entries de anchored con ✕.
- Click body anchored bar → toggle minimize.
- Botón always_on en titlebar anchored.

### Commit 5 — Drag (fase 2 separable)

- Detección drag con raw_input + posemove.
- Drag focus → anchored (drop fuera).
- Drag anchored → focus (drop dentro de zone).
- Highlight visual de focus drop zone durante drag.

### Commit 6 — Widgets toolkit

- `web/internal/widgets.js` con button, toggle, slider, section, label.

### Commit 7 — Settings real

- Eliminar `LUNA_SETTINGS` actual.
- `web/apps/settings.hsml` declarando `<space role="application-embedded">`.
- Secciones: Appearance, Performance, Network.
- Bookmark `Settings` cambia a `kind: 'app-embedded'`.

---

## Riesgos / open questions

1. **Sincronización slot ↔ shell pose**: si shell se mueve, qué tan rápido
   responden las apps? Latencia perceptible? Mitigación: mensajear con
   prioridad alta, app interpola/anima.
2. **Race conditions** en menu press durante transición de animation. Guard
   "animation in flight, ignore menu press" — ya tenemos pattern.
3. **App freezea**: shell le manda 'close', no responde en 2s, shell unmount
   igual. Watchdog timer.
4. **Cap `ux_embed` filtración**: si una app legítimamente embedded re-monta
   otra app no embedded, transitividad. Política: el cap NO se hereda.
5. **Drag UX en VR**: pulgar en trigger + movimiento de brazo puede ser
   trabajoso. Posiblemente requiere gesture específica (grab con grip + mover
   mano). Decidir cuando llegue fase 2.

---

## Glosario

- **focus app / focused**: app activa en primer plano, oculta el menú.
- **anchored**: app flotando libre, sigue el ciclo del menú.
- **always_on**: anchored que sobrevive a menu hidden.
- **minimized**: visible=false pero mount preservado.
- **shell**: el space ux_vr que orquesta UI (bookmarks + bottom bar + windows).
- **focus zone**: rectángulo lógico donde la app focused se renderiza.
- **slot**: instancia de espacio donde una app embedded dibuja su contenido.
