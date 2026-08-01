export function compatibilityUsesCachedImages(env = process.env) {
  const mode = env.PORTMATE_COMPAT_USE_CACHED_IMAGES;
  if (mode !== undefined && !["0", "1"].includes(mode)) {
    throw new Error("PORTMATE_COMPAT_USE_CACHED_IMAGES must be 0 or 1");
  }
  return mode === "1";
}

export function filterCompatibilityEntries(
  entries,
  env = process.env,
  knownEntries = entries,
) {
  const knownNames = new Set();
  for (const entry of knownEntries) {
    const name = entry?.name;
    if (typeof name !== "string" || knownNames.has(name)) {
      throw new Error(`compatibility matrices contain an invalid or duplicate entry name: ${String(name)}`);
    }
    knownNames.add(name);
  }

  const raw = env.PORTMATE_COMPAT_FILTER;
  if (raw === undefined) return entries;
  if (typeof raw !== "string" || raw.length === 0 || raw.length > 8_192) {
    throw new Error("PORTMATE_COMPAT_FILTER must be a non-empty string of at most 8192 characters");
  }

  const requested = raw.split(",").map((name) => name.trim());
  if (!requested.length || requested.length > 256 || requested.some((name) => !name)) {
    throw new Error("PORTMATE_COMPAT_FILTER must contain 1 to 256 comma-separated names without empty entries");
  }
  for (const name of requested) {
    if (!/^[a-z0-9.-]+$/.test(name)) {
      throw new Error(`PORTMATE_COMPAT_FILTER contains an invalid entry name: ${name}`);
    }
  }
  const requestedNames = new Set(requested);
  if (requestedNames.size !== requested.length) {
    throw new Error("PORTMATE_COMPAT_FILTER must not contain duplicate entry names");
  }

  const unknown = requested.filter((name) => !knownNames.has(name));
  if (unknown.length) {
    throw new Error(`PORTMATE_COMPAT_FILTER contains unknown entries: ${unknown.join(",")}`);
  }
  return entries.filter(({ name }) => requestedNames.has(name));
}

export async function prepareCompatibilityImage({
  run,
  image,
  buildArgs,
  useCachedImages,
  buildOptions = {},
  inspectOptions = {},
  attempts = 3,
}) {
  if (useCachedImages) {
    const inspected = run("docker", ["image", "inspect", image], {
      ...inspectOptions,
      quiet: true,
      allowFailure: true,
    });
    if (inspected.status !== 0) {
      throw new Error(`cached compatibility image is unavailable: ${image}`);
    }
    return;
  }

  let lastError;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      run("docker", buildArgs, buildOptions);
      return;
    } catch (error) {
      lastError = error;
      if (attempt < attempts) {
        console.warn(`docker build attempt ${attempt}/${attempts} failed; retrying`);
        await new Promise((resolveWait) => setTimeout(resolveWait, attempt * 1_000));
      }
    }
  }
  throw lastError;
}
