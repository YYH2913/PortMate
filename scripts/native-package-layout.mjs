import { createHash } from "node:crypto";
import {
  lstatSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  statSync,
} from "node:fs";
import {
  basename,
  dirname,
  isAbsolute,
  join,
  relative,
  resolve,
  sep,
} from "node:path";

export function verifyWindowsPackageLayout({
  root,
  sourceMain,
  sourceSidecar,
  sourceLicense,
  sourceThirdPartyLicense,
}) {
  const files = inspectPortableTree(root);
  const main = findUniqueRegularFile(files, "portmate.exe", { caseInsensitive: true });
  const sidecar = findUniqueRegularFile(files, "portmate-mcp.exe", { caseInsensitive: true });
  const license = findUniqueRegularFile(files, "LICENSE", { caseInsensitive: true });
  const thirdPartyLicense = findUniqueRegularFile(files, "JetBrainsMono-OFL.txt", { caseInsensitive: true });

  assertSameDirectory("Windows application payload", [main, sidecar, license], true);
  assertExactPath(
    "Windows JetBrains Mono license",
    thirdPartyLicense,
    join(dirname(main), "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt"),
    true,
  );
  assertSameFile(sourceMain, main, "Windows main executable");
  assertSameFile(sourceSidecar, sidecar, "Windows MCP sidecar");
  assertSameFile(sourceLicense, license, "Windows license");
  assertSameFile(sourceThirdPartyLicense, thirdPartyLicense, "Windows JetBrains Mono license");

  return {
    root: resolve(root),
    applicationDirectory: dirname(main),
    main,
    sidecar,
    license,
    thirdPartyLicense,
    sha256: {
      main: sha256File(main),
      sidecar: sha256File(sidecar),
      license: sha256File(license),
      thirdPartyLicense: sha256File(thirdPartyLicense),
    },
  };
}

export function verifyMacAppBundle({
  app,
  sourceMain,
  sourceSidecar,
  sourceLicense,
  sourceThirdPartyLicense,
  metadata,
  expectedMetadata,
  compareBinaries = true,
}) {
  const appRoot = resolve(app);
  if (basename(appRoot) !== "PortMate.app") {
    throw new Error(`Expected the macOS bundle to be named PortMate.app: ${appRoot}`);
  }

  const files = inspectPortableTree(appRoot);
  const main = findUniqueRegularFile(files, "portmate");
  const sidecar = findUniqueRegularFile(files, "portmate-mcp");
  const license = findUniqueRegularFile(files, "LICENSE");
  const thirdPartyLicense = findUniqueRegularFile(files, "JetBrainsMono-OFL.txt");
  const infoPlist = join(appRoot, "Contents", "Info.plist");

  assertExactPath("macOS main executable", main, join(appRoot, "Contents", "MacOS", "portmate"));
  assertExactPath(
    "macOS MCP sidecar",
    sidecar,
    join(appRoot, "Contents", "MacOS", "portmate-mcp"),
  );
  assertExactPath(
    "macOS license",
    license,
    join(appRoot, "Contents", "Resources", "LICENSE"),
  );
  assertExactPath(
    "macOS JetBrains Mono license",
    thirdPartyLicense,
    join(appRoot, "Contents", "Resources", "THIRD_PARTY_LICENSES", "JetBrainsMono-OFL.txt"),
  );
  assertExactPath("macOS Info.plist", infoPlist, join(appRoot, "Contents", "Info.plist"));
  assertNonEmptyRegularFile(infoPlist, "macOS Info.plist");
  if (compareBinaries) {
    assertSameFile(sourceMain, main, "macOS main executable");
    assertSameFile(sourceSidecar, sidecar, "macOS MCP sidecar");
  } else {
    assertNonEmptyRegularFile(sourceMain, "macOS main executable source");
    assertNonEmptyRegularFile(sourceSidecar, "macOS MCP sidecar source");
    assertNonEmptyRegularFile(main, "macOS main executable");
    assertNonEmptyRegularFile(sidecar, "macOS MCP sidecar");
  }
  assertSameFile(sourceLicense, license, "macOS license");
  assertSameFile(sourceThirdPartyLicense, thirdPartyLicense, "macOS JetBrains Mono license");
  verifyMacBundleMetadata(metadata, expectedMetadata);

  return {
    app: appRoot,
    main,
    sidecar,
    license,
    thirdPartyLicense,
    infoPlist,
    compareBinaries,
    metadata: { ...metadata },
    sha256: {
      main: sha256File(main),
      sidecar: sha256File(sidecar),
      license: sha256File(license),
      thirdPartyLicense: sha256File(thirdPartyLicense),
    },
  };
}

