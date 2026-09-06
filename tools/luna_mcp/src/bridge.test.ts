import { expect, test } from "bun:test";
import { Bridge, pngResult } from "./bridge";
function socket() { return { sent: [] as any[], send(data: string) { this.sent.push(JSON.parse(data)); }, close() {} }; }
test("replacement rejects old requests; old close and replies cannot affect new socket", async () => {
  const bridge = new Bridge(1000), a = socket(), b = socket();
  bridge.open(a);
  const old = bridge.request("status").catch(e => e.message);
  bridge.open(b);
  expect(await old).toContain("replaced");
  bridge.close(a);
  expect(bridge.connected).toBe(true);
  const current = bridge.request("status");
  const id = b.sent[0].id;
  bridge.message(a, JSON.stringify({ id, ok: true, result: "wrong" }));
  bridge.message(b, JSON.stringify({ id, ok: true, result: "current" }));
  expect(await current).toBe("current");
});
test("timeouts and send errors settle promises", async () => {
  const bridge = new Bridge(10), s = socket(); bridge.open(s);
  await expect(bridge.request("status")).rejects.toThrow("Timeout");
  bridge.open({ send() { throw new Error("closed"); }, close() {} });
  await expect(bridge.request("status")).rejects.toThrow("closed");
});
test("captures accept image bytes, never paths", () => {
  expect(() => pngResult({path:"C:/private.txt"})).toThrow();
  expect(() => pngResult({mimeType:"image/png",data:Buffer.from("text").toString("base64")})).toThrow();
});
