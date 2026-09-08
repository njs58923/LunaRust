# La API JavaScript

Lo que ve el script de un espacio: la raíz, cómo crear y componer nodos, mover
cosas, consultar y escuchar eventos.

Los eventos de puntero —`toque` y hover— tienen su propia página en
[eventos.md](eventos.md).

---

## Lo que hay

`location` expone la URL del documento de cada isolate, con `search`, `hash`,
`pathname`, `origin` y navegación. Para leer parámetros:
`new URLSearchParams(location.search).get('uuid')`.
También está disponible `new URL(ruta, location.href)`.
Ver [Location y parámetros](ubicacion.md) para includes, permisos y límites.

### Raíz

```js
const root = hiperspace.dimention;   // sí, "dimention" — capa de compat Unity
```

`hiperspace.dimention` es un `HSMLRootElement` (nodo 0 del espacio).
`location` global apunta a `hiperspace.location`.

### Crear y componer

```js
const box = root.createElement('box');   // pendiente hasta que el host lo resuelva
box.setAttribute('color', '#405E55');
box.setAttribute('touchable', 'true');
box.id = 'mi-caja';
box.className = 'boton';
box.classList.add('activo');
root.appendChild(box);

box.removeChild(hijo);
box.remove();
```

### Transformaciones

`position`, `rotation`, `scale` y `globalPosition` devuelven un **proxy vivo**:
mutar `.x/.y/.z` escribe al host inmediatamente.

```js
box.position.y = 1.5;
box.rotation.y += 0.01;
box.scale = { x: 2, y: 2, z: 2 };     // asignación completa también sirve
const p = box.globalPosition;          // world space
```

Ojo: asignar un objeto captura los valores en ese momento, así que podés reusar un
vector temporal en un bucle sin aliasing.

Para mover muchos nodos por frame, usá el batch (una sola op en vez de N):

```js
// layout plano, múltiplos de 7: [nodeId, px,py,pz, rx,ry,rz, ...]
root.setTransformBatch([
  a.nodeId, 0,1,-2, 0,0,0,
  b.nodeId, 1,1,-2, 0,0.5,0,
]);
```

### Consultas

```js
root.getElementById('btn');
root.getElementByClass('boton');
root.getElementByName('avatar');
root.getElementsByClass('boton');   // array
root.getElementsByName('enemigo');  // array
el.parent;    // HSMLElement | null
el.children;  // array
el.tagName;
```

No hay `querySelector` con selectores CSS: sólo estas búsquedas por id / class / name.

### Eventos

Tienen su propia página: **[eventos.md](eventos.md)** — `toque`, hover y
`posemove`, con sus campos y sus reglas de propagación.

### Web APIs disponibles

```js
setTimeout / setInterval / clearTimeout / clearInterval
requestAnimationFrame / cancelAnimationFrame
console.log / warn / error
await fetch(url)                 // requiere fetch_text (lectura) o fetch_http, mismo origen
localStorage                     // persistente por origen, sin permiso
new WebSocket('ws://...')        // onopen/onmessage/onerror/onclose, polling cada 16 ms
location.href = '...'            // navegar (requiere navigate_self / navigate_global)
```

`localStorage` se separa por **protocolo + host + puerto**. Todas las páginas
`luna://` comparten un almacén interno. `data:` y `file:` reciben `SecurityError`.
Detalle completo en [`../LOCAL_STORAGE.md`](almacenamiento.md).

### APIs condicionadas a permisos

```js
// read_hmd_pose
const pose = hiperspace.dimention.readViewerPose();
// { mode:'vr'|'desktop', px,py,pz, forwardX/Y/Z, yaw,pitch, qx,qy,qz,qw, aspect,fovY }

// manage_tabs
dimention.tabs.open(url, { kind: 'spatial' | 'app' | 'app-embedded' });
dimention.tabs.close(tabId);

// root (sólo páginas nativas)
dimension.luna.mountSpace(url, { tabId, kind });
dimension.luna.unmountSpace(id);
dimension.luna.listMountedSpaces();
dimension.luna.switchMode('vr' | 'desktop');
```

Tipos de espacio:

- `spatial` — "irte a otro lugar"; al abrir uno, el shell cierra los otros spatial.
- `app` — aditivo, sobrevive a cambios de espacio spatial.
- `app-embedded` — aditivo, además negocia un slot con el shell vía `dimention.embedded`.

---

---

Volver al [índice de guías](index.md).
