// Prueba de humo: levanta el server MCP, le enchufa un puente falso por
// WebSocket y hace un ida y vuelta completo por stdio. No necesita Luna.
const proc = Bun.spawn(["bun", "run", "src/index.ts"], {
  stdin: "pipe",
  stdout: "pipe",
  stderr: "inherit",
  cwd: import.meta.dir,
});

const enc = new TextEncoder();
function enviar(msg: unknown) {
  proc.stdin.write(enc.encode(JSON.stringify(msg) + "\n"));
  proc.stdin.flush();
}

const respuestas: any[] = [];
(async () => {
  const dec = new TextDecoder();
  let buf = "";
  for await (const chunk of proc.stdout) {
    buf += dec.decode(chunk);
    let i;
    while ((i = buf.indexOf("\n")) >= 0) {
      const linea = buf.slice(0, i).trim();
      buf = buf.slice(i + 1);
      if (linea) respuestas.push(JSON.parse(linea));
    }
  }
})();

const esperar = (ms: number) => new Promise((r) => setTimeout(r, ms));
const buscar = (id: number) => respuestas.find((r) => r.id === id);

await esperar(700);

enviar({
  jsonrpc: "2.0", id: 1, method: "initialize",
  params: { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "test", version: "0" } },
});
await esperar(300);
enviar({ jsonrpc: "2.0", method: "notifications/initialized" });
enviar({ jsonrpc: "2.0", id: 2, method: "tools/list" });
await esperar(300);

// Sin puente: status debe avisar que no hay nadie.
enviar({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "luna_status", arguments: {} } });
await esperar(300);

// Ahora enchufamos un puente falso que responde como lo haría Luna.
const ws = new WebSocket("ws://127.0.0.1:2054");
await new Promise<void>((res, rej) => {
  ws.onopen = () => res();
  ws.onerror = () => rej(new Error("no se pudo conectar el puente falso"));
});
ws.send(JSON.stringify({ hello: "fake-bridge", version: 1, comandos: ["open", "capture"] }));
ws.onmessage = (evt) => {
  const msg = JSON.parse(String(evt.data));
  if (msg.cmd === "open") {
    ws.send(JSON.stringify({ id: msg.id, ok: true, result: { tabId: 7, url: msg.args.url } }));
  } else if (msg.cmd === "capture") {
    ws.send(JSON.stringify({ id: msg.id, ok: false, error: "puente falso: sin GPU" }));
  }
};
await esperar(400);

enviar({ jsonrpc: "2.0", id: 4, method: "tools/call", params: { name: "luna_status", arguments: {} } });
enviar({ jsonrpc: "2.0", id: 5, method: "tools/call", params: { name: "luna_open", arguments: { url: "http://localhost:2053/index.hsml" } } });
enviar({ jsonrpc: "2.0", id: 6, method: "tools/call", params: { name: "luna_capture", arguments: { name: "x" } } });
await esperar(900);

const nombres = (buscar(2)?.result?.tools || []).map((t: any) => t.name);
const linea = (id: number) => buscar(id)?.result?.content?.[0]?.text ?? "(sin respuesta)";

console.log("initialize   :", buscar(1)?.result?.serverInfo?.name ?? "FALLÓ");
console.log("tools/list   :", nombres.join(", ") || "FALLÓ");
console.log("status (sin) :", linea(3));
console.log("status (con) :", linea(4));
console.log("open         :", linea(5));
console.log("capture err  :", linea(6), "| isError:", buscar(6)?.result?.isError);

const ok =
  buscar(1)?.result?.serverInfo?.name === "luna" &&
  nombres.length === 4 &&
  linea(3).includes("desconectado") &&
  linea(4).includes("conectado") &&
  linea(5).includes("tab 7") &&
  buscar(6)?.result?.isError === true;

console.log(ok ? "\nOK: ida y vuelta completo" : "\nFALLÓ");
ws.close();
proc.kill();
process.exit(ok ? 0 : 1);
