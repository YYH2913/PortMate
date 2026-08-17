import { spawn } from "node:child_process";
import { createServer } from "node:net";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const sdkRoot = process.env.PORTMATE_MCP_TYPESCRIPT_SDK_ROOT?.trim();
const sdkModule = (relativePath, packagePath) => sdkRoot
  ? pathToFileURL(path.join(sdkRoot, "dist", "esm", ...relativePath)).href
  : packagePath;
const { Client } = await import(sdkModule(["client", "index.js"], "@modelcontextprotocol/sdk/client/index.js"));
const { StreamableHTTPClientTransport } = await import(sdkModule(
  ["client", "streamableHttp.js"],
  "@modelcontextprotocol/sdk/client/streamableHttp.js",
));
const protocolVersion = process.env.PORTMATE_MCP_EXPECTED_PROTOCOL_VERSION?.trim() || "2025-06-18";
const requestedProtocolVersion = process.env.PORTMATE_MCP_EXPECTED_REQUEST_PROTOCOL_VERSION?.trim();
const sdkVersion = process.env.PORTMATE_MCP_TYPESCRIPT_SDK_VERSION?.trim() || "root";
const expectsProtocolHeader = process.env.PORTMATE_MCP_TYPESCRIPT_EXPECT_PROTOCOL_HEADER !== "0";
const token = "portmate-mcp-http-client-check";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function socketAddress(host, port) {
  return host.includes(":") ? `[${host}]:${port}` : `${host}:${port}`;
}

function httpEndpoint(host, port) {
  return new URL(`http://${socketAddress(host, port)}/mcp`);
}

async function reservePort(host = "127.0.0.1") {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, host, resolve);
  });
  const address = server.address();
  const port = typeof address === "object" && address ? address.port : 0;
  await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
  if (!port) throw new Error("failed to reserve an MCP HTTP test port");
  return port;
}

