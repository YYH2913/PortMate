export const MAX_TUNNELS_PER_PROFILE = 64;
export const MAX_TUNNEL_ID_CHARACTERS = 128;
export const MAX_TUNNEL_LABEL_CHARACTERS = 128;
export const MAX_TUNNEL_HOST_CHARACTERS = 255;

export function canAddTunnel(count: number): boolean {
  return Number.isInteger(count) && count >= 0 && count < MAX_TUNNELS_PER_PROFILE;
}

export function parseTunnelPort(value: string, allowZero: boolean): number | null {
  const normalized = value.trim();
  if (!/^\d{1,5}$/.test(normalized)) return null;
  const port = Number(normalized);
  if (!Number.isInteger(port) || port > 65_535 || (!allowZero && port === 0)) return null;
  return port;
}

export function isValidTunnelHostInput(value: string, allowEmpty = false): boolean {
  const normalized = value.trim();
  if (!allowEmpty && !normalized) return false;
  let count = 0;
  for (const character of normalized) {
    count += 1;
    if (count > MAX_TUNNEL_HOST_CHARACTERS || /\s/u.test(character)) return false;
    const codePoint = character.codePointAt(0) ?? 0;
    if (codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f)) return false;
  }
  return true;
}
