import { existsSync } from "node:fs";
import { dirname, join, win32 } from "node:path";

export function npmInvocation(args, options = {}) {
  if (!Array.isArray(args) || args.some((arg) => typeof arg !== "string" || arg.includes("\0"))) {
    throw new Error("npm arguments must be strings without NUL");
  }
  const environment = options.environment ?? process.env;
  const execPath = options.execPath ?? process.execPath;
  const platform = options.platform ?? process.platform;
  const pathExists = options.pathExists ?? existsSync;
  const configured = environment.npm_execpath?.trim();
  const adjacentNpmCli = platform === "win32"
    ? win32.join(win32.dirname(execPath), "node_modules", "npm", "bin", "npm-cli.js")
    : join(dirname(execPath), "node_modules", "npm", "bin", "npm-cli.js");
  const candidates = [
    configured,
    adjacentNpmCli,
  ].filter((candidate, index, values) => (
    candidate
    && !candidate.includes("\0")
    && values.indexOf(candidate) === index
  ));

  for (const npmCli of candidates) {
    if (pathExists(npmCli)) {
      return { command: execPath, args: [npmCli, ...args] };
    }
  }
  if (platform === "win32") {
    throw new Error(
      "Unable to locate npm-cli.js on Windows; run this command through npm or repair the Node.js installation",
    );
  }
  return { command: "npm", args };
}
