import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { EnvHttpProxyAgent, fetch as undiciFetch } from "undici";

const scriptRoot = dirname(fileURLToPath(import.meta.url));
const defaultProjectRoot = resolve(scriptRoot, "..");
const stableVersionPattern = /^(?:v)?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const defaultMaxRegistryResponseBytes = 2 * 1024 * 1024;

export const MCP_SDK_SOURCES = Object.freeze([
  {
    key: "typescript",
    label: "TypeScript",
    matrixFile: "mcp-typescript-client-versions.json",
    registryUrl: "https://registry.npmjs.org/@modelcontextprotocol%2Fsdk/latest",
    responseFormat: "npm",
  },
  {
    key: "python",
    label: "Python",
    matrixFile: "mcp-python-client-versions.json",
    registryUrl: "https://pypi.org/pypi/mcp/json",
    responseFormat: "pypi",
  },
  {
    key: "go",
    label: "Go",
    matrixFile: "mcp-go-client-versions.json",
    registryUrl: "https://proxy.golang.org/github.com/modelcontextprotocol/go-sdk/@v/list",
    responseFormat: "go-proxy",
  },
  {
    key: "rust",
    label: "Rust",
    matrixFile: "mcp-rust-client-versions.json",
    registryUrl: "https://crates.io/api/v1/crates/rmcp",
    responseFormat: "crates-io",
  },
  {
    key: "ruby",
    label: "Ruby",
    matrixFile: "mcp-ruby-client-versions.json",
    registryUrl: "https://rubygems.org/api/v1/gems/mcp.json",
    responseFormat: "rubygems",
  },
  {
    key: "java",
    label: "Java",
    matrixFile: "mcp-java-client-versions.json",
    registryUrl: "https://repo.maven.apache.org/maven2/io/modelcontextprotocol/sdk/mcp/maven-metadata.xml",
    responseFormat: "maven",
  },
  {
    key: "kotlin",
    label: "Kotlin",
    matrixFile: "mcp-kotlin-client-versions.json",
    registryUrl: "https://repo.maven.apache.org/maven2/io/modelcontextprotocol/kotlin-sdk-client-jvm/maven-metadata.xml",
    responseFormat: "maven",
  },
  {
    key: "csharp",
    label: "C#",
    matrixFile: "mcp-csharp-client-versions.json",
    registryUrl: "https://api.nuget.org/v3-flatcontainer/modelcontextprotocol.core/index.json",
    responseFormat: "nuget",
  },
  {
    key: "swift",
    label: "Swift",
    matrixFile: "mcp-swift-client-versions.json",
    registryUrl: "https://api.github.com/repos/modelcontextprotocol/swift-sdk/releases/latest",
    responseFormat: "github-release",
  },
]);

export function normalizeStableVersion(value) {
  if (typeof value !== "string") return null;
  const match = stableVersionPattern.exec(value.trim());
  if (!match) return null;
  const parts = match.slice(1).map(Number);
  if (!parts.every(Number.isSafeInteger)) return null;
  return parts.join(".");
}

export function compareStableVersions(left, right) {
  const leftParts = normalizedVersionParts(left);
  const rightParts = normalizedVersionParts(right);
  for (let index = 0; index < leftParts.length; index += 1) {
    if (leftParts[index] !== rightParts[index]) return leftParts[index] - rightParts[index];
  }
  return 0;
}

export function newestStableVersion(values) {
  const versions = values
    .map(normalizeStableVersion)
    .filter((version) => version !== null);
  if (!versions.length) throw new Error("Registry response did not contain a stable semantic version");
  return versions.reduce((latest, version) => (
    compareStableVersions(version, latest) > 0 ? version : latest
  ));
}

export function parseLatestSdkVersion(responseFormat, body) {
  switch (responseFormat) {
    case "npm":
      return requireStableVersion(JSON.parse(body).version, "npm latest version");
    case "pypi":
      return requireStableVersion(JSON.parse(body).info?.version, "PyPI latest version");
    case "go-proxy":
      return newestStableVersion(body.split(/\s+/));
    case "crates-io": {
      const crate = JSON.parse(body).crate;
      return requireStableVersion(
        crate?.max_stable_version ?? crate?.max_version,
        "crates.io latest stable version",
      );
    }
    case "rubygems":
      return requireStableVersion(JSON.parse(body).version, "RubyGems latest version");
    case "maven": {
      const release = /<release>\s*([^<]+?)\s*<\/release>/.exec(body)?.[1];
      return requireStableVersion(release, "Maven release version");
    }
    case "nuget":
      return newestStableVersion(JSON.parse(body).versions ?? []);
    case "github-release":
      return requireStableVersion(JSON.parse(body).tag_name, "GitHub release tag");
    default:
      throw new Error(`Unsupported MCP SDK registry response format: ${responseFormat}`);
  }
}

export function loadMatrixVersions(projectRoot, source) {
  const matrixPath = join(projectRoot, "scripts", source.matrixFile);
  const matrix = JSON.parse(readFileSync(matrixPath, "utf8"));
  const entries = Array.isArray(matrix) ? matrix : matrix?.sdks;
  if (!Array.isArray(entries) || !entries.length) {
    throw new Error(`${source.matrixFile} does not contain an SDK version matrix`);
  }
  return entries.map((entry) => requireStableVersion(
    entry?.version,
    `${source.matrixFile} SDK version`,
  ));
}

export function assertLatestCovered(source, matrixVersions, latestVersion) {
  if (matrixVersions.includes(latestVersion)) return;
  throw new Error(
    `${source.label}: registry latest ${latestVersion} is absent from scripts/${source.matrixFile}; newest covered version is ${newestStableVersion(matrixVersions)}`,
  );
}

