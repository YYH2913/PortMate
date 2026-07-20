import { spawn } from "node:child_process";
import { createServer } from "node:net";
import path from "node:path";
import process from "node:process";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

const protocolVersion = "2025-06-18";
const token = "portmate-mcp-http-client-check";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function reservePort() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  const port = typeof address === "object" && address ? address.port : 0;
  await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
  if (!port) throw new Error("failed to reserve an MCP HTTP test port");
  return port;
}

async function waitForServer(url, output) {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    try {
      const response = await fetch(url, { method: "OPTIONS" });
      if (response.status === 204) return;
    } catch {
      // The Rust process is still starting.
    }
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(`timed out waiting for ${url}\n${output()}`);
}

function requestMethod(entry) {
  if (!entry.body) return null;
  try {
    return JSON.parse(entry.body).method ?? null;
  } catch {
    return null;
  }
}

const port = await reservePort();
const endpoint = new URL(`http://127.0.0.1:${port}/mcp`);
const binary = process.env.PORTMATE_MCP_BINARY
  ? path.resolve(process.env.PORTMATE_MCP_BINARY)
  : path.resolve(
    "target",
    "debug",
    process.platform === "win32" ? "portmate-mcp.exe" : "portmate-mcp",
  );
let serverOutput = "";
const server = spawn(binary, ["--http"], {
  cwd: process.cwd(),
  env: {
    ...process.env,
    PORTMATE_MCP_HTTP_ADDR: `127.0.0.1:${port}`,
    PORTMATE_MCP_HTTP_TOKEN: token,
    PORTMATE_MCP_CLIENT_ID: "official-sdk-http-check",
  },
  stdio: ["ignore", "pipe", "pipe"],
});
server.stdout.on("data", (chunk) => { serverOutput += chunk.toString(); });
server.stderr.on("data", (chunk) => { serverOutput += chunk.toString(); });

const requests = [];
const trackedFetch = async (input, init = {}) => {
  const headers = new Headers(init.headers);
  requests.push({
    method: init.method ?? "GET",
    authorization: headers.get("authorization"),
    accept: headers.get("accept"),
    protocolVersion: headers.get("mcp-protocol-version"),
    body: typeof init.body === "string" ? init.body : "",
  });
  return fetch(input, init);
};

let client;
try {
  await waitForServer(endpoint, () => serverOutput);
  const transport = new StreamableHTTPClientTransport(endpoint, {
    requestInit: { headers: { Authorization: `Bearer ${token}` } },
    fetch: trackedFetch,
  });
  client = new Client(
    { name: "portmate-http-client-check", version: "1.0.0" },
    { capabilities: {} },
  );

  await client.connect(transport);
  assert(client.getServerVersion()?.name === "portmate-mcp", "official SDK did not initialize PortMate");
  assert(transport.protocolVersion === protocolVersion, "official SDK negotiated the wrong MCP version");
  assert(transport.sessionId === undefined, "PortMate's documented stateless HTTP mode unexpectedly created a session");

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

  await new Promise((resolve) => setTimeout(resolve, 100));
  const initialize = requests.find((request) => requestMethod(request) === "initialize");
  const initialized = requests.find((request) => requestMethod(request) === "notifications/initialized");
  const eventStream = requests.find((request) => request.method === "GET");
  assert(initialize, "official SDK did not send initialize");
  assert(initialized, "official SDK did not send notifications/initialized");
  assert(eventStream, "official SDK did not open the optional GET event stream");
  assert(initialize.accept === "application/json, text/event-stream", "initialize did not advertise both Streamable HTTP response types");
  assert(eventStream.accept === "text/event-stream", "GET event stream used the wrong Accept header");

  for (const request of requests) {
    assert(request.authorization === `Bearer ${token}`, `${request.method} request omitted HTTP authentication`);
    if (request !== initialize) {
      assert(request.protocolVersion === protocolVersion, `${request.method} request omitted the negotiated MCP version`);
    }
  }

  console.log(`MCP HTTP official SDK check passed (${requests.length} requests)`);
} finally {
  await client?.close().catch(() => {});
  server.kill("SIGTERM");
  await new Promise((resolve) => {
    if (server.exitCode !== null) {
      resolve();
      return;
    }
    server.once("exit", resolve);
    setTimeout(() => {
      server.kill("SIGKILL");
      resolve();
    }, 2_000).unref();
  });
}
