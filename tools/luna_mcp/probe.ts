// Manual diagnostic through MCP, without a second ad-hoc WebSocket protocol.
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
const client = new Client({name:"luna-probe",version:"2"});
try {
  await client.connect(new StdioClientTransport({command:"bun",args:["run","src/index.ts"],cwd:import.meta.dir,
    env:process.env as Record<string,string>,stderr:"inherit"}));
  const result = await client.callTool({name:"luna_status"});
  console.log(JSON.stringify(result,null,2));
  if(result.isError) process.exitCode = 1;
} catch(error) { console.error(error); process.exitCode = 1; }
finally { await client.close(); }
