# Catálogo compartido de los controllers

Los controllers plano y curvo leen la misma lista. El root define sus valores
iniciales en `crates/luna/src/web/internal/root_api.js` y la guarda en su propio
`localStorage`, bajo la clave **`luna.shell.items.v1`**, como un array JSON.
La primera ejecución guarda los valores iniciales; una lista vacía guardada se
respeta. Cambiar de controller, entrar en VR o reiniciar no restablece la lista.
Los defaults posteriores del código tampoco pisan una lista ya guardada.

El catálogo es un contrato JavaScript del shell, no una lista ni un formato de
menú impuesto por Rust. Cada entrada tiene un `id` string único y estable.
Los controllers actuales usan `name`, `url`, `kind` y el `glyph` opcional, y
omiten las entradas sin nombre o URL. `kind` puede ser `spatial`, `app` o
`app-embedded` para esos controllers. Otros controllers pueden conservar e
interpretar campos JSON adicionales, como colores, grupos o acciones propias.

## Leer desde el controller

El root abre el canal al montar el controller como `systemShell`. El estado
inicial está disponible en props, incluso antes de ejecutar su script:

```js
function actualizar() {
  const catalogo = component.props.shellItems;
  if (!catalogo) return;
  // { version: 1, revision: 0, items: [...] }
  dibujarMisItems(catalogo.items);
}
component.addEventListener('propschange', actualizar);
component.addEventListener('connect', actualizar);
actualizar();
```

El controller puede dibujar cualquier interfaz. Los nativos usan
`ShellApp.mount({ ..., sharedItems: true })`; los consumidores independientes
de ShellApp pueden seguir pasando `bookmarks` sin usar este contrato.
No se añade polling ni `requestAnimationFrame` para sincronizar el catálogo.

## Consultar y modificar por mensajes

Los nombres son minúsculos, como exige el canal de componentes:

```js
component.addEventListener('message:shell-items-result', e => {
  const respuesta = e.detail;
  // { requestId, ok, version, revision, items, error? }
  if (!respuesta.ok) console.warn(respuesta.error);
});

await component.emit('shell-items-get', { requestId: 'leer-1' });

await component.emit('shell-items-push', {
  requestId: 'agregar-1',
  revision: component.props.shellItems.revision,
  items: [{ id: 'mi-mundo', name: 'Mi mundo', url: 'https://ejemplo.test/', kind: 'spatial' }],
});

// Reemplazar toda la lista (también permite reordenar o borrar).
await component.emit('shell-items-update', {
  requestId: 'reemplazar-1',
  revision: component.props.shellItems.revision,
  items: [{ id: 'inicio', name: 'Inicio', url: 'luna://home', kind: 'spatial' }],
});
```

`emit()` confirma el encolado, **no el guardado**. Esperá la respuesta
`shell-items-result` de una escritura antes de enviar la siguiente. Usá un
`requestId` distinto para correlacionarlas (máximo 128 caracteres).
La revisión pertenece a la sesión del root, no es la revisión de props del
canal (`component.revision`). Ambas escrituras requieren la revisión vigente.
Ante `revision-conflict`, la respuesta incluye la lista actual: conciliá el
cambio antes de reenviarlo. `push` agrega al final y rechaza IDs duplicados;
no reemplaza entradas existentes. No hay deduplicación de solicitudes por
`requestId`.

Una escritura exitosa persiste antes de publicar nuevas props a los controllers
montados. Un fallo de almacenamiento mantiene la lista y la revisión anteriores.
Si el dato guardado es inválido, el root usa los defaults en memoria y registra
el problema sin destruir ese dato; una escritura posterior exitosa lo reemplaza.
Límites actuales: 128 entradas, IDs no vacíos de hasta 128 caracteres, JSON de
hasta 8000 unidades UTF-16 y profundidad 24, para dejar margen en el canal.

## Alcance y permisos

Sólo los includes que el root monta explícitamente como controllers
(`systemShell: true`) reciben este canal. Una página, una app o un include
anidado no obtiene acceso por declarar esos nombres de eventos, ni por compartir
origen con el controller. Los menús hijos siguen usando su canal con el
controller: no heredan el canal del root.

No se expone un proxy genérico de `localStorage`, ni se acepta una clave enviada
por el hijo. La autorización del root permite leer y escribir **este catálogo**;
las demás claves permanecen fuera del contrato. El canal conserva los controles
de endpoint, generación, tamaño y cola descritos en [componentes](componentes.md).
Los controllers son código de confianza elegido por el root, capaces de
modificar la lista compartida; no hace falta un permiso nativo adicional.

Los menús nativos devuelven el ID al seleccionar una entrada para evitar que un
clic pendiente abra otro elemento cuando cambia el orden de la lista.
