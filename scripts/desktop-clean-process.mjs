function endpointPort(endpoint) {
  const match = endpoint.match(/:(\d+)$/);
  return match ? Number(match[1]) : 0;
}

export function parseWindowsListeningPids(output, port) {
  const pids = [];
  for (const line of output.split(/\r?\n/)) {
    const fields = line.trim().split(/\s+/);
    if (fields.length < 5 || fields[0].toUpperCase() !== "TCP") continue;
    if (fields.at(-2)?.toUpperCase() !== "LISTENING") continue;
    if (endpointPort(fields[1]) !== port) continue;
    const pid = Number(fields.at(-1));
    if (Number.isInteger(pid) && pid > 1) pids.push(pid);
  }
  return [...new Set(pids)];
}

export function isProjectViteCommand(command, projectRoot, platform = process.platform) {
  if (!command?.trim() || !projectRoot?.trim()) return false;
  const normalize = platform === "win32"
    ? (value) => value.replaceAll("\\", "/").toLowerCase()
    : (value) => value;
  const normalizedCommand = normalize(command);
  const root = normalize(projectRoot).replace(/\/+$/, "");
  return [
    `${root}/node_modules/.bin/vite`,
    `${root}/node_modules/vite/bin/vite.js`,
  ].some((marker) => normalizedCommand.includes(marker));
}

export function signalProcessIfRunning(pid, signal, kill = process.kill) {
  try {
    kill(pid, signal);
    return true;
  } catch (error) {
    if (error?.code === "ESRCH") return false;
    throw error;
  }
}
