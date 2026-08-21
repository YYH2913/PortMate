import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const sdkRoot = process.env.PORTMATE_MCP_TYPESCRIPT_SDK_ROOT?.trim();
const sdkModule = (relativePath, packagePath) => sdkRoot
  ? pathToFileURL(path.join(sdkRoot, "dist", "esm", ...relativePath)).href
  : packagePath;
const { Client } = await import(sdkModule(["client", "index.js"], "@modelcontextprotocol/sdk/client/index.js"));
const { StdioClientTransport } = await import(sdkModule(["client", "stdio.js"], "@modelcontextprotocol/sdk/client/stdio.js"));
const protocolVersion = process.env.PORTMATE_MCP_EXPECTED_PROTOCOL_VERSION?.trim() || "2025-06-18";
const requestedProtocolVersion = process.env.PORTMATE_MCP_EXPECTED_REQUEST_PROTOCOL_VERSION?.trim();
const sdkVersion = process.env.PORTMATE_MCP_TYPESCRIPT_SDK_VERSION?.trim() || "root";

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
    PORTMATE_STORE_PATH: "",
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
let bridgeProcess;

try {
  await client.connect(transport);
  bridgeProcess = transport._process;
  assert(bridgeProcess?.pid, "official SDK did not start the PortMate stdio bridge");
  assert(client.getServerVersion()?.name === "portmate-mcp", "official SDK did not initialize PortMate over stdio");

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
    "send_bytes",
    "begin_content_upload",
    "append_content_upload",
    "cancel_content_upload",
    "cancel_transfer",
    "retry_transfer",
    "create_tunnel",
    "list_tunnels",
    "stop_tunnel",
  ]) {
    assert(toolNames.has(toolName), `tools/list omitted ${toolName}`);
  }
  const startTransfer = tools.tools.find((tool) => tool.name === "start_transfer");
  assert(startTransfer?.inputSchema?.properties?.protocol?.enum?.includes("tftp"),
    "start_transfer schema omitted TFTP");
  assert(startTransfer?.inputSchema?.oneOf?.length === 3,
    "start_transfer schema did not unify path, inline, and uploaded sources");
  const sourceSchemas = startTransfer?.inputSchema?.properties?.source?.oneOf ?? [];
  assert(sourceSchemas.some((schema) => schema.type === "string"),
    "start_transfer schema omitted the desktop path source");
  assert(sourceSchemas.some((schema) => schema.type === "object"
    && schema.properties?.kind?.const === "mcp"
    && schema.required?.includes("fileName")
    && schema.required?.includes("contentBase64")
    && schema.additionalProperties === false),
    "start_transfer schema omitted the virtual MCP file source");
  const destinationSchemas = startTransfer?.inputSchema?.properties?.destination?.oneOf ?? [];
  assert(destinationSchemas.some((schema) => schema.type === "object"
    && schema.properties?.kind?.const === "tftpboot"
    && schema.required?.includes("deviceIp")
    && schema.additionalProperties === false),
    "start_transfer schema omitted the structured TFTP destination");
  const beginContentUpload = tools.tools.find((tool) => tool.name === "begin_content_upload");
  assert(beginContentUpload?.inputSchema?.properties?.destination?.oneOf?.some((schema) =>
    schema.type === "object" && schema.required?.includes("deviceIp")),
  "begin_content_upload schema omitted the structured TFTP destination");
  const sendBytes = tools.tools.find((tool) => tool.name === "send_bytes");
  assert(sendBytes?.inputSchema?.properties?.encoding?.enum?.includes("base64")
    && sendBytes?.inputSchema?.properties?.encoding?.enum?.includes("hex"),
    "send_bytes schema omitted a binary encoding");
  const createTunnel = tools.tools.find((tool) => tool.name === "create_tunnel");
  assert(createTunnel?.inputSchema?.properties?.egress?.enum?.includes("portmate-host"),
    "create_tunnel schema omitted PortMate-host egress");
  assert(!createTunnel?.inputSchema?.required?.includes("sessionId"),
    "create_tunnel still requires an SSH/Tmux session for every route");
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
  if (requestedProtocolVersion) {
    assert(
      initialize.params.protocolVersion === requestedProtocolVersion,
      "official SDK requested an unexpected stdio MCP version",
    );
  }
  assert(
    (negotiatedProtocolVersion ?? initialize.params.protocolVersion) === protocolVersion,
    "PortMate negotiated an unexpected stdio MCP version",
  );

  console.log(`MCP TypeScript SDK ${sdkVersion} stdio check passed (${sent.length} messages)`);
} catch (error) {
  if (stderr) {
    error.message = `${error.message}\nPortMate stderr:\n${stderr}`;
  }
  throw error;
} finally {
  await client.close().catch(() => {});
}

if (bridgeProcess?.exitCode === null && bridgeProcess.signalCode === null) {
  await Promise.race([
    new Promise((resolveExit) => bridgeProcess.once("exit", resolveExit)),
    new Promise((resolveTimeout) => setTimeout(resolveTimeout, 2_000)),
  ]);
}
assert(
  bridgeProcess && (bridgeProcess.exitCode !== null || bridgeProcess.signalCode !== null),
  "PortMate stdio bridge process survived client shutdown",
);
