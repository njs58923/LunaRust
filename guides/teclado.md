# Teclado, foco y edición

El host selecciona un documento cuando el usuario toca uno de sus elementos.
Puede ser una ventana, un mundo o un documento dentro de un include. Sólo ese
documento recibe el teclado. Otro documento no puede tomar el foco por script.
Cerrar, ocultar o desmontar el documento revoca el foco; perder el foco de la
ventana nativa también lo revoca en escritorio. En VR se usa el estado de la
sesión XR, no el foco de la ventana espectadora.

## Eventos familiares

El runtime expone `keyboard`. Un elemento HSML puede usar `element.focus()` y
`element.blur()`; el foco efectivo requiere que el usuario haya seleccionado
su documento. `keyboard.activeElement` devuelve el destino local. No existe
un DOM HTML ni un `document.activeElement` global que cruce isolates.

```js
const root = hiperspace.dimention;
const visor = root.getElementById('visor');
visor.addEventListener('toque', () => visor.focus());
visor.addEventListener('keydown', e => {
  if (e.key === 'Enter') {
    e.preventDefault();
    confirmar();
  }
});
root.addEventListener('keyup', e => console.log(e.key, e.code));
```

`keydown` y `keyup` incluyen `key`, `code`, `location`, `repeat`, `shiftKey`,
`ctrlKey`, `altKey`, `metaKey` e `isComposing`. Propagan al root del documento;
`preventDefault()` en `keydown` cancela su edición predeterminada. También se
puede cancelar `beforeinput` para impedir la inserción. El canal soporta
`compositionstart`, `compositionupdate` y `compositionend`, y confirma el texto
del IME por separado para no insertarlo dos veces. Los eventos entregados por
controllers tienen `isTrusted: false` y no autorizan operaciones privilegiadas.

Los elementos HSML genéricos reciben eventos, pero no adquieren un editor de
texto automáticamente. El framework UI proporciona `TextBox` como editor.
Los controles dibujados en un include tienen su propio destino y reciben los
eventos directamente: no necesitan callbacks de reenvío por cada padre.

## Contrato del controller

Ambos diseños nativos usan la misma lógica de entrada en `ShellApp`. La elección
de diseño plano/curvo es independiente del dispositivo desktop/VR.

```js
keyboard.addEventListener('deviceinput', e => {
  keyboard.send(e.packet, e.revision);
});
keyboard.addEventListener('statechange', state => {
  // state.vr, state.hasTarget, state.editable, state.revision
});
```

Sólo el controller de sistema montado por el root puede inyectar eventos con
`keyboard.send`. El host comprueba su identidad, el documento seleccionado,
su visibilidad y la revisión. La revisión evita entregar una tecla atrasada a
otra ventana. Ni el controller ni el teclado virtual eligen un ID de destino.
Las colas y los tamaños están acotados.

Cuando el foco es editable y el dispositivo es VR, `ShellApp` monta
`luna://keyboard` como include. Se puede sustituir mediante
`ShellApp.mount({ ..., keyboardUrl: 'https://ejemplo.test/teclado.hsml' })`.
El componente recibe `props.revision` y puede emitir `key` y `close`:

```js
component.emit('key', {
  revision: component.props.revision,
  packet: { type: 'keydown', key: 'ñ', code: 'Unidentified', text: 'ñ' }
});
component.emit('key', {
  revision: component.props.revision,
  packet: { type: 'keyup', key: 'ñ', code: 'Unidentified', text: '' }
});
```

El teclado no guarda otra copia del texto ni recibe el valor del campo.
Sus teclas no le quitan el foco. La distribución nativa ofrece letras españolas,
mayúsculas, símbolos, flechas, Enter, Tab y acciones de portapapeles. Las teclas
conservan sus nodos al cambiar de distribución y sólo solicitan frames durante
las transiciones de hover/pulsación. La orientación usa únicamente yaw.

## TextBox en la UI

```xml
<TextBox Name="direccion" Text="{Binding url, Mode=TwoWay}"
         Width="0.60" MaxLength="2048" Placeholder="https://…"
         TextChanged="escribiendo" Changed="guardar"/>
```

`TextChanged` corresponde a cada modificación (`input`); `Changed` se ejecuta
al confirmar con Enter o perder el foco si el texto cambió. El binding `TwoWay`
ya está actualizado cuando llega el callback. `IsReadOnly`, `IsEnabled`,
`FontSize`, `Background`, `Foreground` y `CornerRadius` controlan su presentación
y comportamiento. `setSelectionRange(inicio, fin)` usa índices UTF-16, y
`select()` selecciona todo.

Admite flechas, Inicio/Fin, selección con Shift, Ctrl/Cmd+A, Backspace/Delete,
deshacer/rehacer y copiar/cortar/pegar. Tab/Shift+Tab recorren los controles
visibles de la aplicación UI actual, con `TabIndex` e `IsTabStop="false"`.
Enter/Espacio activan botones; los sliders y ComboBox también responden a
flechas. `KeyDown`, `KeyUp`, `GotFocus` y `LostFocus` permiten declarar handlers.

La selección, el cursor y el texto se dibujan recortados al campo y se desplazan
horizontalmente. El cursor no parpadea: no requiere un loop permanente.
Mientras hay un campo editable enfocado, WASD/Shift/Escape no activan la
locomoción de escritorio ni su apertura del menú.

El acceso al portapapeles se limita a copiar/cortar/pegar solicitados mediante
una tecla del controller y al documento enfocado. No se añade lectura arbitraria
del portapapeles. Se utiliza el portapapeles nativo disponible en escritorio;
el backend de portapapeles Android queda pendiente.

Alcance actual: editor de una línea, sin texto enriquecido, selección mediante
arrastre ni navegación Tab entre isolates. El transporte de composición está
implementado; candidatos del IME y tipografía dependen del sistema operativo y
de las limitaciones de shaping/fuentes del renderer actual.

Las fuentes de la UI están en `server_ui/public/keyboard.js`, junto al resto
de piezas. `bun run build` en ese proyecto regenera el `ui.js` incluido en Luna.
