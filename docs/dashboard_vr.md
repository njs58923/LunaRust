# Dashboard VR y adopción de server_ui

El shell nativo conserva su máquina de estados y el contrato `window-local` de las apps. El layout visual usa superficies propias: el ejemplo `server_noche/public/menu.js` inspira la orientación cilíndrica y los iconos planos, sin introducir una dependencia de un servidor externo en el menú del sistema.

## Cambios del dashboard

- Cabecera y grilla regular de tres columnas. Cada columna y tarjeta se orienta hacia el usuario a 1,5 m; los fondos siguen las columnas para no atravesar las tarjetas con un plano único.
- Iconos en `plane` redondeado con el mismo atlas compartido `luna://icons/menu.png`, región y padding. Etiquetas legibles en una capa frontal.
- Toda la tarjeta, incluida la etiqueta, es un blanco estable. El hover cambia el fondo sin mover el blanco y respeta dos mandos apuntando simultáneamente. Icono y etiqueta se animan juntos.
- Barra persistente durante la sesión con apps abiertas, nombres de apps, cierre separado de 5,5 cm y paginación cuando hay más de dos apps. Cada pastilla sigue un arco más cercano y se inclina hacia la mirada. Ancho máximo 1,35 m; el botón Inicio siempre está disponible. Nombres largos se abrevian.
- Controles de ventana de 8 cm, superficies planas, títulos acotados y acciones Fijar / Siempre con texto. Las poses y dimensiones del contenido embedded no cambian.

## Qué tiene realmente server_ui

`public/host.js` implementa invalidación bajo demanda, measure/arrange, piletas de textos/imágenes/blancos, estilos por tipo y enlaces. `paneles.js`, `interactivos.js` y `mas.js` incluyen paneles, scroll, virtualización y controles interactivos. No hace falta empezar de cero ni añadir hover desde cero.

## Qué falta para usarlo como shell nativo

1. **Transformaciones de interacción.** `host.js`, listener `toque`, convierte mundo a hoja restando origen X/Y y supone expresamente que el panel no rota. Necesita la inversa completa de la transformación del contenedor y una proyección de superficie para un layout curvo. Pruebas con yaw, escala, panel inclinado y dos mandos.
2. **Invalidación por fases.** `Aplicacion.pasada()` vuelve a enlazar, medir, acomodar, emitir y actualizar toda la malla incluso en un cambio de hover. Separar cambios de layout, pintura y datos; actualizar sólo los buffers o propiedades afectados.
3. **Hover de varios punteros.** El `pointerleave` de los blancos asigna `encima=false` incondicionalmente. Consultar `matches(':hover')` para no apagar el otro mando; definir también qué pasa si una pileta reasigna un blanco mientras se está apuntando.
4. **Recorte de texto real.** `ctx.texto` decide por el centro si dibuja el nodo completo. No sirve para texto parcialmente visible en ScrollViewer; hace falta recorte de superficie o un contrato de viewport compartido para texto e imágenes.
5. **Imágenes y materiales actuales.** El comentario que afirma que las mallas no pueden tener textura quedó desactualizado. Conectar atlas/regiones y materiales del motor; medir imágenes al cargar usando naturalWidth/naturalHeight e invalidar el layout cuando cambian.
6. **Integración de ventanas y ciclo de vida.** Un host que acepte el slot del shell, cambie tamaño/pose, suspenda animaciones y libere recursos/listeners al cerrar. `correr()` agenda RAF continuamente y no expone un método de disposición.
7. **Distribución nativa y pruebas de integración.** Empaquetar una versión local del runtime de UI, con API estable y pruebas de layout + interacción contra el motor. Las pruebas aritméticas actuales son una base útil, pero no verifican rayos, oclusión y transformación de ventanas.

Orden sugerido: transformaciones y ciclo de vida; recorte y hover múltiple; invalidación por fases; integración de atlas. Entonces migrar primero una ventana pequeña, antes de reemplazar el shell completo.
