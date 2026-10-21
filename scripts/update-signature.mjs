import { createHash, createPublicKey, verify } from "node:crypto";

// Tauri encodes the complete minisign public key / signature text in Base64.
// Verify both the payload and the trusted comment with Node's Ed25519 API.
export function verifyUpdateSignature(bytes, encodedSignature, encodedPublicKey, version) {
  const publicLines = Buffer.from(encodedPublicKey.trim(), "base64").toString("utf8").trimEnd().split(/\r?\n/);
  const lines = Buffer.from(encodedSignature.trim(), "base64").toString("utf8").trimEnd().split(/\r?\n/);
  if (publicLines.length !== 2 || lines.length !== 4 || !lines[2].startsWith("trusted comment: ")) {
    throw new Error("Invalid updater signature encoding");
  }
  const key = Buffer.from(publicLines[1], "base64");
  const signature = Buffer.from(lines[1], "base64");
  const global = Buffer.from(lines[3], "base64");
  if (key.length !== 42 || key.subarray(0, 2).toString() !== "Ed"
      || signature.length !== 74 || signature.subarray(0, 2).toString() !== "ED"
      || global.length !== 64 || !key.subarray(2, 10).equals(signature.subarray(2, 10))) {
    throw new Error("Updater signature uses an unexpected key or algorithm");
  }
  const publicKey = createPublicKey({
    key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), key.subarray(10)]),
    format: "der", type: "spki",
  });
  const trusted = lines[2].slice("trusted comment: ".length);
  const signedVersion = trusted.split("\t").filter(field => field.startsWith("version:"));
  if (signedVersion.length !== 1 || signedVersion[0] !== `version:${version}`) {
    throw new Error("Updater signature is not bound to the release version");
  }
  if (!verify(null, createHash("blake2b512").update(bytes).digest(), publicKey, signature.subarray(10))
      || !verify(null, Buffer.concat([signature.subarray(10), Buffer.from(trusted)]), publicKey, global)) {
    throw new Error("Updater signature verification failed");
  }
}
