export function pythonSdkLockFilename(platform = process.platform) {
  return platform === "win32" ? "requirements-windows.txt" : "requirements.txt";
}
