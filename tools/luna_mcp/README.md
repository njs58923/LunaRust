# MCP local de Luna

El adaptador habla MCP por stdio y escucha exclusivamente en `127.0.0.1:2054`.
Luna se conecta desde el host Rust; no depende de una página, de permisos de los
sitios ni de servicios externos. No inicia procesos por su cuenta.

## Uso

1. Instala las dependencias una vez: `bun install` en este directorio.
2. Configura tu cliente MCP con `bun run` y la ruta absoluta de `src/index.ts`.
3. En Luna, abre **Home → Ajustes → Iniciar MCP local**. También está en **Config → Enable local MCP** en escritorio.
4. Para desconectar, pulsa **Detener MCP** o desactiva la casilla. Arranca desactivado cada vez que abres Luna.

Los controles de la página Ajustes funcionan también en VR y al montarla en un
panel. Los alias antiguos `luna://agent` y `luna://agent_app` muestran Ajustes.
Navegar o cambiar entre escritorio y VR no reinicia el puente.

`LUNA_AGENT_PORT` cambia el puerto; usa el mismo valor en Luna y en el adaptador.
`LUNA_AGENT_TIMEOUT` configura la espera del adaptador (30000 ms por defecto).
El host limita cada respuesta a 25 segundos. La conexión se reintenta cada 2 segundos.
El proceso nuevo reemplaza la conexión anterior; no hay reservas, historial ni
seguimiento de sesiones. Un puerto ocupado se informa mediante las herramientas.

## Herramientas

- `luna_status`: consulta el estado actual del host: modo solicitado, modo efectivo,
  estado XR, cámaras, espacios montados, navegación en cola e includes cargando o fallidos.
- `luna_open({url})`: navega un espacio espacial y devuelve el `tabId` asignado por
  el host, `accepted:true` y `loading:true`. Es confirmación de encolado, no de carga terminada.
- `luna_camera({camera, position:[x,y,z], lookAt:[x,y,z]})`: pose absoluta en metros.
- `luna_capture({camera})`: PNG real. `camera` admite `auto`, `desktop` y `spectator`.
- `luna_wait({ms})`: espera entre 0 y 20000 ms; no garantiza que una página haya cargado.

`auto` elige espectador en modo VR y escritorio en modo desktop. La cámara
espectadora tiene pose independiente y captura a 1280×720; solo renderiza cuando
se solicita una imagen. No modifica el rig ni el tracking del visor. La captura
escritorio requiere su cámara activa y nunca cambia el modo automáticamente.
Ejemplo para mirar un panel: `luna_camera({camera:"spectator",position:[0,1.6,1],lookAt:[0,1.6,-3]})`.

Las imágenes viajan como bytes PNG por WebSocket: el adaptador no lee rutas
recibidas del navegador. El listener rechaza conexiones con Origin de páginas
web. El acceso se limita a procesos locales; no es un servicio autenticado para
redes o equipos compartidos.

## Verificación

`bun run test` ejecuta pruebas de reconexión, timeout, validación y MCP stdio con
un host simulado y puerto efímero. No ocupa el puerto de Luna. No prueba el render
de GPU ni un visor real.

`bun run probe.ts` inicia un adaptador para diagnóstico, consulta el estado de
Luna y sale. Úsalo cuando tu cliente MCP no esté usando el puerto; no arranca Luna
ni activa MCP por ti.
