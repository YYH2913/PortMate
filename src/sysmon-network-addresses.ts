const DISPLAYED_SYSMON_ADDRESS_COUNT = 2;

export function orderedSysmonNetworkAddresses(addresses: string[] | undefined) {
  return (addresses ?? [])
    .map((address, index) => ({ address: address.trim(), index }))
    .filter(({ address }) => address.length > 0)
    .sort((left, right) => {
      const priority = sysmonNetworkAddressPriority(left.address) - sysmonNetworkAddressPriority(right.address);
      return priority || left.index - right.index;
    })
    .map(({ address }) => address);
}

export function formatSysmonNetworkAddresses(addresses: string[] | undefined) {
  const ordered = orderedSysmonNetworkAddresses(addresses);
  if (!ordered.length) return "-";
  if (ordered.length <= DISPLAYED_SYSMON_ADDRESS_COUNT) return ordered.join(" · ");
  return `${ordered.slice(0, DISPLAYED_SYSMON_ADDRESS_COUNT).join(" · ")} +${ordered.length - DISPLAYED_SYSMON_ADDRESS_COUNT}`;
}

function sysmonNetworkAddressPriority(address: string) {
  const host = address.split("/", 1)[0].toLowerCase();
  const ipv4 = parseIpv4Address(host);
  if (ipv4) {
    const [first, second] = ipv4;
    if (first === 127 || first === 0) return 4;
    if (first === 169 && second === 254) return 2;
    return 0;
  }
  if (host.includes(":")) {
    if (host === "::" || host === "::1") return 5;
    const firstSegment = Number.parseInt(host.split(":", 1)[0], 16);
    if (Number.isInteger(firstSegment) && firstSegment >= 0xfe80 && firstSegment <= 0xfebf) return 3;
    return 1;
  }
  return 6;
}

function parseIpv4Address(value: string): [number, number, number, number] | null {
  const parts = value.split(".");
  if (parts.length !== 4) return null;
  const octets = parts.map((part) => Number.parseInt(part, 10));
  if (octets.some((octet, index) => !/^\d{1,3}$/.test(parts[index]) || octet < 0 || octet > 255)) return null;
  return octets as [number, number, number, number];
}