export function verifyMacBundleMetadata(actual, expected) {
  for (const key of [
    "CFBundleIdentifier",
    "CFBundleShortVersionString",
    "CFBundleVersion",
    "CFBundleExecutable",
    "LSApplicationCategoryType",
  ]) {
    if (typeof expected?.[key] !== "string" || expected[key].length === 0) {
      throw new Error(`Expected macOS bundle metadata is missing ${key}`);
    }
    if (actual?.[key] !== expected[key]) {
      throw new Error(
        `Expected macOS ${key}=${expected[key]}, found ${String(actual?.[key])}`,
      );
    }
  }
}

export function inspectPortableTree(root) {
  const treeRoot = resolve(root);
  const rootMetadata = lstatSync(treeRoot);
  if (!rootMetadata.isDirectory()) {
    throw new Error(`Package root must be a directory: ${treeRoot}`);
  }

  const files = [];
  for (const entry of readdirSync(treeRoot)) visit(join(treeRoot, entry));
  return files;

  function visit(path) {
    const metadata = lstatSync(path);
    if (metadata.isSymbolicLink()) {
      const target = readlinkSync(path);
      if (isAbsolute(target)) {
        throw new Error(`Package contains an absolute symlink: ${path} -> ${target}`);
      }
      const resolvedTarget = resolve(dirname(path), target);
      assertPathWithinRoot(treeRoot, resolvedTarget, `Package symlink escapes its root: ${path} -> ${target}`);
      statSync(path);
      return;
    }
    if (metadata.isDirectory()) {
      for (const entry of readdirSync(path)) visit(join(path, entry));
      return;
    }
    if (metadata.isFile()) {
      files.push(resolve(path));
      return;
    }
    throw new Error(`Package contains an unsupported filesystem entry: ${path}`);
  }
}

export function findUniqueRegularFile(files, name, options = {}) {
  const normalize = options.caseInsensitive
    ? (value) => value.toLocaleLowerCase("en-US")
    : (value) => value;
  const expected = normalize(name);
  const matches = files.filter((path) => normalize(basename(path)) === expected);
  if (matches.length !== 1) {
    throw new Error(`Expected exactly one ${name} regular file, found ${matches.length}`);
  }
  return matches[0];
}

export function sha256File(path) {
  assertNonEmptyRegularFile(path, "SHA-256 input");
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function assertSameFile(expected, actual, label) {
  assertNonEmptyRegularFile(expected, `${label} source`);
  assertNonEmptyRegularFile(actual, label);
  const expectedHash = sha256File(expected);
  const actualHash = sha256File(actual);
  if (actualHash !== expectedHash) {
    throw new Error(
      `${label} SHA-256 does not match its reference: expected ${expectedHash}, found ${actualHash}`,
    );
  }
}

function assertNonEmptyRegularFile(path, label) {
  const metadata = lstatSync(path);
  if (!metadata.isFile() || metadata.size === 0) {
    throw new Error(`${label} must be a non-empty regular file: ${path}`);
  }
}

function assertSameDirectory(label, paths, caseInsensitive) {
  const directories = paths.map((path) => normalizedPath(dirname(path), caseInsensitive));
  if (!directories.every((path) => path === directories[0])) {
    throw new Error(`${label} files are not installed in one directory: ${paths.join(", ")}`);
  }
}

function assertExactPath(label, actual, expected, caseInsensitive = false) {
  if (normalizedPath(actual, caseInsensitive) !== normalizedPath(expected, caseInsensitive)) {
    throw new Error(`Expected ${label} at ${resolve(expected)}, found ${resolve(actual)}`);
  }
}

function normalizedPath(path, caseInsensitive) {
  const normalized = resolve(path);
  return caseInsensitive ? normalized.toLocaleLowerCase("en-US") : normalized;
}

function assertPathWithinRoot(root, path, message) {
  const fromRoot = relative(root, path);
  if (fromRoot === ".." || fromRoot.startsWith(`..${sep}`) || isAbsolute(fromRoot)) {
    throw new Error(message);
  }
}
