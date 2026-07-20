import path from "node:path";
import process from "node:process";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

const protocolVersion = "2025-06-18";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

const binary = process.env.PORTMATE_MCP_BINARY
  ? path.resolve(process.env.PORTMATE_MCP_BINARY)
  : path.resolve(
    "target",
    "debug",
    process.platform === "win32" ? "portmate-mcp.exe" : "portmate-mcp",
  );
const transport = new StdioClientTransport({
  command: binary,
  cwd: process.cwd(),
  env: {
    ...process.env,
    PORTMATE_MCP_HTTP: "0",
    PORTMATE_MCP_CLIENT_ID: "official-sdk-stdio-check",
  },
  stderr: "pipe",
});
let negotiatedProtocolVersion = null;
transport.setProtocolVersion = (version) => {
  negotiatedProtocolVersion = version;
};
let stderr = "";
transport.stderr?.on("data", (chunk) => {
  stderr = `${stderr}${chunk.toString()}`.slice(-64 * 1024);
});

const sent = [];
const send = transport.send.bind(transport);
transport.send = async (message) => {
  sent.push(message);
  await send(message);
};

const client = new Client(
  { name: "portmate-stdio-client-check", version: "1.0.0" },
  { capabilities: {} },
);

try {
  await client.connect(transport);
  assert(transport.pid, "official SDK did not start the PortMate stdio bridge");
  assert(client.getServerVersion()?.name === "portmate-mcp", "official SDK did not initialize PortMate over stdio");

  await client.ping();
  const tools = await client.listTools();
  assert(tools.tools.some((tool) => tool.name === "list_sessions"), "tools/list omitted list_sessions");
  const resources = await client.listResources();
  assert(resources.resources.some((resource) => resource.uri === "portmate://sessions"), "resources/list omitted portmate://sessions");
  const templates = await client.listResourceTemplates();
  assert(templates.resourceTemplates.some((template) => template.uriTemplate.startsWith("portmate://sessions/{id}/")), "resources/templates/list omitted session templates");
  const prompts = await client.listPrompts();
  assert(prompts.prompts.length > 0, "prompts/list returned no prompts");
  const sessions = await client.readResource({ uri: "portmate://sessions" });
  assert(sessions.contents[0]?.mimeType === "application/json", "resources/read returned the wrong sessions MIME type");

  const initialize = sent.find((message) => message.method === "initialize");
  const initialized = sent.find((message) => message.method === "notifications/initialized");
  assert(initialize, "official SDK did not send initialize over stdio");
  assert(initialized, "official SDK did not send notifications/initialized over stdio");
  assert(typeof initialize.params?.protocolVersion === "string", "official SDK omitted its preferred MCP version");
  assert(negotiatedProtocolVersion === protocolVersion, "PortMate negotiated an unexpected stdio MCP version");

  console.log(`MCP stdio official SDK check passed (${sent.length} messages)`);
} catch (error) {
  if (stderr) {
    error.message = `${error.message}\nPortMate stderr:\n${stderr}`;
  }
  throw error;
} finally {
  await client.close().catch(() => {});
}

assert(transport.pid === null, "PortMate stdio bridge process survived client shutdown");
