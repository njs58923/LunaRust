# Guías de Luna (HSML + JS)

Controllers: [catálogo compartido del shell](catalogo-shell.md).
Entrada y edición: [teclado físico, virtual y TextBox](teclado.md).

Cómo **escribir documentos espaciales** que corran en Luna. Todo lo de acá está
verificado contra el código, no contra el README; cuando algo cambió en el motor,
la página lo dice.

> Si vas a leer una sola página, que sea **[trampas.md](trampas.md)**. Es lo que
> cuesta una corrida averiguar por cuenta propia.

---

## Lo mínimo

Un documento es un XML con una raíz `<space>`, geometría adentro y un script.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head><name>Hola</name></head>

  <!-- Separadas por COMA. Con espacios queda un token que no matchea nada y el
       documento se queda sin capacidades, en silencio. -->
  <space resources="navigate_self,read_camera_pose">
    <box id="boton" y="1.4" z="-1.5" sx="0.6" sy="0.25" sz="0.06"
         color="#405E55" touchable="true" border-radius="0.03"/>
    <text id="cartel" y="1.7" z="-1.5" value="tocá la caja" size="0.08"/>

    <script src="./hola.js"/>
  </space>
</hsml>
```

```js
const root = hiperspace.dimention;
let n = 0;
root.getElementById('boton').addEventListener('toque', (e) => {
  root.getElementById('cartel').setAttribute('value', 'toques: ' + (++n));
  console.log('impacto en', e.x, e.y, e.z);
});
```

Cuatro cosas que conviene tener claras desde el principio:

- **Las unidades son metros.** El suelo está en `y = 0`, los ojos alrededor de
  `1.6`, y `-z` es «hacia adelante» desde el origen del espacio.
- **`touchable` no tiene default permisivo.** Sin él, un nodo no recibe `toque`
  ni hover.
- **Los `resources` van separados por coma**, sin espacios.
- **Un error de sintaxis en el script no sale por ningún lado.** No hay log, no
  hay excepción: el script simplemente no corre. Comprobalo con
  `bun build tu.js --outfile /dev/null` antes de buscar el problema en otro lado.

---

## Modelo mental

Un documento HSML es un **árbol de nodos con transformación**, no una escena de
un motor de juego. Se parece más al DOM que a Unity:

- cada nodo tiene posición, rotación y escala **locales** a su padre;
- el script vive en un isolate por espacio y habla con el motor por una cola:
  **nunca bloquea al render, y el render nunca lo bloquea a él**;
- un `<include>` es otro documento adentro, con **sus propios permisos y su
  propio isolate** — no es una plantilla que se expande.

---

## Las páginas

### Referencia

| | |
|---|---|
| [documento.md](documento.md) | anatomía de un `.hsml`, tags válidos y atributos de cada uno |
| [permisos.md](permisos.md) | `resources`, los bundles y qué hereda un `<include>` |
| [javascript.md](javascript.md) | la API del script: raíz, crear, mover, consultar |
| [eventos.md](eventos.md) | `toque` y hover (`pointerenter`, `matches(':hover')`) |
| [componentes.md](componentes.md) | `props` y `events`: hablar con un `<include>` |
| [ejemplo.md](ejemplo.md) | un documento y su script, enteros |

### Por tema

| | |
|---|---|
| [interfaz.md](interfaz.md) | medir texto, el atlas de glifos, curvas y color con alfa |
| [framework-ui.md](framework-ui.md) | `luna://internal/ui.js`: paneles, controles y enlaces |
| [superficies.md](superficies.md) | texturas sobre primitivas, atlas, y el tag `<image>` |
| [modelos.md](modelos.md) | `<model>`: cómo se cargan y comparten los glTF |
| [animaciones.md](animaciones.md) | los clips que trae un `.glb` |
| [llegada.md](llegada.md) | `<spawn>`: dónde aparece el visitante |
| [ubicacion.md](ubicacion.md) | `location`, `URL`, navegar entre documentos |
| [red.md](red.md) | `fetch`, y qué permisos pide |
| [audio.md](audio.md) | `Audio` y `AudioStream`: clips y PCM generado |
| [binario.md](binario.md) | `Blob`, `TextEncoder`/`TextDecoder`, `IO.Buffer`, `IO.pipe` |
| [almacenamiento.md](almacenamiento.md) | `localStorage` por origen |
| [entorno.md](entorno.md) | el entorno nativo que monta el shell |

### Y lo importante

| | |
|---|---|
| **[trampas.md](trampas.md)** | **todo lo que falla en silencio** |

### Diseños

Propuestas y notas de diseño, no referencia de lo que existe hoy:

| | |
|---|---|
| [disenos/menu-vr.md](disenos/menu-vr.md) | el layout del menú en VR, con medidas |
| [disenos/navegacion-de-mundo.md](disenos/navegacion-de-mundo.md) | cómo podría ser `navigate_world` |

---

## Fuentes de verdad

Cuando una guía y el motor no coincidan, gana el motor. Y avisá.

| Tema | Archivo |
|---|---|
| Tags válidos | `crates/virtual_dom/src/dom/tags.rs` |
| Atributos y render de cada tag | `crates/luna/src/dom.rs` |
| Texturas y materiales | `crates/luna/src/surface.rs` |
| API JS completa | `crates/js_runtime/runtime.js` |
| Permisos / bundles | `crates/luna/src/permissions.rs` |
| Eventos (raycast) | `crates/luna/src/touch.rs` |
| Mallas dinámicas | `crates/js_runtime/src/mesh.rs` |
| Texto, glifos y curvas | `crates/ui_graphics/src/`, `crates/js_runtime/src/ui_text.rs` |
| El framework de interfaz | `server_ui/` (se genera desde ahí; ver framework-ui.md) |
| Canal de componentes | `crates/luna/src/components.rs`, `crates/js_runtime/components.js` |
| Audio (ops y mezcla) | `crates/js_runtime/src/audio.rs`, `crates/luna/src/audio.rs` |
| Blob, texto y buffers | `crates/js_runtime/binary.js` |
| Páginas internas de ejemplo | `crates/luna/src/routes.rs` |

## Lo que no está acá

Estas guías son sobre **cómo usar** la plataforma. Lo demás vive aparte:

- `docs/` — análisis, mediciones y estado de implementación;
- `docs/deuda_tecnica/` — lo que se sabe que está mal y todavía no se arregló;
- `CORE_PERFORMANCE.md`, `ENGINE_DENSITY_OPTIMIZATION.md`,
  `PLATFORM_DIAGNOSTICS_FETCH.md` — trabajo sobre el motor, no sobre su uso.

## Poses y trabajo pendiente

- [Navegar el mundo desde una puerta/include](navegacion-mundo.md).

- [Poses locales y setJointBatch](poses.md).
- [Deuda técnica: carga masiva de modelos](deuda-carga-modelos.md).
