// Sonda: hace de servidor (el mismo rol que luna_mcp) y maneja el puente real
// que corre dentro de Luna. Sirve para probar el camino completo —permiso, op,
// captura de Bevy, escritura del PNG— sin depender de que el MCP esté
// registrado en el cliente.
//
//   bun run probe.ts [url-a-abrir]
const PUERTO = 2054;
const TIMEOUT_MS = 900_000;
const urlAbrir = process.argv[2];

let ws: any = null;
let id = 1;
const pendientes = new Map<number, (r: any) => void>();
const log: string[] = [];

function decir(s: string) {
  log.push(s);
  console.log(s);
}

function pedir(cmd: string, args: any = {}): Promise<any> {
  return new Promise((resolve) => {
    const mio = id++;
    pendientes.set(mio, resolve);
    ws.send(JSON.stringify({ id: mio, cmd, args }));
  });
}

const server = Bun.serve({
  port: PUERTO,
  fetch(req, s) {
    if (s.upgrade(req)) return;
    return new Response("sonda", { status: 426 });
  },
  websocket: {
    async open(sock) {
      ws = sock;
      decir("puente conectado");
      try {
        const pong = await pedir("ping");
        decir("ping -> " + JSON.stringify(pong));

        if (urlAbrir) {
          const abierto = await pedir("open", { url: urlAbrir });
          decir("open -> " + JSON.stringify(abierto));
          const esperado = await pedir("wait", { ms: 3000 });
          decir("wait -> " + JSON.stringify(esperado));
        }

        const cap = await pedir("capture", { name: "sonda" });
        decir("capture -> " + JSON.stringify(cap));
      } catch (err: any) {
        decir("ERROR: " + String(err?.message || err));
      }
      terminar(0);
    },
    message(_s, raw) {
      let msg: any;
      try {
        msg = JSON.parse(String(raw));
      } catch {
        return;
      }
      if (msg.hello) {
        decir("hola: " + JSON.stringify(msg));
        return;
      }
      const r = pendientes.get(msg.id);
      if (!r) return;
      pendientes.delete(msg.id);
      r(msg.ok ? msg.result : { ERROR: msg.error });
    },
    close() {
      decir("puente desconectado");
    },
  },
});

decir(`sonda escuchando en ws://127.0.0.1:${PUERTO} — esperando a Luna`);

function terminar(code: number) {
  Bun.write("probe_out.txt", log.join("\n") + "\n");
  server.stop(true);
  process.exit(code);
}

setTimeout(() => {
  decir("TIMEOUT: el puente nunca se conectó");
  terminar(1);
}, TIMEOUT_MS);
