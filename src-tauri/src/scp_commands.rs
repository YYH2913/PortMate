use super::*;

pub(super) fn scp_upload_command(remote_destination: &str, file_name: &str, total: u64) -> String {
    format!(
        concat!(
            "dst={}; source_name={}; total={}; target=; part=; ",
            "if [ -z \"$source_name\" ]; then source_name=portmate-upload.bin; fi; ",
            "case \"$dst\" in */) target=\"${{dst%/}}/$source_name\" ;; ",
            "*) if [ -d \"$dst\" ]; then target=\"${{dst%/}}/$source_name\"; else target=\"$dst\"; fi ;; esac; ",
            "case \"$target\" in */*) part=\"${{target%/*}}/${{target##*/}}.portmate-part\" ;; ",
            "*) part=\"$target.portmate-part\" ;; esac; ",
            "portable_path() {{ case \"$1\" in -*) printf './%s\\n' \"$1\" ;; *) printf '%s\\n' \"$1\" ;; esac; }}; ",
            "target=$(portable_path \"$target\") || exit 1; part=$(portable_path \"$part\") || exit 1; ",
            "reject_link() {{ if [ -L \"$1\" ]; then printf 'PortMate refuses symbolic link: %s\\n' \"$1\" >&2; return 1; fi; }}; ",
            "file_size() {{ value=$(wc -c < \"$1\") || return 1; value=$(printf '%s' \"$value\" | tr -d '[:space:]') || return 1; case \"$value\" in ''|*[!0-9]*) return 1 ;; esac; printf '%s\\n' \"$value\"; }}; ",
            "part_sha256() {{ if command -v sha256sum >/dev/null 2>&1; then value=$(sha256sum < \"$1\") || return 1; elif command -v shasum >/dev/null 2>&1; then value=$(shasum -a 256 < \"$1\") || return 1; elif command -v sha256 >/dev/null 2>&1; then value=$(sha256 -q \"$1\") || return 1; else printf 'PortMate SCP upload has no SHA-256 tool\\n' >&2; return 1; fi; value=${{value%% *}}; [ -n \"$value\" ] || return 1; printf '%s\\n' \"$value\"; }}; ",
            "if ! reject_link \"$part\" || ! reject_link \"$target\"; then exit 1; fi; ",
            "printf '__PORTMATE_SIZE__%s\\n' \"$total\"; ",
            "offset=0; ",
            "if [ -e \"$part\" ]; then ",
            "if current=$(file_size \"$part\" 2>/dev/null); then ",
            "if [ \"$current\" -eq 0 ]; then offset=0; elif [ \"$current\" -le \"$total\" ]; then ",
            "printf '__PORTMATE_RESUME_CANDIDATE__%s\\n' \"$current\"; ",
            "if ! IFS= read -r expected_prefix_sha256; then exit 1; fi; ",
            "if ! actual_prefix_sha256=$(part_sha256 \"$part\"); then printf 'PortMate SCP upload cannot hash resume prefix\\n' >&2; exit 1; fi; ",
            "if [ \"$expected_prefix_sha256\" = \"__PORTMATE_PREFIX_SHA256__$actual_prefix_sha256\" ]; then offset=$current; else : > \"$part\" || exit 1; fi; ",
            "else : > \"$part\" || exit 1; fi; ",
            "else : > \"$part\" || exit 1; fi; ",
            "else : > \"$part\" || exit 1; fi; ",
            "printf '__PORTMATE_RESUME__%s\\n' \"$offset\"; ",
            "printf '__PORTMATE_PROGRESS__%s\\n' \"$offset\"; ",
            "if [ \"$offset\" -lt \"$total\" ]; then ",
            "cat >> \"$part\" || exit 1; ",
            "if current=$(file_size \"$part\" 2>/dev/null); then ",
            "printf '__PORTMATE_PROGRESS__%s\\n' \"$current\"; ",
            "fi; ",
            "fi; ",
            "final=$(file_size \"$part\") || exit 1; ",
            "if [ \"$final\" -ne \"$total\" ]; then ",
            "printf 'PortMate SCP upload size mismatch: %s of %s\\n' \"$final\" \"$total\" >&2; exit 1; ",
            "fi; ",
            "if ! reject_link \"$part\" || ! reject_link \"$target\"; then exit 1; fi; ",
            "mv -f \"$part\" \"$target\" || exit 1; ",
            "final_target=$(file_size \"$target\") || exit 1; printf '__PORTMATE_DONE__%s\\n' \"$final_target\""
        ),
        shell_quote(remote_destination),
        shell_quote(file_name),
        total
    )
}

pub(super) fn scp_download_command(remote_source: &str) -> String {
    format!(
        "source={}; if [ -L \"$source\" ] || [ ! -f \"$source\" ]; then printf 'PortMate refuses symbolic link or non-file source: %s\\n' \"$source\" >&2; exit 1; fi; exec scp -f \"$source\"",
        shell_quote(remote_source)
    )
}
