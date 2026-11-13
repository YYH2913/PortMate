import type { SessionProfile, ShellConnection } from "./types";

export function shellElevationRequested(profile: SessionProfile): boolean {
  return profile.connection.kind === "shell" && profile.connection.elevated === true;
}

export function localTerminalTag(elevated: boolean): string {
  return elevated ? "portmate-elevated-local-terminal" : "portmate-local-terminal";
}

export function matchesLocalTerminal(profile: SessionProfile, elevated: boolean): boolean {
  return profile.kind === "shell" && profile.connection.kind === "shell"
    && shellElevationRequested(profile) === elevated
    && (profile.tags.includes(localTerminalTag(elevated)) || (!elevated && profile.name === "local-terminal"));
}

/** Keep the saved result from silently broadening the configuration confirmed before save. */
export function shellElevationConsentMatches(profile: SessionProfile, consent: ShellConnection | null): boolean {
  if (!consent) return !shellElevationRequested(profile);
  const shell = profile.connection;
  return shell.kind === "shell" && shell.elevated === true && consent.elevated === true
    && shell.program === consent.program && (shell.cwd ?? null) === (consent.cwd ?? null)
    && shell.args.length === consent.args.length && shell.args.every((argument, index) => argument === consent.args[index]);
}
