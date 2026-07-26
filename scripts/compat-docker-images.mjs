export function compatibilityUsesCachedImages(env = process.env) {
  const mode = env.PORTMATE_COMPAT_USE_CACHED_IMAGES;
  if (mode !== undefined && !["0", "1"].includes(mode)) {
    throw new Error("PORTMATE_COMPAT_USE_CACHED_IMAGES must be 0 or 1");
  }
  return mode === "1";
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