export async function auditMcpSdkFreshness(options = {}) {
  const projectRoot = options.projectRoot ?? defaultProjectRoot;
  const environment = options.environment ?? process.env;
  const dispatcher = options.fetchImpl ? null : new EnvHttpProxyAgent(
    proxyOptionsFromEnvironment(environment),
  );
  const fetchImpl = options.fetchImpl ?? ((url, init) => undiciFetch(url, {
    ...init,
    dispatcher,
  }));
  try {
    const results = await Promise.all(MCP_SDK_SOURCES.map(async (source) => {
      const matrixVersions = loadMatrixVersions(projectRoot, source);
      const body = await fetchTextWithRetries(source.registryUrl, {
        fetchImpl,
        headers: registryHeaders(source, environment),
        attempts: options.attempts,
        timeoutMs: options.timeoutMs,
        maxBytes: options.maxBytes,
      });
      const latestVersion = parseLatestSdkVersion(source.responseFormat, body);
      assertLatestCovered(source, matrixVersions, latestVersion);
      return {
        ...source,
        latestVersion,
        matrixVersions,
        newestCoveredVersion: newestStableVersion(matrixVersions),
      };
    }));

    const packageJson = JSON.parse(readFileSync(join(projectRoot, "package.json"), "utf8"));
    const typescriptLatest = results.find((result) => result.key === "typescript")?.latestVersion;
    const rootTypescriptVersion = packageJson.devDependencies?.["@modelcontextprotocol/sdk"];
    if (rootTypescriptVersion !== typescriptLatest) {
      throw new Error(
        `TypeScript: package.json must pin @modelcontextprotocol/sdk ${typescriptLatest}; found ${rootTypescriptVersion ?? "no version"}`,
      );
    }

    for (const result of results) {
      console.log(
        `MCP ${result.label} SDK latest ${result.latestVersion} is covered (${result.matrixVersions.length} pinned versions)`,
      );
    }
    console.log(`MCP SDK freshness audit passed (${results.length} official SDKs)`);
    return results;
  } finally {
    await dispatcher?.close();
  }
}

export async function fetchTextWithRetries(url, options = {}) {
  const fetchImpl = options.fetchImpl ?? fetch;
  const attempts = options.attempts ?? 3;
  const timeoutMs = options.timeoutMs ?? 20_000;
  const maxBytes = options.maxBytes ?? defaultMaxRegistryResponseBytes;
  if (!Number.isInteger(attempts) || attempts < 1 || attempts > 5) {
    throw new Error(`Registry fetch attempts must be between 1 and 5; received ${attempts}`);
  }
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1_000 || timeoutMs > 60_000) {
    throw new Error(`Registry fetch timeout must be between 1000 and 60000 ms; received ${timeoutMs}`);
  }
  if (!Number.isInteger(maxBytes) || maxBytes < 1_024 || maxBytes > 4 * 1024 * 1024) {
    throw new Error(`Registry response limit must be between 1024 and 4194304 bytes; received ${maxBytes}`);
  }

  let lastError;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      const response = await fetchImpl(url, {
        headers: options.headers,
        signal: AbortSignal.timeout(timeoutMs),
      });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      return await readBoundedResponseText(response, maxBytes);
    } catch (error) {
      lastError = error;
      if (attempt < attempts) {
        await new Promise((resolveWait) => setTimeout(resolveWait, attempt * 500));
      }
    }
  }
  throw new Error(
    `Unable to query MCP SDK registry ${url} after ${attempts} attempts: ${lastError?.message ?? "unknown error"}`,
    { cause: lastError },
  );
}

export async function readBoundedResponseText(response, maxBytes) {
  const declaredLength = response.headers.get("content-length");
  if (/^\d+$/.test(declaredLength ?? "") && Number(declaredLength) > maxBytes) {
    await response.body?.cancel();
    throw new Error(`Registry response declares ${declaredLength} bytes, exceeding the ${maxBytes}-byte limit`);
  }
  if (!response.body) return "";

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  const chunks = [];
  let totalBytes = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      totalBytes += value.byteLength;
      if (totalBytes > maxBytes) {
        await reader.cancel();
        throw new Error(`Registry response exceeds the ${maxBytes}-byte limit`);
      }
      chunks.push(decoder.decode(value, { stream: true }));
    }
    chunks.push(decoder.decode());
    return chunks.join("");
  } finally {
    reader.releaseLock();
  }
}

export function proxyOptionsFromEnvironment(environment) {
  return {
    httpProxy: environment.http_proxy ?? environment.HTTP_PROXY,
    httpsProxy: environment.https_proxy ?? environment.HTTPS_PROXY,
    noProxy: environment.no_proxy ?? environment.NO_PROXY,
  };
}

function requireStableVersion(value, label) {
  const version = normalizeStableVersion(value);
  if (!version) throw new Error(`${label} is not a stable semantic version: ${String(value)}`);
  return version;
}

function normalizedVersionParts(value) {
  const version = requireStableVersion(value, "Version");
  return version.split(".").map(Number);
}

function registryHeaders(source, environment) {
  const headers = {
    Accept: source.responseFormat === "maven" ? "application/xml" : "application/json, text/plain;q=0.9",
    "User-Agent": "PortMate-MCP-SDK-Freshness/1.0",
  };
  if (source.responseFormat === "github-release" && environment.GITHUB_TOKEN?.trim()) {
    headers.Authorization = `Bearer ${environment.GITHUB_TOKEN.trim()}`;
    headers["X-GitHub-Api-Version"] = "2022-11-28";
  }
  return headers;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  auditMcpSdkFreshness().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  });
}
