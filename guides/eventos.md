# Eventos: toque, hover y pose

Todo lo que el motor le manda al script cuando alguien apunta, toca o mueve una
mano. El requisito es siempre el mismo: **`touchable="true"` en el objetivo**.

---

## Los que emite el host

`addEventListener(type, fn)` / `removeEventListener` / `dispatchEvent(evt)`.
Los tipos se normalizan a minúsculas. También hay handlers `onX` por propiedad.

Eventos que emite el host:

| Evento | Cuándo | Campos extra |
|---|---|---|
| `toque` | click de mouse (escritorio) o trigger derecho (VR) sobre un nodo `touchable` | `x`, `y`, `z` = punto de impacto en world space |
| `pointerenter`, `pointerleave` | entrada/salida del cursor o rayos VR; no burbujean | `target`, `currentTarget`, `relatedTarget`, `pointerId`, `pointerType`, `hand` |
| `pointerover`, `pointerout` | cambio de objetivo; burbujean dentro del isolate | mismos campos |
| `posemove` | mano/mando dentro de un `<posezone>` (requiere `read_pose_stream`) | `hand`, `px py pz`, `dx dy dz`, `trigger`, `grip`, `qx qy qz qw` |

`toque` es *el* click en Luna. No existe `click` nativo: `el.click()` sólo despacha
un evento sintético `click` a tus propios listeners.

```js
const btn = root.getElementById('btn');
btn.addEventListener('toque', (e) => {
  console.log('impacto en', e.x, e.y, e.z);
  location.href = 'luna://home';
});
```

Hover admite también `mouseenter`, `mouseleave`, `mouseover` y `mouseout` para
mouse y mando derecho, propiedades `onpointerenter`, etc., y `matches(':hover')`.
Para ambos mandos usar `pointer*`. Requiere `touchable="true"` en el objetivo.
Ver [semántica y ejemplo de hover](eventos.md). No incorpora un motor CSS.

---

## Hover, en detalle

Los nodos con `touchable="true"` reciben hover sin pulsar ningún botón: con el
cursor en escritorio, con el centro de la vista en modo shooter y con los rayos
de ambos mandos en VR. Se elige el objeto visible más cercano, usando las mismas
formas de impacto que `toque`. Los rayos VR tienen un alcance de 20 metros.

```js
const boton = root.getElementById('boton');
boton.onpointerenter = () => boton.setAttribute('color', '#FFFFFF');
boton.onpointerleave = () => {
  if (!boton.matches(':hover')) boton.setAttribute('color', '#405E55');
};
// También: boton.addEventListener('mouseenter', handler).
```

## Semántica familiar a HTML

- `pointerenter` / `pointerleave`: entrada y salida del elemento y sus
  descendientes; no burbujean. Pasar entre dos hijos no hace salir al padre.
- `pointerover` / `pointerout`: cambio del objeto apuntado; burbujean hasta la
  raíz del documento del isolate.
- `mouseenter`, `mouseleave`, `mouseover`, `mouseout`: equivalentes de
  compatibilidad para el mouse y el mando derecho. Para ambos mandos, usar los
  eventos `pointer*`.
- Se admiten listeners y propiedades `onpointerenter`, `onmouseleave`, etc.
- `target`, `currentTarget` y `relatedTarget` son elementos; `relatedTarget` es
  `null` al salir al fondo o cruzar a otro documento. Los eventos no atraviesan
  el límite entre isolates de un include y su padre.
- `stopPropagation()` y `stopImmediatePropagation()` controlan la propagación.
  Estos eventos no son cancelables.
- `pointerId`: 1 mouse, 2 mando izquierdo, 3 mando derecho. `pointerType`:
  `mouse` o la extensión de Luna `xr`. `hand`: `left`, `right` o `null`.
- `matches(':hover')` incluye descendientes y permanece activo mientras alguno
  de los punteros apunte allí. Se actualiza antes de ejecutar los callbacks.

Esto no incorpora estilos CSS ni selectores generales: `matches()` admite por
ahora únicamente `:hover`. `toque` conserva su comportamiento actual de click.
No se incorporan todavía `pointermove`, captura del puntero ni fase de captura.

## Ciclo de vida y coste

El host reutiliza el raycast de click para el hover de mouse y mando derecho.
Solo envía cambios de objetivo, con identificadores locales al isolate. Una
cola ocupada conserva el último estado pendiente. Los callbacks se ejecutan en
el tick del worker, bajo su límite de ejecución habitual.

Ocultar o quitar el objetivo, salir de la ventana o desactivar el modo de entrada
libera el hover en la siguiente actualización de input y del worker. La
jerarquía se vuelve a comprobar en el worker para contemplar movimientos o
eliminaciones del DOM aunque el puntero permanezca quieto.

---

## Eventos que cruzan un `<include>`

Ni `toque` ni el hover cruzan el borde de un include: se despachan sólo al
espacio dueño del nodo. Lo que sí cruza es el canal de componentes — el hijo
emite y el padre escucha `component:<nombre>` **en el nodo include**. Ver
[componentes.md](componentes.md).


Volver al [índice de guías](index.md).
