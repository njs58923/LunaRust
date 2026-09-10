# Dibujar interfaz: texto medido, glifos y curvas

Tres servicios que el host presta al isolate y que juntos cambian cómo se hace
una interfaz adentro de un mundo. Antes había que elegir entre **un nodo por
cosa** —caro— o **una malla** que no sabía escribir ni curvar. Ahora la malla
puede las dos.

Ninguno de los tres pide permiso: son cómputo, no acceso. Ninguno crea entidades
ni mallas por su cuenta, y ninguno pide cuadros: devuelven datos y se van.

---

## `TextLayout.create` — cuánto mide, y dónde va cada glifo

```js
const l = TextLayout.create("La cueva", 0.08);
// { w: 0.31, h: 0.08, glyphs: [ {x, y, w, h, u, v, uw, vh, page}, ... ] }

// Con ancho: corta en palabras y devuelve el alto de las líneas que salieron.
const parrafo = TextLayout.create(texto, 0.04, 0.6);
```

`w` y `h` son metros. Cada glifo trae su caja (`x, y, w, h`) y su rectángulo en
el atlas (`u, v, uw, vh`) más la `page` donde cayó.

Límites: 16 KiB de texto, `size` finito entre 0 y 100, `width` no negativo —
`0` significa sin corte. Fuera de eso tira, no recorta.

### El atlas es una textura como cualquier otra

```xml
<plane texture="luna://ui-font/0" texture-face="front" material-alpha="blend"/>
```

`luna://ui-font/<page>` entrega la página `<page>` del atlas de glifos: 1024×1024,
gris con alfa. Es lo que cierra el círculo — con las UV de `TextLayout.create` y
esa textura, **el texto entra en tu propia malla** y deja de necesitar un `<text>`
por renglón.

El atlas crece solo, a medida que se piden glifos nuevos, hasta 8 páginas. Una
página se llena y se abre otra; por eso el `page` viene por glifo y no por
llamada.

> Ojo: la página cambia de contenido cuando entran glifos nuevos. Si cacheás una
> malla de texto, cacheá también qué `page` usaste, y volvé a medir cuando el
> texto cambie: las UV de ayer pueden apuntar a otra letra.

---

## `PathGeometry.tessellate` — curvas

```js
const g = PathGeometry.tessellate([
  { op: "moveTo",   x: 0,    y: 0 },
  { op: "bezierTo", c1x: 0,   c1y: 0.3, c2x: 0.3, c2y: 0.3, x: 0.3, y: 0 },
  { op: "close" },
], { tolerance: 0.0001, strokeWidth: 0, nonZero: false });
// { positions, indices, contours, closed }
```

Comandos: `moveTo` / `lineTo` (`x,y`), `quadraticTo` (`cx,cy,x,y`),
`bezierTo` (`c1x,c1y,c2x,c2y,x,y`) y `close`. **Cada subtrazo arranca con
`moveTo`.**

- `positions` son pares **XY**. Para una `MeshResource` hay que pasarlos a XYZ —
  ver [mallas dinámicas](javascript.md).
- `strokeWidth: 0` **rellena**, con regla EvenOdd (o sea, los subtrazos internos
  hacen agujeros) o NonZero si se pide `nonZero: true`.
- `strokeWidth > 0` **traza**, con puntas y uniones redondeadas.
- `contours` describe los subtrazos de entrada ya aplanados — **no** el contorno
  del trazo generado. Se confunden.
- El winding de `indices` es positivo en XY.

Es **síncrono** y no programa nada: aplana las curvas a la tolerancia pedida y
tesela. Guardá la geometría y volvé a llamar sólo cuando la forma cambie.

Límites: 1 a 128 comandos, coordenadas finitas dentro de ±1000, `tolerance` de
0.00001 a 0.1, `strokeWidth` de 0 a 10, hasta 512 eventos aplanados y 65 536
vértices / 196 608 índices. Pasarse **devuelve error**; un dibujo complejo se
parte en varios trazos.

---

## Color con alfa

`color` acepta ahora la notación CSS completa, con el alfa **al final**:

| | |
|---|---|
| `#RGB` / `#RGBA` | forma corta, cada dígito se duplica: `#1234` es `#11223344` |
| `#RRGGBB` / `#RRGGBBAA` | forma larga |
| `transparent` | la palabra, equivale a `#00000000` |

**Pero el alfa solo no alcanza.** Un `box`, `plane`, `sphere` o `cylinder` sin
atributos de superficie se dibuja con un material **opaco**: el alfa se parsea,
se guarda, y no se ve. Para que atraviese hay que pedirlo:

```xml
<plane color="#0A84FF80" material-alpha="blend"/>
```

`material-alpha` es lo que hace que el nodo pase por el camino de superficies
(`surface.rs`), que es el único que fija el modo de mezcla. Sin él el nodo lo
arma `spawn_colored_primitive`, que deja el default de Bevy — `Opaque`.

De ahí sale el uso que más rinde: **un blanco táctil invisible**. Un plano
`color="transparent"` con `material-alpha="blend"` sigue recibiendo `toque` y
`pointerenter` y no dibuja nada, así que las zonas de impacto dejan de tener que
disfrazarse del color de lo que tapan.

---

## Qué reemplaza a qué

| Antes | Ahora |
|---|---|
| adivinar el ancho con `size · nChars · 0.6` | `TextLayout.create(...).w` |
| un `<text>` por renglón, con su nodo y su material | glifos en tu malla, con `luna://ui-font/<page>` |
| esquinas redondeadas teseladas a mano | `PathGeometry.tessellate` |
| pintar el blanco táctil del color del fondo | `transparent` + `material-alpha="blend"` |

El primero también corrige al motor mismo: el ancho de un `<text>` **ya no es la
aproximación monoespaciada**. `build_text_transform` mide la cadena con la fuente
real (FiraSans) vía `text_texture_size`, así que un `<text>` ocupa lo que ocupa.

---

## Fuentes de verdad

| Tema | Archivo |
|---|---|
| Atlas, layout y color | `crates/ui_graphics/src/lib.rs` |
| Teselado de trazos | `crates/ui_graphics/src/paths.rs` |
| Los dos ops y sus límites | `crates/js_runtime/src/ui_text.rs` |
| El atlas como textura | `crates/luna/src/surface.rs` (`luna://ui-font/`) |
| Páginas del atlas en Bevy | `crates/luna/src/ui_text.rs` |

Un consumidor entero de todo esto vive en `server_ui/RENDERING.md`, con su demo
en `/curves.hsml`. Y ese consumidor viene con el motor: es el framework de
[framework-ui.md](framework-ui.md), que ya usa las tres cosas de esta página —
mide su texto, lo mete en su propia malla y tesela sus esquinas.

---

Volver al [índice de guías](index.md).
