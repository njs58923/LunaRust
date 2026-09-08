# 🌙 Luna — Navegador Espacial 3D

**Luna es un navegador para la web espacial.** Así como un navegador tradicional
carga páginas HTML y ejecuta JavaScript en una ventana 2D, Luna carga *documentos
espaciales* y ejecuta JavaScript dentro de un mundo 3D que puedes recorrer en
**escritorio, VR o AR**.

Está construido sobre [Bevy](https://bevyengine.org/) (motor de juegos en Rust) y
OpenXR, con un runtime de JavaScript embebido y un DOM virtual que conecta el
scripting con el motor de render.

> **Nota:** Este proyecto es un fork de
> [`awtterpip/bevy_oxr`](https://github.com/awtterpip/bevy_oxr). El soporte base de
> OpenXR/WebXR para Bevy y todo su crédito pertenecen a los autores originales.
> Luna añade encima la capa de navegador espacial (HSML, runtime JS, espacios,
> permisos, locomoción, devtools).

> **¿Vas a escribir documentos para Luna?** Empezá por
> **[guides/index.md](guides/index.md)**, y sobre todo por
> [guides/trampas.md](guides/trampas.md), que es lo que falla en silencio.

---

## ¿Por qué un "navegador espacial"?

La idea central es trasladar el modelo mental de la web a 3D:

| Web tradicional | Luna |
|-----------------|------|
| Documento HTML | Documento **HSML** |
| `<div>`, `<img>`, `<button>` | `<box>`, `<model>`, `<text>`, `<skybox>`… |
| El DOM | DOM virtual sobre ECS |
| `https://…` | `luna://…` y `http://…` |
| Pestañas | **Espacios** (spatial / app / app-embedded) |
| Ventana del navegador | Tu sala en VR/AR o una ventana de escritorio |
| `<script>` + JS | `<script>` + runtime JS embebido |

Un documento HSML describe una escena; el JavaScript la hace interactiva. Navegar a
otra URL carga otra escena, igual que cambiar de página en la web.

---

## Conceptos clave

### Documentos HSML
HSML (HyperSpatial Markup Language) es un marcado tipo HTML para escenas 3D. Cada
etiqueta es un nodo de la escena con atributos de transformación (`x`, `y`, `z`,
rotación, escala) y propiedades propias:

```html
<hsml>
  <head>
    <meta name="title" content="Mi primer espacio" />
    <script src="app.js" />
  </head>
  <space>
    <skybox src="cielo.png" />
    <text x="0" y="1.6" z="-3" value="Hola, mundo espacial" size="0.2" />
    <model id="avatar" src="robot.glb" x="0" y="0" z="-2" />
    <box id="boton" x="0.5" y="1" z="-2" />
  </space>
</hsml>
```

Etiquetas disponibles: `space`, `group`, `box`, `sphere`, `cylinder`, `plane`,
`model`, `text`, `image`, `skybox`, `posezone`, `include`, `script`, `state`, entre
otras.

### Espacios y dimensiones
Cada documento cargado vive en un **espacio**. Luna admite varios espacios a la vez
(como pestañas), con distintos comportamientos:

- **spatial** — "moverte a otro lugar"; al abrir uno se cierran los demás spatial.
- **app** — aditivo, persiste aunque cambies de espacio (p. ej. un panel de ajustes).
- **app-embedded** — app renderizada dentro del shell, con su marco y controles.

### El protocolo `luna://`
Las páginas internas del navegador (home, ajustes, demos, about, estadísticas de
caché…) se sirven mediante un manejador de protocolo virtual `luna://`, sin
servidor externo. El contenido remoto se carga por `http://`/`https://`.

`luna://home` es el inicio predeterminado, con accesos a Explorar, Ajustes y
Acerca de. Las páginas internas comparten el islote de `luna://environment`
(`crates/luna/src/web/environment.hsml`), sin assets externos ni animación.
El root monta una sola instancia del entorno, fuera de las pestañas; la conserva
entre páginas nativas y la oculta para contenido externo. Los paneles y demos
usan el mismo origen y suelo en y=0. El suelo es visual por ahora, sin colisiones
ni límites de locomoción. Ver `guides/entorno.md`.

### Runtime de JavaScript
Cada espacio ejecuta su script en un contexto JS aislado. El runtime expone una API
tipo DOM (crear/mover/eliminar elementos, leer/escribir atributos y transformaciones,
jerarquía, `fetch`, timers, `requestAnimationFrame`) además de la API del navegador
`dimension.luna` para montar/desmontar espacios y cambiar de modo. La comunicación
JS ↔ motor usa un patrón **queue/snapshot**: JS nunca bloquea al render y viceversa.

Los documentos disponen de **localStorage persistente por origen**, compartido
entre sus isolates, con la API habitual de Web Storage y eventos `storage`.
Ver [almacenamiento](guides/almacenamiento.md) para ejemplos, cuotas y separación de datos.

Las primitivas aceptan **texturas** —con recorte por región para atlas, ajuste,
relleno y control de alfa— y el tag `<image>` expone la API de carga de
`HTMLImageElement` (`naturalWidth`, `onload`, `onerror`).
Ver [superficies](guides/superficies.md).

Los nodos `touchable` reciben **hover** con semántica de HTML —`pointerenter`,
`pointerleave`, `pointerover`, `pointerout` y `matches(':hover')`— con el cursor
en escritorio y con los rayos de los mandos en VR. Ver [eventos](guides/eventos.md).

### Modos de render
- **Escritorio** — ventana plana, navegación con teclado/ratón.
- **VR** — inmersión completa vía OpenXR (Quest, etc.), con locomoción y mandos.
- **AR** — passthrough con `--ar`.

---

## Arquitectura

Luna es un workspace de Rust dividido en crates. La capa XR proviene del fork
upstream; la capa de navegador es propia de Luna.

| Crate | Rol |
|-------|-----|
| **`luna`** | El navegador en sí: shell, pestañas/espacios, render, UI, devtools, locomoción, permisos. Binario principal. |
| **`virtual_dom`** | DOM virtual y parser HSML; mapea nodos del documento a entidades del motor. |
| **`js_runtime`** | Runtime JavaScript embebido, con caché HTTP y Content Security Policy (CSP). |
| **`dom`** | Tipos compartidos del modelo de documento. |
| `bevy_openxr` | Backend de OpenXR para Bevy *(upstream)*. |
| `bevy_webxr` | Backend WebXR *(upstream, en desarrollo)*. |
| `bevy_xr` | API XR semi-genérica para Bevy *(upstream)*. |
| `bevy_xr_utils` | Utilidades de tracking, manos, locomoción *(upstream)*. |

El binario corre como una app de Bevy: en cada frame sincroniza el DOM (creaciones,
borrados, cambios de atributos y transformaciones que el JS encoló), luego ejecuta el
tick de JS, y finalmente renderiza. El orden de estos sistemas es estricto para
mantener la coherencia entre lo que el script ve y lo que el motor dibuja.

---

## Estructura del repositorio

```
.
├── crates/
│   ├── luna/            # navegador (binario principal)
│   ├── virtual_dom/     # DOM virtual + HSML
│   ├── js_runtime/      # runtime JS + caché + CSP
│   ├── dom/             # tipos del documento
│   └── bevy_*/          # capa XR (fork upstream)
├── docs/                # notas de diseño e implementación
├── docker-compose.yml   # Caddy + Bun para servir contenido HSML
├── Caddyfile
└── package.json         # atajos de scripts (npm run …)
```

---

## Requisitos

- [Rust](https://rustup.rs/) (edición 2021).
- En Linux: el paquete del sistema `openxr`.
- Para VR: un runtime OpenXR (p. ej. Meta Quest Link, SteamVR, Monado).
- Opcional: [Node](https://nodejs.org/) para los atajos de `package.json`, y Docker
  para servir contenido HSML local.

---

## Cómo ejecutar

Con cargo directamente:

```bash
# Escritorio
cargo run -p luna

# Activar MCP local desde el arranque (sin cambiar la preferencia guardada)
cargo run -p luna -- --mcp

# Modo AR (passthrough)
cargo run -p luna -- --ar

# Release optimizado
cargo run -p luna --release

# Iteración rápida (compila más rápido, ~90% del rendimiento)
cargo run -p luna --profile release-fast
```

En `luna://settings`, «MCP al iniciar» guarda la preferencia de arranque
(`mcp_auto_start` en la configuración de Luna; desactivada por defecto).
En la ventana Config de escritorio también aparece; pulsar Guardar configuración
para persistirla. `luna.exe --mcp` fuerza la activación al arrancar, y se puede
detener desde Ajustes durante la sesión. La opción no inicia el adaptador MCP externo.

O con los atajos de `package.json`:

```bash
npm run luna            # escritorio
npm run luna:ar         # AR
npm run luna:release    # release
npm run build:release   # solo compilar
```

> En VR, asegúrate de tener el runtime OpenXR activo antes de lanzar. En escritorio,
> muévete con `WASD` + `Q`/`E` (subir/bajar) y mira con el ratón.

### Servir contenido HSML

Para cargar documentos remotos durante el desarrollo, el repo incluye un stack con
Caddy (proxy) y Bun:

```bash
docker compose up -d      # o: npm run docker
```

Luego, dentro de Luna, escribe la URL en la barra de direcciones (p. ej.
`http://localhost:2052/mi_espacio.hsml`) y navega.

---

## Scripting: API de JavaScript

Cada `<script>` corre en el contexto de su espacio. Resumen de la API:

```javascript
// Crear y componer la escena
const root = hiperspace.dimention;          // elemento raíz del espacio
const box = root.createElement('box');
box.setAttribute('name', 'mi-caja');
box.position.y = 1.5;                        // transform: position/rotation/scale
root.appendChild(box);

// Consultar e interactuar
const t = root.getElementById('boton');
t.addEventListener('toque', () => location.href = 'luna://home');

// Web APIs habituales
requestAnimationFrame(loop);
setTimeout(fn, 1000);
const res = await fetch('http://localhost:2052/data.json');

// API del navegador (espacios)
dimension.luna.mountSpace(url, { tabId, kind: 'spatial' });
dimension.luna.unmountSpace(id);
dimension.luna.listMountedSpaces();
dimension.luna.switchMode('vr');             // 'vr' | 'desktop'
```

El runtime aplica una **Content Security Policy** y una **caché HTTP** (ETag,
Last-Modified, `max-age`) a los recursos cargados, y un sistema de **permisos** por
espacio gobierna capacidades sensibles (leer la pose del visor, navegar, skybox, etc.).

Hay más detalle de diseño en [`docs/`](docs/).

---

## Estado del proyecto

Luna es un proyecto experimental en desarrollo activo. La base de navegador
(documentos HSML, runtime JS, espacios, navegación, permisos, locomoción VR/escritorio
y devtools) funciona; áreas como WebXR, apps embebidas y aislamiento multi-contexto
siguen evolucionando. Las APIs pueden cambiar.

---

## Documentación

| Dónde | Qué |
|---|---|
| **[guides/](guides/index.md)** | cómo **usar** la plataforma: escribir documentos, permisos, API JS, eventos, texturas, mallas |
| [guides/trampas.md](guides/trampas.md) | lo que falla en silencio — la página más útil de todas |
| [guides/disenos/](guides/disenos/) | propuestas y notas de diseño, no referencia de lo que existe |
| [docs/](docs/) | análisis, mediciones y estado de implementación |
| [docs/deuda_tecnica/](docs/deuda_tecnica/) | lo que se sabe que está mal y todavía no se arregló |
| `CORE_PERFORMANCE.md`, `ENGINE_DENSITY_OPTIMIZATION.md`, `PLATFORM_DIAGNOSTICS_FETCH.md` | trabajo sobre el motor, no sobre su uso |


## Licencia

Salvo que se indique lo contrario, todo el código de este repositorio tiene licencia
dual, a tu elección:

- Licencia MIT ([LICENSE-MIT](LICENSE-MIT) o http://opensource.org/licenses/MIT)
- Licencia Apache 2.0 ([LICENSE-APACHE](LICENSE-APACHE) o http://www.apache.org/licenses/LICENSE-2.0)

### Tus contribuciones

Salvo que declares explícitamente lo contrario, cualquier contribución que envíes
intencionalmente para su inclusión en el trabajo, según la define la licencia
Apache-2.0, será licenciada de forma dual como arriba, sin términos ni condiciones
adicionales.
