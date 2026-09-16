# Translation catalog contract

The six catalogs use the same English message IDs. UI code calls `t(id, values)`
or `tr(id, ReactNodes)`; only application-owned presentation text is translated.
Every locale must define every English catalog key. Unsupported system languages
still resolve to English.

- Keep `{0}`, `{1}`, and other numbered placeholders intact, including repeated
  occurrences. Do not translate or recursively interpolate their values.
- Keep technical identifiers, command syntax, protocol names, paths, and keyboard
  mnemonics such as `(P)` intact. Form options must declare explicit English or
  numeric values independently of their translated labels.
- Do not translate user-authored names, scripts, commands, or terminal output.
- Use `localizeDiagnostic` only at diagnostic presentation sites. Its native
  templates identify application-owned messages; unknown OS/device diagnostics
  remain verbatim. Normal notices are not treated as diagnostics by default.
- Native-message IDs are stable English-prefixed hashes of the original message
  template. When backend copy changes, update the template registry and all six
  translations together. `causeIndices` identifies only nested error causes,
  never path/name parameters.

Catalog, placeholder, mnemonic, form-value, and native-template coverage checks
run with `npm test`. `PORTMATE_UI_I18N_ONLY=1 npm run test:workspace-ui` covers live
language switching, RTL layout, numeric settings, serial enums, and window sync.
Translation models and generation tools are not application dependencies; the
application ships only these static catalogs.
