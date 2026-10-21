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

> **¿Querés verlo andando?** Bajá `luna.exe` de
> [Releases](https://github.com/njs58923/LunaRust/releases/latest) y abrí
> **`https://njs58923.github.io/luna-demos/index.hsml`** en la barra de
> direcciones: es una plaza con una puerta a cada demo. Los demos viven en
> **[njs58923/luna-demos](https://github.com/njs58923/luna-demos)** (ver
> [Demos](#demos)).

> **¿Vas a escribir documentos para Luna?** Empezá por
> **[guides/index.md](guides/index.md)**.

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

## Demos

**[njs58923/luna-demos](https://github.com/njs58923/luna-demos)** publica los
demos del taller como sitio estático en GitHub Pages. No hace falta levantar
ningún servidor: abrí en Luna

**https://njs58923.github.io/luna-demos/index.hsml**

| Demo | Qué es | Entrada |
|---|---|---|
| El atrio de noche | veintiocho escenas: cielos, cuevas, un tren, un bosque | [`noche/index.hsml`](https://njs58923.github.io/luna-demos/noche/index.hsml) |
| La nave | la Skeld con sus tareas, quince minijuegos y los conductos | [`nave/index.hsml`](https://njs58923.github.io/luna-demos/nave/index.hsml) |
| La calle de los objetos | más de cien objetos que andan, para llevar a tu mundo | [`expo/calle.hsml`](https://njs58923.github.io/luna-demos/expo/calle.hsml) |
| La sala de los ecos | un juego de memoria sonora, sin archivos de audio | [`eco/index.hsml`](https://njs58923.github.io/luna-demos/eco/index.hsml) |
| El framework de UI | ventanas, listas, combos y paneles al estilo WPF | [`ui/index.hsml`](https://njs58923.github.io/luna-demos/ui/index.hsml) |
| El taller de las manos *(VR)* | manipular objetos, un arco y una galería de tiro | [`manos/index.hsml`](https://njs58923.github.io/luna-demos/manos/index.hsml) |
| La planta *(VR)* | un juego corto con pistolas en una fábrica tomada por robots | [`planta/index.hsml`](https://njs58923.github.io/luna-demos/planta/index.hsml) |
| Sables de Luna *(VR)* | juego de ritmo tipo Beat Saber, con modo demo para escritorio | [`sables/index.hsml`](https://njs58923.github.io/luna-demos/sables/index.hsml) |
| Luna Stress | un test de carga por eje, hasta que el motor se cae | [`stress/index.hsml`](https://njs58923.github.io/luna-demos/stress/index.hsml) |

El taller de las manos y Sables usan [anclajes](guides/anclajes.md): necesitan
Luna v0.3.0 o posterior. Sables se puede mirar sin mandos en
[modo demo](https://njs58923.github.io/luna-demos/sables/index.hsml?jugar=tormenta&dif=dificil&demo=1).

Son también la mejor referencia para escribir documentos: cada demo es HSML y
JavaScript que se puede leer directamente en el
[repo](https://github.com/njs58923/luna-demos).
Los demos que dependen de un servidor vivo (física en red, WebSocket, mundos
infinitos) no entran en el volcado estático; el README de luna-demos explica
cuáles y por qué.

---

## Arquitectura

Luna es un workspace de Rust. La capa XR viene del fork upstream; el navegador
es propio.

| Crate | Rol |
|-------|-----|
| **`luna`** | El navegador (binario principal): shell y pestañas, render, UI, entrada (raycast, teclado, mandos), locomoción, permisos, audio, modelos y animación, mallas dinámicas, anclajes y el host del MCP local. |
| **`virtual_dom`** | Parser HSML y DOM virtual. |
| **`js_runtime`** | Runtime JavaScript sobre V8 (`deno_core`): APIs web (`fetch`, `location`, `localStorage`, `Audio`, `Blob`), componentes, mallas, anclajes, caché HTTP, CSP y caché de bytecode. |
| **`ui_graphics`** | Texto, glifos y curvas para las interfaces. |
| `bevy_openxr`, `bevy_xr`, `bevy_xr_utils` | OpenXR, API XR genérica y utilidades de tracking *(upstream)*. |
| `bevy_webxr` | Backend WebXR *(upstream, en desarrollo)*. |
| `lua_exps`, `rust_exps` | Experimentos; no forman parte del navegador. |

Fuera de `crates/`, [`tools/luna_mcp`](tools/luna_mcp/README.md) es el adaptador
MCP (stdio) que deja a un agente abrir páginas, capturar y leer logs de Luna.

### Cómo corre un documento

- **Un isolate por espacio.** Cada `<space>`, incluido cada `<include>`, tiene
  su propio isolate de V8 en su propio hilo (`js-space-N`), con sus permisos.
  Habla con el motor por colas acotadas: el render nunca espera al script, ni
  el script al render. Un único hilo vigila los tiempos de todos los isolates,
  y los documentos estáticos duermen hasta que algo los despierta.
- **DOM y motor.** El script muta su DOM; el host aplica esos cambios al DOM
  virtual y los refleja en entidades de Bevy. Los eventos (`toque`, hover,
  `posemove`, teclado) recorren el camino inverso, sólo hacia los espacios con
  permiso.
- **Trabajo fuera del hilo de render.** Normales, límites y huella de las
  mallas generadas se calculan en el hilo del isolate; la geometría se comparte
  entre instancias.
- **Poses en el mismo frame.** Después de propagar transforms y antes del
  culling, el motor coloca los elementos anclados y evalúa los programas de
  movimiento. Un objeto en la mano sigue al mando sin pasar por JavaScript.
- **Permisos por espacio.** Cada documento declara `resources`; el host concede
  la intersección con lo que permite su contexto, y un `<include>` sólo recibe
  lo que su padre le delega.

---

## Estructura del repositorio

```
.
├── crates/
│   ├── luna/            # navegador (binario principal)
│   │   └── src/web/     # páginas luna:// (home, ajustes, demos, shell)
│   ├── virtual_dom/     # parser HSML + DOM virtual
│   ├── js_runtime/      # V8, APIs web, caché, CSP
│   ├── ui_graphics/     # texto, glifos y curvas
│   └── bevy_*/          # capa XR (fork upstream)
├── guides/              # cómo escribir documentos para Luna
├── tools/luna_mcp/      # adaptador MCP para agentes
├── docker-compose.yml   # Caddy + servidores de demo para desarrollo
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
> muévete con `WASD` + `Q`/`E` (subir/bajar), corre hacia adelante con
> `Shift` + `W` y mira con el ratón. En VR, presiona el stick izquierdo mientras
> te mueves para correr.

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

La referencia completa de la API está en [`guides/`](guides/index.md).

---

## Estado del proyecto

Luna es un proyecto experimental en desarrollo activo. La base de navegador
(documentos HSML, runtime JS, espacios, navegación, permisos, locomoción VR/escritorio
y devtools) funciona; áreas como WebXR, apps embebidas y aislamiento multi-contexto
siguen evolucionando. Las APIs pueden cambiar.

---

## Documentación

Las guías de [`guides/`](guides/index.md) explican cómo escribir documentos que
corran en Luna. Están verificadas contra el código.

### Por dónde empezar

1. Probá los **[demos](#demos)** para ver qué se puede hacer.
2. Leé **[el documento mínimo](guides/index.md#lo-mínimo)** y el
   [modelo mental](guides/index.md#modelo-mental) en el índice de guías.
3. Seguí con [un ejemplo completo](guides/ejemplo.md): un documento y su
   script, enteros.
4. Cuando algo no ande y no haya error a la vista, revisá
   [trampas.md](guides/trampas.md).

### Según lo que quieras hacer

| Quiero… | Leé |
|---|---|
| Armar la escena: tags, atributos, jerarquía | [documento.md](guides/documento.md) |
| Pedir capacidades (navegar, leer la pose, audio…) | [permisos.md](guides/permisos.md) |
| Programar la escena: crear, mover y consultar nodos | [javascript.md](guides/javascript.md) |
| Responder a toques y hover | [eventos.md](guides/eventos.md) |
| Dividir en piezas reutilizables con `<include>` | [componentes.md](guides/componentes.md) |
| Hacer paneles, botones y controles | [framework-ui.md](guides/framework-ui.md), [interfaz.md](guides/interfaz.md) |
| Escribir con teclado físico o virtual | [teclado.md](guides/teclado.md) |
| Poner texturas e imágenes | [superficies.md](guides/superficies.md) |
| Cargar modelos glTF | [modelos.md](guides/modelos.md) |
| Animar modelos: clips o poses desde el script | [animaciones.md](guides/animaciones.md), [poses.md](guides/poses.md) |
| Que un objeto siga a los mandos sin latencia, o combinar dos manos | [anclajes.md](guides/anclajes.md), [programas-de-movimiento.md](guides/programas-de-movimiento.md) |
| Elegir dónde aparece el visitante | [llegada.md](guides/llegada.md) |
| Enlazar documentos y navegar entre mundos | [ubicacion.md](guides/ubicacion.md), [navegacion-mundo.md](guides/navegacion-mundo.md) |
| Pedir datos a un servidor | [red.md](guides/red.md) |
| Reproducir o generar sonido | [audio.md](guides/audio.md) |
| Trabajar con datos binarios | [binario.md](guides/binario.md) |
| Guardar datos en el visitante | [almacenamiento.md](guides/almacenamiento.md) |
| Usar Luna por escritorio remoto | [escritorio-remoto.md](guides/escritorio-remoto.md) |

El [índice de guías](guides/index.md) tiene la lista completa y, para cada tema,
el archivo del motor que manda cuando una guía y el código no coinciden.

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
