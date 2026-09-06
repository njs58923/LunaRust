type Socket = { send(data: string): unknown; close(code?: number, reason?: string): unknown };
type Pending = { resolve(value: unknown): void; reject(error: Error): void; timer: ReturnType<typeof setTimeout> };

/** Replies belong to the socket that received the request. No session registry. */
export class Bridge {
  private socket: Socket | null = null;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  constructor(private timeout = 30_000) {}
  get connected() { return this.socket !== null; }
  open(socket: Socket) {
    const old = this.socket;
    this.failPending("Luna connection replaced");
    this.socket = socket;
    old?.close(1000, "replaced");
  }
  close(socket: Socket) {
    if (this.socket !== socket) return;
    this.socket = null;
    this.failPending("Luna disconnected");
  }
  private failPending(message: string) {
    for (const p of this.pending.values()) { clearTimeout(p.timer); p.reject(new Error(message)); }
    this.pending.clear();
  }
  message(socket: Socket, raw: string) {
    if (socket !== this.socket) return;
    let value: any;
    try { value = JSON.parse(raw); } catch { return; }
    if (!value || !Number.isSafeInteger(value.id) || typeof value.ok !== "boolean") return;
    const p = this.pending.get(value.id);
    if (!p) return;
    clearTimeout(p.timer); this.pending.delete(value.id);
    if (value.ok) p.resolve(value.result);
    else p.reject(new Error(typeof value.error === "string" ? value.error : "Luna command failed"));
  }
  request(command: string, args: Record<string, unknown> = {}): Promise<any> {
    const socket = this.socket;
    if (!socket) return Promise.reject(new Error("Luna desconectada: inicia MCP local desde Ajustes."));
    if (this.pending.size >= 32) return Promise.reject(new Error("Too many pending requests"));
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`Timeout: ${command}`)); }, this.timeout);
      this.pending.set(id, { resolve, reject, timer });
      try { socket.send(JSON.stringify({ id, cmd: command, args })); }
      catch (error) { clearTimeout(timer); this.pending.delete(id); reject(error); }
    });
  }
}

export function pngResult(value: any) {
  if (value?.mimeType !== "image/png" || typeof value.data !== "string" || value.data.length > 24_000_000
    || !/^[A-Za-z0-9+/]*={0,2}$/.test(value.data)) throw new Error("Invalid PNG response from Luna");
  const bytes = Buffer.from(value.data, "base64");
  if (!bytes.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10]))) throw new Error("Invalid PNG signature");
  return { content: [{ type: "image" as const, data: value.data, mimeType: "image/png" }] };
}
