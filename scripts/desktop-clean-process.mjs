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

export async function releaseProjectDevPort({
  host,
  port,
  initialWaitMs,
  terminateWaitMs,
  forceWaitMs,
  waitForPort,
  listeningPids,
  isProjectVite,
  signalProcess = signalProcessIfRunning,
  onStopped = () => {},
}) {
  if (await waitForPort(host, port, initialWaitMs)) return;
  const listeners = listeningPids(port);
  if (!listeners.length) {
    if (await waitForPort(host, port, initialWaitMs)) return;
    throw new Error(`Port ${port} is busy, but its listener could not be identified.`);
  }

  const foreign = listeners.filter((pid) => !isProjectVite(pid));
  if (foreign.length) {
    throw new Error(`Port ${port} is owned by another process (PID ${foreign.join(", ")}); it was not terminated.`);
  }

  for (const pid of listeners) signalProcess(pid, "SIGTERM");
  if (await waitForPort(host, port, terminateWaitMs)) {
    onStopped(`Stopped stale PortMate Vite listener on ${host}:${port}.`);
    return;
  }

  for (const pid of listeners) signalProcess(pid, "SIGKILL");
  if (!await waitForPort(host, port, forceWaitMs)) {
    throw new Error(`Port ${port} remained busy after stopping stale PortMate Vite (PID ${listeners.join(", ")}).`);
  }
  onStopped(`Force-stopped stale PortMate Vite listener on ${host}:${port}.`);
}

export async function waitForStablePortAvailability(checkAvailable, options) {
  const {
    timeoutMs,
    stableMs,
    intervalMs = 100,
    now = Date.now,
    sleep = (durationMs) => new Promise((resolve) => setTimeout(resolve, durationMs)),
  } = options;
  const deadline = now() + timeoutMs;
  let stableSince = null;
  while (true) {
    if (await checkAvailable()) {
      const checkedAt = now();
      stableSince ??= checkedAt;
      if (checkedAt - stableSince >= stableMs) return true;
    } else {
      stableSince = null;
    }

    const remaining = deadline - now();
    if (remaining <= 0) return false;
    await sleep(Math.min(intervalMs, remaining));
  }
}
