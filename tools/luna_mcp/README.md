# luna_mcp

Servidor MCP para manejar Luna desde un agente: abrir una URL, esperar y
**capturar el frame real que renderiza el motor**.

## Cómo está partido

```
Claude Code  ──stdio(MCP)──>  luna_mcp  ──WebSocket(2054)──>  luna://agent_app
                                                                (dentro de Luna)
```

El que escucha es este proceso, no Luna: el runtime del navegador sólo sabe ser
cliente WebSocket (`crates/luna/src/ws.rs` usa `connect_async`). El puente
adentro de Luna se conecta hacia acá y ejecuta los comandos que le llegan.

## Piezas en el motor

| Qué | Dónde |
|---|---|
| Capacidad `CAPTURE_FRAME` (elevada) y bundle `capture_frame` | `crates/luna/src/permissions.rs` |
| Ops `op_capture_frame` / `op_capture_poll` | `crates/js_runtime/src/lib.rs` |
| API JS `dimention.captureFrame(nombre)` | `crates/js_runtime/runtime.js` |
| Validación de la cap por space | `crates/luna/src/js.rs` |
| Captura con Bevy y escritura del PNG | `crates/luna/src/capture.rs` |
| Lanzador y puente | `crates/luna/src/web/apps/agent_launcher.hsml`, `agent_bridge.hsml` |
| Grants de esas dos URLs | `crates/luna/src/web/internal/root_api.js` |

`CAPTURE_FRAME` es elevada a propósito: leer la pantalla no es algo que un
documento remoto pueda pedir sin consentimiento. Las únicas páginas que la
reciben son `luna://agent_app`, y por una excepción explícita en `root_api.js`.

Los PNG van a `%TEMP%\luna-captures\` (`std::env::temp_dir()`), no al repo.

## Uso

1. Levantá el servidor MCP (lo hace el cliente por vos si lo registrás):

   ```bash
   bun run src/index.ts
   ```

2. Registralo en Claude Code:

   ```bash
   claude mcp add luna -- bun run "G:/#Proyectos Activos/Unity/LunaExperiments/bevy_oxr/tools/luna_mcp/src/index.ts"
   ```

3. En Luna, andá a `luna://agent` y tocá **Iniciar agente**. Eso monta el puente
   como app aditiva: sobrevive a los cambios de espacio, así que puede navegar
   la escena sin matarse a sí mismo.

El cubito del puente indica el estado: naranja conectando, verde conectado,
rojo caído. Reintenta solo cada 2 s, así que el orden de arranque no importa.

## Herramientas

| Herramienta | Qué hace |
|---|---|
| `luna_status` | dice si el puente está conectado |
| `luna_open` | abre una URL como espacio spatial (abrir uno cierra el anterior, así que también sirve de recarga) |
| `luna_wait` | espera N ms dentro de Luna |
| `luna_capture` | captura el frame y devuelve el PNG como imagen |

## Prueba sin Luna

```bash
bun run test_e2e.ts
```

Levanta el server, le enchufa un puente falso y hace el ida y vuelta completo
por stdio: `initialize`, `tools/list`, status con y sin puente, un comando que
responde bien y otro que responde error.

## Lo que falta

- **`wait_ready` de verdad.** Hoy `luna_wait` es un sleep. El host ya sabe
  cuándo terminó de cargar (`IncludeLoadStates`, `ScriptLoadStates`,
  `PendingModelLoads`, `dirty_nodes`): exponer eso convierte la espera en
  determinista.
- **Mover la cámara.** `op_read_viewer_pose` es de sólo lectura. Mientras tanto
  se encuadra moviendo el documento (`<meta position>`) o la pose del tab.
- **Inspeccionar el DOM de la escena.** No sale gratis: cada space es su propio
  isolate con su propia tabla de handles, así que el puente no ve el DOM del
  documento que está mirando. Requiere una op nueva del lado del host.
