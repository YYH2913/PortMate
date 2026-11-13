import { ShieldAlert } from "lucide-react";
import { t, useLocale } from "./i18n";
import { shellElevationRequested } from "./shell-elevation";
import type { SessionProfile } from "./types";

/** A launch preference, not a claim about the current process token. */
export default function ShellElevationIndicator({ profile }: { profile: SessionProfile }) {
  useLocale();
  return shellElevationRequested(profile)
    ? <span className="shell-elevation-indicator" role="img" aria-label={t("shell-elevation-mode")} title={t("shell-elevation-mode")}><ShieldAlert size={12} aria-hidden="true" /></span>
    : null;
}
