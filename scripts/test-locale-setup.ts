import { beforeEach } from "vitest";
import { setLanguagePreference } from "../src/i18n";

// Existing UI/diagnostic assertions use Chinese explicitly, independently of the host OS.
// Locale-specific tests select their own language after this reset.
beforeEach(() => setLanguagePreference("zh"));