async function stopServer(server) {
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

function requestParams(entry) {
  if (!entry.body) return null;
  try {
    return JSON.parse(entry.body).params ?? null;
  } catch {
    return null;
  }
}

async function verifyRemoteBindRequiresOptIn(binary, bindHost, portHost = "127.0.0.1") {
  const port = await reservePort(portHost);
  let output = "";
  const denied = spawn(binary, ["--http"], {
    cwd: process.cwd(),
    env: {
      ...process.env,
      PORTMATE_MCP_HTTP_ADDR: socketAddress(bindHost, port),
      PORTMATE_MCP_HTTP_ALLOW_REMOTE: "0",
      PORTMATE_MCP_HTTP_TOKEN: token,
      PORTMATE_STORE_PATH: "",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  denied.stdout.on("data", (chunk) => { output += chunk.toString(); });
  denied.stderr.on("data", (chunk) => { output += chunk.toString(); });
  const code = await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      denied.kill("SIGKILL");
      reject(new Error(`MCP HTTP accepted a remote bind without opt-in\n${output}`));
    }, 2_000);
    denied.once("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });
    denied.once("exit", (exitCode) => {
      clearTimeout(timeout);
      resolve(exitCode);
    });
  });
  assert(code !== 0, "MCP HTTP remote bind without opt-in exited successfully");
  assert(output.includes("PORTMATE_MCP_HTTP_ALLOW_REMOTE=1"), "MCP HTTP remote-bind rejection omitted the opt-in diagnostic");
}

async function verifyIpv6Listeners(binary) {
  let loopbackPort;
  try {
    loopbackPort = await reservePort("::1");
  } catch (error) {
    if (["EAFNOSUPPORT", "EADDRNOTAVAIL", "EPROTONOSUPPORT"].includes(error?.code)) {
      console.log(`MCP IPv6 HTTP checks skipped: ${error.code}`);
      return;
    }
    throw error;
  }

  await verifyIpv6Listener(binary, "::1", "::1", loopbackPort, false);
  await verifyRemoteBindRequiresOptIn(binary, "::", "::1");
  const wildcardPort = await reservePort("::1");
  await verifyIpv6Listener(binary, "::", "::1", wildcardPort, true);
  console.log("MCP IPv6 HTTP listener checks passed (::1 and ::)");
}

async function verifyIpv6Listener(binary, bindHost, connectHost, port, allowRemote) {
  const endpoint = httpEndpoint(connectHost, port);
  const origin = `http://${socketAddress(connectHost, port)}`;
  let output = "";
  const server = spawn(binary, ["--http"], {
    cwd: process.cwd(),
    env: {
      ...process.env,
      PORTMATE_MCP_HTTP_ADDR: socketAddress(bindHost, port),
      PORTMATE_MCP_HTTP_ALLOW_REMOTE: allowRemote ? "1" : "0",
      PORTMATE_MCP_HTTP_ORIGINS: origin,
      PORTMATE_MCP_HTTP_TOKEN: token,
      PORTMATE_MCP_CLIENT_ID: "official-sdk-ipv6-check",
      PORTMATE_STORE_PATH: "",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  server.stdout.on("data", (chunk) => { output += chunk.toString(); });
  server.stderr.on("data", (chunk) => { output += chunk.toString(); });

  try {
    await waitForServer(endpoint, () => output);
    const response = await fetch(endpoint, {
      method: "POST",
      headers: {
        Accept: "application/json",
        Authorization: `Bearer ${token}`,
        "Content-Type": "application/json",
        Origin: origin,
      },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping", params: {} }),
    });
    assert(response.status === 200, `MCP IPv6 listener ${bindHost} returned HTTP ${response.status}`);
    const body = await response.json();
    assert(body.jsonrpc === "2.0" && body.id === 1 && body.result,
      `MCP IPv6 listener ${bindHost} returned an invalid ping response`);
  } finally {
    await stopServer(server);
  }
}

const binary = process.env.PORTMATE_MCP_BINARY
  ? path.resolve(process.env.PORTMATE_MCP_BINARY)
  : path.resolve(
    "target",
    "debug",
    process.platform === "win32" ? "portmate-mcp.exe" : "portmate-mcp",
  );
await verifyRemoteBindRequiresOptIn(binary, "0.0.0.0");
await verifyIpv6Listeners(binary);

const port = await reservePort();
const endpoint = httpEndpoint("127.0.0.1", port);
let serverOutput = "";
const server = spawn(binary, ["--http"], {
  cwd: process.cwd(),
  env: {
    ...process.env,
    PORTMATE_MCP_HTTP_ADDR: `0.0.0.0:${port}`,
    PORTMATE_MCP_HTTP_ALLOW_REMOTE: "1",
    PORTMATE_MCP_HTTP_ORIGINS: `http://127.0.0.1:${port}`,
    PORTMATE_MCP_HTTP_TOKEN: token,
    PORTMATE_MCP_CLIENT_ID: "official-sdk-http-check",
    PORTMATE_STORE_PATH: "",
  },
  stdio: ["ignore", "pipe", "pipe"],
});
server.stdout.on("data", (chunk) => { serverOutput += chunk.toString(); });
server.stderr.on("data", (chunk) => { serverOutput += chunk.toString(); });

const requests = [];
const nativeFetch = globalThis.fetch.bind(globalThis);
const trackedFetch = async (input, init = {}) => {
  const headers = new Headers(init.headers);
  requests.push({
    method: init.method ?? "GET",
    authorization: headers.get("authorization"),
    accept: headers.get("accept"),
    protocolVersion: headers.get("mcp-protocol-version"),
    body: typeof init.body === "string" ? init.body : "",
  });
  return nativeFetch(input, init);
};

let client;
try {
  await waitForServer(endpoint, () => serverOutput);
  const deniedOrigin = await nativeFetch(endpoint, {
    method: "OPTIONS",
    headers: { Origin: "https://denied.example.test" },
  });
  assert(deniedOrigin.status === 403, "MCP HTTP remote listener accepted an Origin outside its allowlist");
  const allowedOrigin = await nativeFetch(endpoint, {
    method: "OPTIONS",
    headers: { Origin: `http://127.0.0.1:${port}` },
  });
  assert(allowedOrigin.status === 204, "MCP HTTP remote listener rejected its configured Origin");
  const unauthenticated = await nativeFetch(endpoint, {
    method: "POST",
    headers: { "Content-Type": "application/json", Accept: "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping", params: {} }),
  });
  assert(unauthenticated.status === 401, "MCP HTTP remote listener accepted a request without its token");
  globalThis.fetch = trackedFetch;
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
  if ("protocolVersion" in transport) {
    assert(transport.protocolVersion === protocolVersion, "official SDK negotiated the wrong MCP version");
  }
  assert(transport.sessionId === undefined, "PortMate's documented stateless HTTP mode unexpectedly created a session");

  await client.ping();
  const tools = await client.listTools();
  const toolNames = new Set(tools.tools.map((tool) => tool.name));
  for (const toolName of [
    "list_sessions",
    "mcp_bridge_status",
    "reload_mcp",
    "restart_mcp",
    "list_transfers",
    "get_transfer",
    "start_transfer",
    "start_content_transfer",
    "begin_content_upload",
    "append_content_upload",
    "start_content_upload_transfer",
    "cancel_content_upload",
    "cancel_transfer",
    "retry_transfer",
    "create_tunnel",
    "list_tunnels",
    "stop_tunnel",
  ]) {
    assert(toolNames.has(toolName), `tools/list omitted ${toolName}`);
  }
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
  if (requestedProtocolVersion) {
    assert(
      requestParams(initialize)?.protocolVersion === requestedProtocolVersion,
      "initialize used the wrong MCP version",
    );
  }
  assert(initialize.accept === "application/json, text/event-stream", "initialize did not advertise both Streamable HTTP response types");
  assert(eventStream.accept === "text/event-stream", "GET event stream used the wrong Accept header");

  for (const request of requests) {
    assert(request.authorization === `Bearer ${token}`, `${request.method} request omitted HTTP authentication`);
    if (request !== initialize) {
      if (expectsProtocolHeader) {
        assert(request.protocolVersion === protocolVersion, `${request.method} request omitted the negotiated MCP version`);
      } else {
        assert(request.protocolVersion === null, `${request.method} unexpectedly sent a newer MCP protocol header`);
      }
    }
  }

  console.log(`MCP TypeScript SDK ${sdkVersion} HTTP check passed (${requests.length} requests)`);
} finally {
  globalThis.fetch = nativeFetch;
  await client?.close().catch(() => {});
  await stopServer(server);
}
