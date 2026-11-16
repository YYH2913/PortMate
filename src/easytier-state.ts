export type EasyTierConfig = {
  networkName: string;
  hostname: string;
  addressMode: "dhcp" | "static" | "relay";
  virtualIpv4: string;
  peers: string[];
  listeners: string[];
  latencyFirst: boolean;
  mtu: number;
};

export type EasyTierError = { code: string; detail: string };
export type EasyTierPeer = { id: string; hostname: string; ipv4: string; route: string; latency: string; received: string; sent: string; protocol: string };
export type NpcapStatus = {
  required: boolean;
  installed: boolean;
  phase: "not-required" | "detected" | "missing" | "downloading" | "downloaded" | "installing" | "completed" | "reboot-required" | "failed";
  downloaded: number;
  total: number | null;
  installerVersion: string;
  downloadUrl: string;
  licenseUrl: string;
  error: EasyTierError | null;
};
export type EasyTierView = {
  config: EasyTierConfig;
  componentReady: boolean;
  componentVersion: string;
  npcap?: NpcapStatus;
  status: {
    phase: "idle" | "starting" | "running" | "stopping" | "failed";
    pid: number | null;
    virtualIpv4: string;
    hostname: string;
    peers: EasyTierPeer[];
    logs: string[];
    error: EasyTierError | null;
  };
};

export const defaultEasyTierConfig = (): EasyTierConfig => ({
  networkName: "PortMate", hostname: "", addressMode: "dhcp", virtualIpv4: "",
  peers: [], listeners: ["tcp://0.0.0.0:11010", "udp://0.0.0.0:11010"], latencyFirst: false, mtu: 1360,
});

export function easyTierEndpoints(value: string): string[] {
  return value.split(/\r?\n/).map(line => line.trim()).filter(Boolean);
}

export function easyTierConfigValid(config: EasyTierConfig): boolean {
  const safe = (value: string, max: number) => [...value].length <= max && !/[\x00-\x1f\x7f-\x9f]/.test(value) && !value.includes("${");
  if (!config.networkName.trim() || !safe(config.networkName, 128) || !safe(config.hostname, 128)) return false;
  if (!Number.isInteger(config.mtu) || config.mtu < 576 || config.mtu > 9000) return false;
  if (config.addressMode === "static") {
    const parts = config.virtualIpv4.split(".");
    if (parts.length !== 4 || parts.some(part => !/^(0|[1-9]\d{0,2})$/.test(part) || Number(part) > 255)) return false;
    if (Number(parts[0]) === 127 || Number(parts[0]) >= 224 || parts.every(part => part === "0")) return false;
  }
  for (const [endpoints, listener] of [[config.peers, false], [config.listeners, true]] as const) {
    if (endpoints.length > 32 || new Set(endpoints).size !== endpoints.length) return false;
    for (const endpoint of endpoints) {
      try {
        const url = new URL(endpoint);
        const port = url.port || (url.protocol === "ws:" ? "80" : url.protocol === "wss:" ? "443" : "");
        if (!safe(endpoint, 512) || !["tcp:", "udp:", "ws:", "wss:", "quic:", "wg:"].includes(url.protocol)
          || !url.hostname || !port || Number(port) < 1 || Number(port) > 65535
          || url.username || url.password || url.search || url.hash
          || !listener && ["0.0.0.0", "[::]"].includes(url.hostname)) return false;
      } catch { return false; }
    }
  }
  return true;
}

export function easyTierSecretValid(secret: string): boolean {
  return Boolean(secret.trim()) && [...secret].length <= 1024 && !/[\x00-\x1f\x7f-\x9f]/.test(secret) && !secret.includes("${");
}
