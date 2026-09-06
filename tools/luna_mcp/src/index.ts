// Servidor MCP para Luna.
//
// Dos patas:
//   1. Habla MCP por stdio con el cliente (Claude Code).
//   2. Escucha WebSocket en 2054, que es donde se conecta el puente
//      (`luna://agent_app`) corriendo dentro del navegador.
//
// El que escucha es este proceso y no Luna porque el runtime del navegador sólo
// sabe ser cliente WebSocket (crates/luna/src/ws.rs usa connect_async). Todo lo
// que hace este archivo es traducir una llamada de herramienta MCP en un
// mensaje JSON para el puente, y esperar su respuesta.
import { readFileSync } from "fs";
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";

const PUERTO = Number(process.env.LUNA_AGENT_PORT) || 2054;
const TIMEOUT_MS = Number(process.env.LUNA_AGENT_TIMEOUT) || 30_000;

// ── Puente ────────────────────────────────────────────────────────────────

type Pendiente = {
  resolve: (valor: any) => void;
  reject: (err: Error) => void;
  timer: ReturnType<typeof setTimeout>;
};

let puente: any = null; // el socket del navegador, si está conectado
let siguienteId = 1;
const pendientes = new Map<number, Pendiente>();

/** Manda un comando al puente y espera su respuesta. */
function pedir(cmd: string, args: Record<string, unknown> = {}): Promise<any> {
  if (!puente) {
    return Promise.reject(
      new Error(
        "Luna no está conectada. Abrí luna://agent en el navegador y tocá 'Iniciar agente'.",
      ),
    );
  }
  const id = siguienteId++;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pendientes.delete(id);
      reject(new Error(`timeout de ${TIMEOUT_MS} ms esperando '${cmd}'`));
    }, TIMEOUT_MS);
    pendientes.set(id, { resolve, reject, timer });
    puente.send(JSON.stringify({ id, cmd, args }));
  });
}

// Si el puerto está ocupado (otra instancia, una sonda de test) el server no
// puede levantar. Eso NO tiene que matar al proceso: el cliente MCP lo vería
// como "servidor roto" en vez de como lo que es. Seguimos vivos y que
// luna_status lo explique.
let errorPuerto: string | null = null;

try {
  Bun.serve({
  port: PUERTO,
  fetch(req, server) {
    if (server.upgrade(req)) return;
    return new Response("luna-mcp: este endpoint es sólo WebSocket", { status: 426 });
  },
  websocket: {
    open(ws) {
      // Sólo se maneja un puente a la vez. Si ya había uno, el nuevo lo pisa:
      // avisamos, porque si no una segunda instancia de Luna (o una sonda de
      // test) secuestra la sesión sin dejar rastro.
      if (puente) {
        console.error("[luna-mcp] OJO: ya había un puente conectado, lo reemplaza el nuevo");
      }
      puente = ws;
      console.error(`[luna-mcp] puente conectado`);
    },
    close() {
      puente = null;
      // Cortamos lo que quedó esperando: mejor un error claro que un cuelgue.
      for (const [id, p] of pendientes) {
        clearTimeout(p.timer);
        p.reject(new Error("el puente se desconectó"));
        pendientes.delete(id);
      }
      console.error("[luna-mcp] puente desconectado");
    },
    message(_ws, raw) {
      let msg: any;
      try {
        msg = JSON.parse(String(raw));
      } catch {
        return;
      }
      if (msg.hello) {
        console.error(`[luna-mcp] hola de ${msg.hello} v${msg.version}: ${(msg.comandos || []).join(", ")}`);
        return;
      }
      const p = pendientes.get(msg.id);
      if (!p) return;
      clearTimeout(p.timer);
      pendientes.delete(msg.id);
      if (msg.ok) p.resolve(msg.result);
      else p.reject(new Error(msg.error || "error desconocido en el puente"));
    },
  },
  });
  console.error(`[luna-mcp] escuchando ws://127.0.0.1:${PUERTO}`);
} catch (err: any) {
  errorPuerto = String(err?.message || err);
  console.error(`[luna-mcp] no se pudo escuchar en ${PUERTO}: ${errorPuerto}`);
}

// ── Herramientas MCP ──────────────────────────────────────────────────────

const HERRAMIENTAS = [
  {
    name: "luna_status",
    description:
      "Dice si el puente de Luna está conectado. Útil para diagnosticar antes de intentar cualquier otra cosa.",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "luna_open",
    description:
      "Abre una URL en Luna como espacio spatial. Abrir un spatial cierra el anterior, así que sirve igual para navegar y para recargar la escena actual.",
    inputSchema: {
      type: "object",
      properties: {
        url: { type: "string", description: "URL del documento HSML (http:// o luna://)" },
      },
      required: ["url"],
    },
  },
  {
    name: "luna_wait",
    description:
      "Espera dentro de Luna. Pensado para darle tiempo a que carguen los includes, los modelos y los scripts antes de capturar.",
    inputSchema: {
      type: "object",
      properties: {
        ms: { type: "number", description: "milisegundos (máx 20000)" },
      },
      required: ["ms"],
    },
  },
  {
    name: "luna_capture",
    description:
      "Captura el frame que Luna está renderizando y devuelve la imagen. Es el render real del motor, no una aproximación.",
    inputSchema: {
      type: "object",
      properties: {
        name: { type: "string", description: "nombre del archivo, sin ruta" },
      },
    },
  },
];

const server = new Server(
  { name: "luna", version: "0.1.0" },
  { capabilities: { tools: {} } },
);

server.setRequestHandler(ListToolsRequestSchema, async () => ({ tools: HERRAMIENTAS }));

server.setRequestHandler(CallToolRequestSchema, async (peticion) => {
  const { name, arguments: args = {} } = peticion.params as {
    name: string;
    arguments?: Record<string, any>;
  };

  try {
    switch (name) {
      case "luna_status":
        if (errorPuerto) {
          return texto(
            `no pude escuchar en ${PUERTO}: ${errorPuerto}. Probablemente haya otra instancia del servidor corriendo.`,
            true,
          );
        }
        return texto(
          puente
            ? "puente conectado"
            : "puente desconectado — abrí luna://agent en Luna y tocá 'Iniciar agente'",
        );

      case "luna_open": {
        const r = await pedir("open", { url: args.url });
        return texto(`abierto ${r.url} (tab ${r.tabId})`);
      }

      case "luna_wait": {
        const r = await pedir("wait", { ms: args.ms });
        return texto(`esperé ${r.waited} ms`);
      }

      case "luna_capture": {
        const r = await pedir("capture", { name: args.name });
        // El puente resuelve recién cuando el PNG está cerrado en disco, así
        // que acá se puede leer sin esperar nada más.
        const png = readFileSync(r.path);
        return {
          content: [
            { type: "text", text: `captura: ${r.path} (${png.length} bytes)` },
            { type: "image", data: png.toString("base64"), mimeType: "image/png" },
          ],
        };
      }

      default:
        return texto(`herramienta desconocida: ${name}`, true);
    }
  } catch (err: any) {
    return texto(String(err?.message || err), true);
  }
});

function texto(t: string, esError = false) {
  return { content: [{ type: "text", text: t }], isError: esError };
}

await server.connect(new StdioServerTransport());
console.error("[luna-mcp] listo (stdio)");
