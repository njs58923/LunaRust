// Integration test: actual MCP stdio server + fake native host on an ephemeral port.
import { test, expect } from "bun:test";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

test("stdio tools, live status, camera, PNG, validation and reconnect", async () => {
  const transport = new StdioClientTransport({ command: "bun", args: ["run", "src/index.ts"], cwd: import.meta.dir,
    env: { ...process.env as Record<string,string>, LUNA_AGENT_PORT: "0" }, stderr: "pipe" });
  const endpoint = new Promise<string>((resolve,reject) => {
    let output = "";
    const timer = setTimeout(() => reject(new Error("listener startup timeout")), 5000);
    transport.stderr!.on("data", data => { output += String(data); const match = output.match(/ws:\/\/127\.0\.0\.1:\d+/); if(match) {clearTimeout(timer); resolve(match[0]);} });
  });
  const client = new Client({name:"test",version:"1"});
  let host: WebSocket | undefined;
  try {
    await client.connect(transport);
    const url = await endpoint;
    expect((await fetch(url.replace("ws:","http:"), {headers:{Origin:"https://example.org"}})).status).toBe(403);
    const tools = await client.listTools(); expect(tools.tools).toHaveLength(5);
    const connect = () => new Promise<WebSocket>((resolve,reject) => {
      const ws = new WebSocket(url); ws.onopen = () => resolve(ws); ws.onerror = reject;
      ws.onmessage = event => {
        const req = JSON.parse(String(event.data));
        const result = req.cmd === "status" ? {connected:true,requestedMode:"vr",xrState:"Running",spaces:[{url:"luna://home"}]}
          : req.cmd === "open" ? {tabId:42,url:req.args.url,accepted:true,loading:true}
          : req.cmd === "capture" ? {mimeType:"image/png",data:"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII="}
          : req.args;
        ws.send(JSON.stringify({id:req.id,ok:true,result}));
      };
    });
    host = await connect();
    const status = await client.callTool({name:"luna_status"});
    expect(JSON.stringify(status)).toContain("Running");
    const open = await client.callTool({name:"luna_open",arguments:{url:"luna://home"}});
    expect(JSON.stringify(open)).toContain("42"); expect(JSON.stringify(open)).toContain("loading");
    const camera = await client.callTool({name:"luna_camera",arguments:{position:[0,2,4],lookAt:[0,1,0],camera:"spectator"}});
    expect(camera.isError).toBe(false);
    const capture = await client.callTool({name:"luna_capture"});
    expect((capture.content as any[])[0].type).toBe("image");
    expect((await client.callTool({name:"luna_wait",arguments:{ms:-1}})).isError).toBe(true);
    expect((await client.callTool({name:"luna_open",arguments:{url:"file:///secret"}})).isError).toBe(true);
    const old = host; host = await connect(); old.close();
    expect((await client.callTool({name:"luna_status"})).isError).toBe(false);
  } finally { host?.close(); await client.close(); }
}, 15000);
