import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { CallToolRequestSchema, ListToolsRequestSchema } from "@modelcontextprotocol/sdk/types.js";
import { Bridge, pngResult } from "./bridge.ts";

const port = Number(process.env.LUNA_AGENT_PORT ?? 2054);
if (!Number.isInteger(port) || port < 0 || port > 65535) throw new Error("Invalid LUNA_AGENT_PORT");
const timeout = Number(process.env.LUNA_AGENT_TIMEOUT ?? 30_000);
if (!Number.isFinite(timeout) || timeout <= 0) throw new Error("Invalid LUNA_AGENT_TIMEOUT");
const bridge = new Bridge(timeout);
let listenError: string | null = null;
let listener: ReturnType<typeof Bun.serve> | undefined;

// Escuchar puede fallar al arrancar si otra sesión todavía tiene el puerto (dos
// ventanas de Claude, o una que quedó abierta). Antes se intentaba una sola vez
// y el adaptador quedaba mudo para siempre aunque el otro se cerrara; ahora cada
// llamada a una herramienta vuelve a probar mientras no esté escuchando.
function escuchar(): void {
  if (listener) return;
  try {
    listener = Bun.serve({
      hostname: "127.0.0.1", port,
      fetch(request, server) {
        // Native clients only; browser pages send Origin.
        if (request.headers.has("origin") || new URL(request.url).pathname !== "/") return new Response("Forbidden", { status: 403 });
        if (server.upgrade(request, { data: undefined })) return;
        return new Response("Luna MCP WebSocket endpoint", { status: 426 });
      },
      websocket: {
        maxPayloadLength: 24_000_000,
        open(socket) { bridge.open(socket); },
        close(socket) { bridge.close(socket); },
        message(socket, raw) { bridge.message(socket, String(raw)); },
      },
    });
    listenError = null;
    console.error(`[luna-mcp] ws://127.0.0.1:${listener.port}`);
  } catch (error) { listenError = String(error); console.error(listenError); }
}
escuchar();

const camera = { type: "string", enum: ["auto", "desktop", "spectator"], description: "auto: spectator in VR, desktop otherwise" };
const vector = { type: "array", items: { type: "number" }, minItems: 3, maxItems: 3 };
const tools = [
  { name: "luna_logs", description: "Read retained host/page logs; optional tab, space, level and Rust regex filters. First request returns latest N, after pages forward using nextCursor. Read-only.", inputSchema: { type: "object", properties: { limit: { type: "integer", minimum: 1, maximum: 300 }, after: { type: "integer", minimum: 0 }, tabId: { type: "integer", minimum: 0 }, spaceId: { type: "integer", minimum: 0 }, level: { type: "string", enum: ["info", "warn", "error"] }, pattern: { type: "string", maxLength: 512 } } } },
  { name: "luna_status", description: "Current pages, requested/effective render mode, XR state and cameras. Queries Luna now.", inputSchema: { type: "object", properties: {} } },
  { name: "luna_open", description: "Navigate the spatial page. Returns the host-assigned tab ID and loading:true once queued.", inputSchema: { type: "object", properties: { url: { type: "string" } }, required: ["url"] } },
  { name: "luna_camera", description: "Set position and lookAt in world coordinates (meters). Spectator is independent of the VR headset.", inputSchema: { type: "object", properties: { camera, position: vector, lookAt: vector }, required: ["position", "lookAt"] } },
  { name: "luna_capture", description: "Capture a rendered PNG. Spectator works in VR. Desktop requires its camera to be active.", inputSchema: { type: "object", properties: { camera } } },
  { name: "luna_wait", description: "Wait up to 20000 ms. Does not assert that loading has finished.", inputSchema: { type: "object", properties: { ms: { type: "integer", minimum: 0, maximum: 20000 } }, required: ["ms"] } },
];
const server = new Server({ name: "luna", version: "0.2.0" }, { capabilities: { tools: {} } });
server.setRequestHandler(ListToolsRequestSchema, async () => ({ tools }));
function text(value: unknown, isError = false) { return { content: [{ type: "text" as const, text: typeof value === "string" ? value : JSON.stringify(value, null, 2) }], isError }; }
server.setRequestHandler(CallToolRequestSchema, async ({ params }) => {
  const args = params.arguments ?? {};
  try {
    escuchar();
    if (listenError) throw new Error(`Local listener unavailable: ${listenError} (otra sesión puede tener el puerto ${port}; se reintenta en cada llamada)`);
    const selected = args.camera ?? "auto";
    if (!["auto", "desktop", "spectator"].includes(selected as string)) throw new Error("Invalid camera");
    switch (params.name) {
      case "luna_logs": {
        for (const key of ["limit", "after", "tabId", "spaceId"]) {
          if (args[key] !== undefined && (!Number.isSafeInteger(args[key]) || Number(args[key]) < 0)) throw new Error(`Invalid ${key}`);
        }
        if (args.limit !== undefined && (Number(args.limit) < 1 || Number(args.limit) > 300)) throw new Error("Invalid limit");
        if (args.level !== undefined && !["info", "warn", "error"].includes(String(args.level))) throw new Error("Invalid level");
        if (args.pattern !== undefined && (typeof args.pattern !== "string" || args.pattern.length > 512)) throw new Error("Invalid pattern");
        return text(await bridge.request("logs", args));
      }
      case "luna_status": return text(bridge.connected ? await bridge.request("status") : { connected: false, message: "Inicia MCP local desde Ajustes de Luna." });
      case "luna_open": {
        if (typeof args.url !== "string" || args.url.length > 8192 || !["luna:", "http:", "https:"].includes(new URL(args.url).protocol)) throw new Error("Expected luna://, http:// or https:// URL");
        return text(await bridge.request("open", { url: args.url }));
      }
      case "luna_camera": {
        for (const key of ["position", "lookAt"]) {
          const v = args[key];
          if (!Array.isArray(v) || v.length !== 3 || !v.every(n => typeof n === "number" && Number.isFinite(n) && Math.abs(n) <= 1e6)) throw new Error(`Invalid ${key}`);
        }
        return text(await bridge.request("camera", { camera: selected, position: args.position, lookAt: args.lookAt }));
      }
      case "luna_capture": return pngResult(await bridge.request("capture", { camera: selected }));
      case "luna_wait": {
        const ms = args.ms;
        if (typeof ms !== "number" || !Number.isInteger(ms) || ms < 0 || ms > 20000) throw new Error("ms must be an integer between 0 and 20000");
        await new Promise(resolve => setTimeout(resolve, ms));
        return text({ waited: ms });
      }
      default: throw new Error(`Unknown tool: ${params.name}`);
    }
  } catch (error) { return text(error instanceof Error ? error.message : String(error), true); }
});
server.onclose = () => { listener?.stop(true); };
await server.connect(new StdioServerTransport());
