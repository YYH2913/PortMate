export type TerminalCommandCatalogEntry = {
  value: string;
  /** Canonical English description. Localize only when presenting a suggestion. */
  detail: string;
  takesValue?: boolean;
};

export function terminalCommandDetailMessage(detail: string): string {
  return `command-help-${detail.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "")}`;
}

export type TerminalCommandSchema = TerminalCommandCatalogEntry & {
  usage: string;
  options: readonly TerminalCommandCatalogEntry[];
  arguments: readonly TerminalCommandCatalogEntry[];
  subcommands: readonly TerminalCommandSchema[];
};

type TerminalCommandSchemaInput = {
  options?: readonly TerminalCommandCatalogEntry[];
  arguments?: readonly TerminalCommandCatalogEntry[];
  subcommands?: readonly TerminalCommandSchema[];
};

const commonPathArguments = entries([".", "..", "~", "/tmp"], "Common paths");

export const terminalCommandCatalog: readonly TerminalCommandSchema[] = [
  ansibleCommand("ansible"),
  ansibleCommand("ansible-playbook"),
  apkCommand(),
  aptCommand("apt"),
  aptCommand("apt-get"),
  schema("awk", "Process structured text", "awk [options] <program> [file...]", {
    options: options(["-F", "-f", "-v"], "awk options", ["-F", "-f", "-v"]),
  }),
  schema("bash", "Start Bash shell", "bash [options] [script [args...]]", {
    options: options(["--help", "--version", "-c", "-i", "-l", "-n", "-x"], "Bash options"),
  }),
  bunCommand(),
  schema("cargo", "Rust Build and package management", "cargo [global-options] <subcommand> [args...]", {
    options: options(["--help", "--version", "--locked", "--offline", "-q"], "Cargo global-options"),
    subcommands: [
      schema("add", "Add dependencies", "cargo add [options] <dependency...>", { options: options(["--dev", "--optional", "--rename", "--features"], "cargo add options") }),
      schema("build", "Build the project", "cargo build [options]", { options: cargoBuildOptions() }),
      schema("check", "Check the project", "cargo check [options]", { options: cargoBuildOptions() }),
      schema("clean", "Clean build artifacts", "cargo clean [options]", { options: options(["--doc", "--package", "--release", "--target"], "cargo clean options") }),
      schema("clippy", "Run Clippy", "cargo clippy [options] [-- <Clippy args...>]", { options: cargoBuildOptions() }),
      schema("doc", "Generate documentation", "cargo doc [options]", { options: options(["--document-private-items", "--no-deps", "--open", "--package"], "cargo doc options") }),
      schema("fmt", "Format source code", "cargo fmt [options] [-- <rustfmt args...>]", { options: options(["--all", "--check", "--package"], "cargo fmt options") }),
      schema("run", "Run a binary", "cargo run [options] [-- <program-args...>]", { options: options(["--bin", "--example", "--features", "--package", "--release"], "cargo run options") }),
      schema("test", "Run tests", "cargo test [options] [filter] [-- <test-args...>]", { options: options(["--all-targets", "--features", "--lib", "--no-fail-fast", "--package", "--release", "--test"], "cargo test options") }),
      schema("update", "Update the dependency lockfile", "cargo update [options]", { options: options(["--aggressive", "--dry-run", "--package", "--precise"], "cargo update options") }),
    ],
  }),
  schema("cat", "Print file contents", "cat [options] [file...]", {
    options: options(["--number", "--show-all", "-A", "-b", "-n", "-s", "-v"], "cat options"),
    arguments: commonPathArguments,
  }),
  schema("cd", "Change directory", "cd [directory]", { arguments: commonPathArguments }),
  schema("chmod", "Change file permissions", "chmod [options] <mode> <path...>", {
    options: options(["--reference", "-R", "-f", "-v"], "chmod options"),
    arguments: entries(["600", "644", "700", "755", "a+r", "u+x"], "Common permission modes"),
  }),
  schema("chown", "Change file ownership", "chown [options] <owner[:group]> <path...>", {
    options: options(["--from", "--reference", "-R", "-h", "-v"], "chown options"),
  }),
  schema("cmake", "Configure or build CMake project", "cmake [options] <source-directory|build-directory>", {
    options: options(
      ["--build", "--install", "--preset", "-B", "-D", "-G", "-S"],
      "CMake options",
      ["--build", "--install", "--preset", "-B", "-D", "-G", "-S"],
    ),
  }),
  schema("cmd", "Run Windows command interpreter", "cmd [options] [command]", {
    options: options(["/c", "/d", "/e:off", "/e:on", "/k", "/q", "/s", "/u", "/v:off", "/v:on"], "cmd options", ["/c", "/k"]),
  }),
  schema("clear", "Clear the terminal screen", "clear"),
  schema("cp", "Copy files or directories", "cp [options] <source...> <target>", {
    options: options(["--parents", "-a", "-f", "-i", "-n", "-r", "-v"], "cp options"),
    arguments: commonPathArguments,
  }),
  schema("curl", "Transfer URL data", "curl [options] <URL...>", {
    options: options(["--connect-timeout", "--fail", "--max-time", "-H", "-I", "-L", "-o", "-X", "-d"], "curl options"),
  }),
  schema("cut", "Extract text by field or character", "cut <mode> [options] [file...]", {
    options: options(["--characters", "--delimiter", "--fields", "--only-delimited", "-b", "-c", "-d", "-f", "-s"], "cut options", ["--characters", "--delimiter", "--fields", "-b", "-c", "-d", "-f"]),
    arguments: commonPathArguments,
  }),
  schema("date", "Display or set the date and time", "date [options] [+format]", {
    options: options(["--date", "--iso-8601", "--reference", "--rfc-3339", "--set", "--utc", "-d", "-I", "-r", "-R", "-s", "-u"], "date options", ["--date", "--iso-8601", "--reference", "--rfc-3339", "--set", "-d", "-I", "-r", "-s"]),
  }),
  denoCommand(),
  schema("df", "Display filesystem space", "df [options] [filesystem...]", {
    options: options(["--human-readable", "--inodes", "--total", "-h", "-i", "-T"], "df options"),
  }),
  schema("dig", "query DNS records", "dig [@server] <name> [type] [options]", {
    options: options(["+short", "+tcp", "+trace", "-4", "-6", "-p", "-x"], "dig options", ["-p", "-x"]),
    arguments: entries(["A", "AAAA", "CNAME", "MX", "NS", "TXT"], "DNS record-type"),
  }),
  schema("dmesg", "Read kernel messages", "dmesg [options]", {
    options: options(["--follow", "--human", "--level", "--since", "--time-format", "-H", "-T", "-w"], "dmesg options", ["--level", "--since", "--time-format"]),
  }),
  rpmPackageCommand("dnf"),
  schema("docker", "Manage containers and images", "docker [global-options] <subcommand> [args...]", {
    options: options(
      ["--config", "--context", "--help", "--host", "--version"],
      "Docker global-options",
      ["--config", "--context", "--host"],
    ),
    subcommands: [
      schema("build", "Build an image", "docker build [options] <context>", { options: options(["--build-arg", "--file", "--no-cache", "--pull", "--tag"], "docker build options") }),
      schema("compose", "Manage Compose Apply", "docker compose [options] <subcommand> [args...]", {
        options: options(
          ["--env-file", "--file", "--profile", "--project-name"],
          "docker compose options",
          ["--env-file", "--file", "--profile", "--project-name"],
        ),
        subcommands: simpleSubcommands([
          ["build", "Build services", "docker compose build [options] [service...]"],
          ["down", "Stop and remove resources", "docker compose down [options]"],
          ["exec", "Execute a command in a service", "docker compose exec [options] <service> <command...>"],
          ["logs", "View service logs", "docker compose logs [options] [service...]"],
          ["ps", "List service containers", "docker compose ps [options]"],
          ["pull", "Pull service images", "docker compose pull [options] [service...]"],
          ["restart", "Restart services", "docker compose restart [options] [service...]"],
          ["start", "Start existing services", "docker compose start [service...]"],
          ["stop", "Stop services", "docker compose stop [options] [service...]"],
          ["up", "Create and start services", "docker compose up [options] [service...]"],
        ]),
      }),
      schema("exec", "Execute a command in a container", "docker exec [options] <container> <command...>", { options: options(["--detach", "--env", "--interactive", "--tty", "--user", "--workdir"], "docker exec options") }),
      schema("images", "List images", "docker images [options] [repository[:tag]]", { options: options(["--all", "--digests", "--filter", "--format", "--quiet"], "docker images options") }),
      schema("inspect", "Inspect objects", "docker inspect [options] <object...>", { options: options(["--format", "--size", "--type"], "docker inspect options") }),
      schema("logs", "View container logs", "docker logs [options] <container>", { options: options(["--follow", "--since", "--tail", "--timestamps", "--until"], "docker logs options") }),
      schema("ps", "List containers", "docker ps [options]", { options: options(["--all", "--filter", "--format", "--latest", "--quiet", "--size"], "docker ps options") }),
      schema("pull", "Pull an image", "docker pull [options] <image>", { options: options(["--all-tags", "--platform", "--quiet"], "docker pull options") }),
      schema("push", "Push an image", "docker push [options] <image>", { options: options(["--all-tags", "--quiet"], "docker push options") }),
      schema("run", "Create and run a container", "docker run [options] <image> [command...]", { options: options(["--detach", "--env", "--name", "--network", "--publish", "--rm", "--volume"], "docker run options") }),
    ],
  }),
  dotnetCommand(),
  schema("du", "Summarize file and directory space", "du [options] [path...]", {
    options: options(["--max-depth", "--summarize", "-a", "-h", "-s", "-x"], "du options", ["--max-depth"]),
    arguments: commonPathArguments,
  }),
  schema("echo", "Print text", "echo [options] [text...]", { options: options(["-E", "-e", "-n"], "echo options") }),
  schema("env", "Display or set the command environment", "env [options] [name=value...] [command [args...]]", {
    options: options(["--chdir", "--ignore-environment", "--unset", "-C", "-i", "-u"], "env options", ["--chdir", "--unset", "-C", "-u"]),
  }),
  schema("find", "Find files", "find [starting-path...] [expression]", {
    options: options(["-maxdepth", "-mindepth", "-mtime", "-name", "-path", "-size", "-type"], "find condition"),
    arguments: commonPathArguments,
  }),
  schema("free", "Display memory usage", "free [options]", {
    options: options(["--bytes", "--giga", "--human", "--mega", "--seconds", "-b", "-g", "-h", "-m", "-s"], "free options", ["--seconds", "-s"]),
  }),
  schema("git", "Manage Git repository", "git [global-options] <subcommand> [args...]", {
    options: options(
      ["--help", "--no-pager", "--version", "-C", "-c"],
      "Git global-options",
      ["-C", "-c"],
    ),
    subcommands: [
      schema("add", "Stage files", "git add [options] <path...>", { options: options(["--all", "--intent-to-add", "--patch", "--update", "-A", "-p", "-u"], "git add options") }),
      schema("branch", "Manage branches", "git branch [options] [branch-name]", { options: options(["--all", "--delete", "--move", "--remotes", "-D", "-a", "-d", "-m", "-r"], "git branch options") }),
      schema("checkout", "Switch branches or restore files", "git checkout [options] <branch|path>", { options: options(["--detach", "--force", "-B", "-b", "-f"], "git checkout options") }),
      schema("commit", "Commit staged changes", "git commit [options] [path...]", { options: options(["--amend", "--no-edit", "--signoff", "-S", "-a", "-m"], "git commit options") }),
      schema("diff", "Compare changes", "git diff [options] [commit] [--] [path...]", { options: options(["--cached", "--name-only", "--stat", "--word-diff"], "git diff options") }),
      schema("fetch", "Fetch remote references", "git fetch [options] [remote [ref...]]", { options: options(["--all", "--prune", "--tags", "-p"], "git fetch options") }),
      schema("log", "View commit history", "git log [options] [revision-range] [--] [path...]", { options: options(["--all", "--decorate", "--graph", "--oneline", "--stat", "-n"], "git log options") }),
      schema("pull", "Fetch and integrate remote changes", "git pull [options] [remote [branch]]", { options: options(["--ff-only", "--no-rebase", "--rebase", "--tags"], "git pull options") }),
      schema("push", "Push local references", "git push [options] [remote [ref...]]", { options: options(["--delete", "--force-with-lease", "--set-upstream", "--tags", "-u"], "git push options") }),
      schema("rebase", "Rebase commits", "git rebase [options] [upstream [branch]]", { options: options(["--abort", "--continue", "--interactive", "--onto", "--skip", "-i"], "git rebase options") }),
      schema("restore", "Restore working tree files", "git restore [options] <path...>", { options: options(["--source", "--staged", "--worktree", "-S", "-W"], "git restore options") }),
      schema("stash", "Manage stashed changes", "git stash <subcommand> [args...]", {
        subcommands: simpleSubcommands([
          ["apply", "Apply stash", "git stash apply [stash]"],
          ["drop", "Remove stash", "git stash drop [stash]"],
          ["list", "List stash", "git stash list [options]"],
          ["pop", "Apply and remove stash", "git stash pop [stash]"],
          ["push", "Save working tree changes", "git stash push [options] [--] [path...]"],
          ["show", "View stash", "git stash show [options] [stash]"],
        ]),
      }),
      schema("status", "View working tree status", "git status [options] [--] [path...]", { options: options(["--branch", "--porcelain", "--short", "--show-stash", "-b", "-s"], "git status options") }),
      schema("switch", "Switch branches", "git switch [options] <branch>", { options: options(["--create", "--detach", "--force-create", "-C", "-c"], "git switch options") }),
      schema("tag", "Manage tags", "git tag [options] [tag [object]]", { options: options(["--delete", "--list", "--sign", "-a", "-d", "-l", "-s"], "git tag options") }),
    ],
  }),
  goCommand(),
  schema("gradle", "Run Gradle Build", "gradle [options] [task...]", {
    options: options(["--build-cache", "--console", "--continue", "--daemon", "--debug", "--dry-run", "--info", "--no-daemon", "--offline", "--parallel", "--project-dir", "--quiet", "--refresh-dependencies", "--scan", "--stacktrace", "-b", "-p", "-q", "-x"], "Gradle options", ["--console", "--project-dir", "-b", "-p", "-x"]),
  }),
  schema("grep", "Search text", "grep [options] <mode> [file...]", { options: options(["--color=auto", "-E", "-F", "-i", "-n", "-r", "-v"], "grep options") }),
  schema("gzip", "Compress or decompress gzip file", "gzip [options] [file...]", {
    options: options(["--decompress", "--force", "--keep", "--recursive", "--stdout", "-c", "-d", "-f", "-k", "-r"], "gzip options"),
    arguments: commonPathArguments,
  }),
  schema("head", "Print the beginning of files", "head [options] [file...]", { options: options(["--bytes", "--lines", "-c", "-n", "-q", "-v"], "head options"), arguments: commonPathArguments }),
  helmCommand(),
  schema("hostname", "Display or set the hostname", "hostname [options] [name]", {
    options: options(["--all-fqdns", "--fqdn", "--ip-address", "--short", "-A", "-I", "-f", "-s"], "hostname options"),
  }),
  schema("htop", "Interactively view system processes", "htop [options]", {
    options: options(["--delay", "--filter", "--pid", "--sort-key", "--tree", "-d", "-p", "-s", "-t"], "htop options", ["--delay", "--filter", "--pid", "--sort-key", "-d", "-p", "-s"]),
  }),
  schema("id", "Display user and group identifiers", "id [options] [user]", {
    options: options(["--group", "--groups", "--name", "--real", "--user", "-G", "-g", "-n", "-r", "-u"], "id options"),
  }),
  ipCommand(),
  schema("ipconfig", "View Windows network configuration", "ipconfig [options]", {
    options: options(["/all", "/displaydns", "/flushdns", "/registerdns", "/release", "/release6", "/renew", "/renew6", "/showclassid", "/showclassid6"], "ipconfig options"),
  }),
  schema("java", "Run Java Apply", "java [options] <class|jar|module> [args...]", {
    options: options(["--class-path", "--enable-preview", "--jar", "--module", "--module-path", "--show-version", "--version", "-cp", "-jar", "-m", "-p"], "Java options", ["--class-path", "--module", "--module-path", "-cp", "-jar", "-m", "-p"]),
  }),
  schema("javac", "Compile Java source", "javac [options] <source-file...>", {
    options: options(["--class-path", "--enable-preview", "--module-path", "--release", "--source", "--target", "-classpath", "-cp", "-d", "-encoding", "-g"], "javac options", ["--class-path", "--module-path", "--release", "--source", "--target", "-classpath", "-cp", "-d", "-encoding"]),
    arguments: commonPathArguments,
  }),
  schema("journalctl", "query systemd logs", "journalctl [options] [match...]", { options: options(["--boot", "--follow", "--no-pager", "--since", "--until", "-b", "-f", "-n", "-u"], "journalctl options") }),
  schema("jq", "Query and transform JSON", "jq [options] <filter> [file...]", {
    options: options(["--arg", "--argjson", "--compact-output", "--exit-status", "--join-output", "--null-input", "--raw-output", "--slurp", "-c", "-e", "-j", "-n", "-r", "-s"], "jq options", ["--arg", "--argjson"]),
    arguments: commonPathArguments,
  }),
  kubectlCommand(),
  schema("kill", "Send a signal to processes", "kill [options] <PID...>", { options: options(["--list", "-HUP", "-INT", "-KILL", "-TERM", "-l", "-s"], "kill signal-or-options") }),
  schema("less", "View text one page at a time", "less [options] [file...]", {
    options: options(["--ignore-case", "--quit-if-one-screen", "--RAW-CONTROL-CHARS", "-F", "-N", "-R", "-S", "-i"], "less options"),
    arguments: commonPathArguments,
  }),
  schema("ln", "Create file links", "ln [options] <target...> [link-name]", {
    options: options(["--force", "--relative", "--symbolic", "-f", "-n", "-r", "-s", "-v"], "ln options"),
    arguments: commonPathArguments,
  }),
  schema("ls", "List directory contents", "ls [options] [path...]", { options: options(["--color=auto", "-R", "-a", "-h", "-l", "-t"], "ls options"), arguments: commonPathArguments }),
  schema("lsof", "List open files and sockets", "lsof [options] [name...]", {
    options: options(["-i", "-n", "-p", "-P", "-u"], "lsof options", ["-i", "-p", "-u"]),
  }),
  schema("make", "Run Make build-target", "make [options] [target...]", {
    options: options(["--directory", "--file", "--jobs", "--keep-going", "--silent", "-C", "-f", "-j", "-k", "-n", "-s"], "Make options", ["--directory", "--file", "--jobs", "-C", "-f", "-j"]),
  }),
  schema("man", "View command manuals", "man [options] [section] <name...>", {
    options: options(["--apropos", "--html", "--where", "-a", "-f", "-k", "-w"], "man options"),
  }),
  checksumCommand("md5sum", "MD5"),
  schema("mkdir", "Create directories", "mkdir [options] <directory...>", { options: options(["--mode", "--parents", "-m", "-p", "-v"], "mkdir options") }),
  schema("mount", "Mount a filesystem", "mount [options] [device] [directory]", {
    options: options(["--all", "--bind", "--options", "--read-only", "--types", "-B", "-L", "-U", "-a", "-o", "-r", "-t", "-v"], "mount options", ["--options", "--types", "-B", "-L", "-U", "-o", "-t"]),
    arguments: commonPathArguments,
  }),
  schema("mv", "Move or rename files", "mv [options] <source...> <target>", { options: options(["--backup", "--target-directory", "-f", "-i", "-n", "-t", "-v"], "mv options"), arguments: commonPathArguments }),
  schema("mvn", "Run Maven Build", "mvn [options] [phase|target...]", {
    options: options(["--activate-profiles", "--also-make", "--batch-mode", "--define", "--file", "--offline", "--projects", "--quiet", "--settings", "--threads", "-B", "-D", "-P", "-T", "-f", "-o", "-pl", "-q", "-s"], "Maven options", ["--activate-profiles", "--define", "--file", "--projects", "--settings", "--threads", "-D", "-P", "-T", "-f", "-pl", "-s"]),
  }),
  schema("nano", "Use Nano Edit files", "nano [options] [file...]", {
    options: options(["--linenumbers", "--mouse", "--nowrap", "--restricted", "-B", "-l", "-m", "-v"], "Nano options"),
    arguments: commonPathArguments,
  }),
  schema("nc", "Establish TCP or UDP connection", "nc [options] <host> <port>", {
    options: options(["-4", "-6", "-l", "-n", "-p", "-s", "-u", "-v", "-w", "-z"], "netcat options", ["-p", "-s", "-w"]),
  }),
  schema("netstat", "Display network connections and routes", "netstat [options]", {
    options: options(["--listening", "--numeric", "--program", "--route", "--tcp", "--udp", "-a", "-l", "-n", "-p", "-r", "-t", "-u"], "netstat options"),
  }),
  schema("node", "Run Node.js program", "node [options] [script [args...]]", {
    options: options(["--check", "--eval", "--inspect", "--print", "--require", "--test", "--version", "-c", "-e", "-p", "-r", "-v"], "Node.js options", ["--eval", "--inspect", "--print", "--require", "-e", "-p", "-r"]),
  }),
  schema("npx", "Execute npm package commands", "npx [options] <package|command> [args...]", {
    options: options(["--call", "--package", "--shell", "--yes", "-c", "-p", "-y"], "npx options", ["--call", "--package", "--shell", "-c", "-p"]),
  }),
  schema("npm", "Node.js package management", "npm [global-options] <subcommand> [args...]", {
    options: options(
      ["--help", "--silent", "--version", "--workspace"],
      "npm global-options",
      ["--workspace"],
    ),
    subcommands: simpleSubcommands([
      ["audit", "Audit dependencies", "npm audit [options]"],
      ["exec", "Execute a package command", "npm exec [options] -- <command...>"],
      ["install", "Install dependencies", "npm install [options] [package...]"],
      ["outdated", "Check outdated dependencies", "npm outdated [options]"],
      ["publish", "Publish a package", "npm publish [options]"],
      ["run", "Run package script", "npm run <script> [-- <args...>]"],
      ["start", "Run start script", "npm start [-- <args...>]"],
      ["test", "Run test script", "npm test [-- <args...>]"],
      ["update", "Update dependencies", "npm update [options] [package...]"],
    ]),
  }),
  schema("pnpm", "Efficient Node.js package management", "pnpm [global-options] <subcommand> [args...]", {
    options: options(
      ["--dir", "--filter", "--help", "--silent", "--version"],
      "pnpm global-options",
      ["--dir", "--filter"],
    ),
    subcommands: simpleSubcommands([
      ["add", "Add dependencies", "pnpm add [options] <package...>"],
      ["audit", "Audit dependencies", "pnpm audit [options]"],
      ["build", "Run build script", "pnpm build [-- <args...>]"],
      ["exec", "Execute a command", "pnpm exec <command...>"],
      ["install", "Install dependencies", "pnpm install [options]"],
      ["remove", "Remove dependencies", "pnpm remove [options] <package...>"],
      ["run", "Run package script", "pnpm run <script> [-- <args...>]"],
      ["test", "Run test script", "pnpm test [-- <args...>]"],
      ["update", "Update dependencies", "pnpm update [options] [package...]"],
    ]),
  }),
  schema("openssl", "Use OpenSSL cryptography tools", "openssl <subcommand> [options]", {
    subcommands: simpleSubcommands([
      ["dgst", "Compute message digests", "openssl dgst [options] [file...]"],
      ["rand", "Generate random data", "openssl rand [options] <byte-count>"],
      ["req", "Manage certificate requests", "openssl req [options]"],
      ["s_client", "Run TLS client", "openssl s_client [options]"],
      ["s_server", "Run TLS server", "openssl s_server [options]"],
      ["version", "Display OpenSSL version", "openssl version [options]"],
      ["x509", "Manage X.509 certificate", "openssl x509 [options]"],
    ]),
  }),
  schema("pacman", "Manage Arch Linux package", "pacman <operation> [options] [package...]", {
    options: options(["--noconfirm", "--needed", "-Q", "-R", "-S", "-Syu", "-U", "-Ss", "-Sy"], "pacman operation-or-options"),
  }),
  schema("ping", "Test network connectivity", "ping [options] <target>", {
    options: options(["-4", "-6", "-c", "-i", "-I", "-s", "-t", "-W"], "ping options", ["-c", "-i", "-I", "-s", "-t", "-W"]),
  }),
  pipCommand("pip"),
  pipCommand("pip3"),
  podmanCommand(),
  powershellCommand("powershell"),
  schema("ps", "Display process status", "ps [options]", {
    options: options(["--forest", "--pid", "--sort", "-A", "-a", "-e", "-f", "-o", "-u", "-x"], "ps options", ["--pid", "--sort", "-o", "-u"]),
  }),
  schema("pwd", "Display the current directory", "pwd [options]", { options: options(["-L", "-P"], "pwd options") }),
  powershellCommand("pwsh"),
  pythonCommand("python"),
  pythonCommand("python3"),
  schema("rg", "Use ripgrep Search text", "rg [options] <mode> [path...]", {
    options: options(["--files", "--glob", "--hidden", "--ignore-case", "--type", "-F", "-g", "-i", "-l", "-n", "-S", "-t", "-v"], "ripgrep options", ["--glob", "--type", "-g", "-t"]),
    arguments: commonPathArguments,
  }),
  schema("rm", "Remove files or directories", "rm [options] <path...>", { options: options(["--one-file-system", "-d", "-f", "-i", "-r", "-v"], "rm options"), arguments: commonPathArguments }),
  schema("rsync", "Synchronize files and directories", "rsync [options] <source...> <target>", { options: options(["--delete", "--dry-run", "--exclude", "--progress", "-a", "-e", "-n", "-v", "-z"], "rsync options") }),
  schema("rustc", "Compile Rust program", "rustc [options] <input-file>", {
    options: options(["--crate-name", "--crate-type", "--edition", "--emit", "--extern", "--out-dir", "-C", "-L", "-O", "-o"], "rustc options", ["--crate-name", "--crate-type", "--edition", "--emit", "--extern", "--out-dir", "-C", "-L", "-o"]),
    arguments: commonPathArguments,
  }),
  schema("scp", "via SSH Copy files", "scp [options] <source...> <target>", { options: options(["-3", "-C", "-J", "-P", "-i", "-p", "-r", "-v"], "SCP options") }),
  schema("screen", "Manage GNU Screen session", "screen [options] [command [args...]]", {
    options: options(["-D", "-L", "-R", "-S", "-d", "-ls", "-r", "-x", "-X"], "Screen options", ["-S", "-r", "-x", "-X"]),
  }),
  schema("sed", "Edit text as a stream", "sed [options] <script> [file...]", {
    options: options(["--expression", "--file", "--in-place", "--regexp-extended", "-E", "-e", "-f", "-i", "-n"], "sed options", ["--expression", "--file", "--in-place", "-e", "-f", "-i"]),
  }),
  schema("service", "Manage SysV service", "service <service> <operation> [args...]", {
    options: options(["--status-all", "-h"], "service options"),
  }),
  schema("sftp", "Interactively transfer SSH file", "sftp [options] <target>", {
    options: options(["-4", "-6", "-B", "-C", "-F", "-J", "-P", "-b", "-i", "-o", "-R", "-v"], "SFTP options", ["-B", "-F", "-J", "-P", "-b", "-i", "-o", "-R"]),
  }),
  schema("sh", "Start POSIX shell", "sh [options] [script [args...]]", {
    options: options(["-c", "-e", "-n", "-s", "-u", "-x"], "Shell options", ["-c"]),
  }),
  checksumCommand("sha256sum", "SHA-256"),
  schema("sort", "Sort lines of text", "sort [options] [file...]", {
    options: options(["--field-separator", "--key", "--numeric-sort", "--reverse", "--unique", "-k", "-n", "-r", "-t", "-u"], "sort options", ["--field-separator", "--key", "-k", "-t"]),
    arguments: commonPathArguments,
  }),
  schema("ss", "Display socket status", "ss [options] [filter]", {
    options: options(["--all", "--listening", "--numeric", "--processes", "--tcp", "--udp", "-a", "-l", "-n", "-p", "-t", "-u"], "ss options"),
  }),
  schema("ssh", "connection SSH host", "ssh [options] [user@]host [command...]", { options: options(["-4", "-6", "-A", "-D", "-J", "-L", "-R", "-i", "-o", "-p", "-t", "-v"], "SSH options") }),
  schema("ssh-add", "Manage ssh-agent identity", "ssh-add [options] [private-key-file...]", {
    options: options(["-D", "-K", "-L", "-T", "-c", "-d", "-l", "-t", "-x"], "ssh-add options", ["-T", "-t"]),
    arguments: commonPathArguments,
  }),
  schema("ssh-keygen", "Generate and manage SSH keys", "ssh-keygen [options]", {
    options: options(["-C", "-E", "-F", "-N", "-R", "-b", "-f", "-l", "-p", "-q", "-t", "-y"], "ssh-keygen options", ["-C", "-E", "-F", "-N", "-R", "-b", "-f", "-t"]),
  }),
  schema("stat", "Display file or filesystem status", "stat [options] <path...>", {
    options: options(["--dereference", "--file-system", "--format", "--printf", "-L", "-c", "-f"], "stat options", ["--format", "--printf", "-c"]),
    arguments: commonPathArguments,
  }),
  schema("sudo", "Execute a command as another user", "sudo [options] <command> [args...]", {
    options: options(["--chdir", "--group", "--preserve-env", "--user", "-E", "-H", "-S", "-g", "-i", "-u"], "sudo options", ["--chdir", "--group", "--user", "-g", "-u"]),
  }),
  schema("systemctl", "Manage systemd unit", "systemctl [options] <operation> [unit...]", {
    options: options(["--no-pager", "--now", "--system", "--user"], "systemctl global-options"),
    subcommands: simpleSubcommands([
      ["daemon-reload", "Reload unit definitions", "systemctl daemon-reload"],
      ["disable", "Disable units", "systemctl disable [options] <unit...>"],
      ["enable", "Enable units", "systemctl enable [options] <unit...>"],
      ["is-active", "Check active status", "systemctl is-active [options] <unit...>"],
      ["list-units", "List units", "systemctl list-units [options] [mode...]"],
      ["reload", "Reload units", "systemctl reload [options] <unit...>"],
      ["restart", "Restart units", "systemctl restart [options] <unit...>"],
      ["start", "Start units", "systemctl start [options] <unit...>"],
      ["status", "View unit status", "systemctl status [options] [unit...]"],
      ["stop", "Stop units", "systemctl stop [options] <unit...>"],
    ]),
  }),
  schema("taskkill", "Terminate Windows process", "taskkill [options]", {
    options: options(["/f", "/fi", "/im", "/pid", "/t"], "taskkill options", ["/fi", "/im", "/pid"]),
  }),
  schema("tasklist", "List Windows process", "tasklist [options]", {
    options: options(["/fi", "/fo", "/m", "/nh", "/svc", "/v"], "tasklist options", ["/fi", "/fo", "/m"]),
  }),
  schema("tail", "Print the end of files", "tail [options] [file...]", { options: options(["--follow", "--lines", "--pid", "--retry", "-F", "-f", "-n"], "tail options"), arguments: commonPathArguments }),
  schema("tar", "Archive files", "tar <operation> [options] [file...]", { options: options(["--directory", "--exclude", "-C", "-c", "-f", "-j", "-t", "-v", "-x", "-z"], "tar operation-or-options") }),
  schema("telnet", "connection Telnet service", "telnet [options] <host> [port]", {
    options: options(["-4", "-6", "-E", "-K", "-a", "-e", "-l", "-n"], "Telnet options", ["-e", "-l", "-n"]),
  }),
  schema("tee", "Copy standard input to files", "tee [options] [file...]", {
    options: options(["--append", "--ignore-interrupts", "--output-error", "-a", "-i", "-p"], "tee options", ["--output-error"]),
    arguments: commonPathArguments,
  }),
  terraformCommand(),
  tmuxCommand(),
  schema("touch", "Create files or update timestamps", "touch [options] <file...>", {
    options: options(["--date", "--reference", "-a", "-c", "-d", "-m", "-r", "-t"], "touch options", ["--date", "--reference", "-d", "-r", "-t"]),
    arguments: commonPathArguments,
  }),
  schema("top", "View system processes", "top [options]", { options: options(["-H", "-b", "-d", "-n", "-p", "-u"], "top options") }),
  schema("traceroute", "Trace network routes", "traceroute [options] <target> [packet-length]", {
    options: options(["-4", "-6", "-I", "-m", "-n", "-p", "-q", "-T", "-w"], "traceroute options", ["-m", "-p", "-q", "-w"]),
  }),
  schema("tree", "List directories as a tree", "tree [options] [path...]", {
    options: options(["--dirsfirst", "--filelimit", "--prune", "-a", "-d", "-L", "-p", "-s"], "tree options", ["--filelimit", "-L"]),
    arguments: commonPathArguments,
  }),
  schema("umount", "Unmount filesystems", "umount [options] <device|directory...>", {
    options: options(["--all", "--force", "--lazy", "--recursive", "--types", "-R", "-a", "-f", "-l", "-t", "-v"], "umount options", ["--types", "-t"]),
    arguments: commonPathArguments,
  }),
  schema("uname", "Display system information", "uname [options]", { options: options(["--all", "--kernel-release", "--machine", "--operating-system", "-a", "-m", "-r", "-s"], "uname options") }),
  schema("unzip", "Extract ZIP archive", "unzip [options] <archive> [file...]", {
    options: options(["-d", "-j", "-l", "-n", "-o", "-q", "-t"], "unzip options", ["-d"]),
    arguments: commonPathArguments,
  }),
  schema("vi", "Start Vi editor", "vi [options] [file...]", { options: options(["-R", "-c", "-d", "-n", "-u"], "Vi options"), arguments: commonPathArguments }),
  schema("vim", "Start Vim editor", "vim [options] [file...]", { options: options(["--clean", "-R", "-c", "-d", "-n", "-u"], "Vim options"), arguments: commonPathArguments }),
  schema("watch", "Run and display a command periodically", "watch [options] <command...>", {
    options: options(["--beep", "--color", "--differences", "--errexit", "--interval", "--no-title", "--precise", "-b", "-c", "-d", "-e", "-n", "-p", "-t"], "watch options", ["--differences", "--interval", "-d", "-n"]),
  }),
  schema("wc", "Count lines、words and bytes", "wc [options] [file...]", {
    options: options(["--bytes", "--chars", "--lines", "--words", "-c", "-l", "-m", "-w"], "wc options"),
    arguments: commonPathArguments,
  }),
  schema("wget", "Download network resources", "wget [options] <URL...>", { options: options(["--continue", "--directory-prefix", "--output-document", "--quiet", "-O", "-P", "-c", "-q"], "wget options") }),
  schema("where", "Locate Windows executable", "where [options] <mode...>", {
    options: options(["/f", "/q", "/r", "/t"], "where options", ["/r"]),
  }),
  schema("which", "Locate executable commands", "which [options] <command...>", {
    options: options(["--all", "--read-alias", "-a", "-i"], "which options"),
  }),
  schema("whoami", "Display the current user", "whoami [options]", { options: options(["--help", "--version"], "whoami options") }),
  wingetCommand(),
  schema("xargs", "Build commands from standard input", "xargs [options] [command [initial-args...]]", {
    options: options(["--max-args", "--max-procs", "--null", "--replace", "-0", "-I", "-n", "-P", "-r"], "xargs options", ["--max-args", "--max-procs", "--replace", "-I", "-n", "-P"]),
  }),
  yarnCommand(),
  rpmPackageCommand("yum"),
  schema("zip", "Create or update ZIP archive", "zip [options] <archive> [file...]", {
    options: options(["--recurse-paths", "-1", "-9", "-d", "-e", "-j", "-q", "-r", "-u"], "zip options"),
    arguments: commonPathArguments,
  }),
  schema("zsh", "Start Z shell", "zsh [options] [script [args...]]", {
    options: options(["--no-rcs", "-c", "-d", "-f", "-i", "-l", "-n", "-x"], "Zsh options", ["-c"]),
  }),
];

export function terminalCommandSchema(value: string): TerminalCommandSchema | null {
  return terminalCommandCatalog.find((command) => command.value === value) ?? null;
}

export function terminalCommandSubcommand(
  command: TerminalCommandSchema,
  value: string,
): TerminalCommandSchema | null {
  return command.subcommands.find((subcommand) => subcommand.value === value) ?? null;
}

function schema(
  value: string,
  detail: string,
  usage: string,
  input: TerminalCommandSchemaInput = {},
): TerminalCommandSchema {
  return {
    value,
    detail,
    usage,
    options: input.options ?? [],
    arguments: input.arguments ?? [],
    subcommands: input.subcommands ?? [],
  };
}

function entries(values: readonly string[], detail: string): TerminalCommandCatalogEntry[] {
  return values.map((value) => ({ value, detail }));
}

function options(
  values: readonly string[],
  detail: string,
  takesValue: readonly string[] = [],
): TerminalCommandCatalogEntry[] {
  const valuedOptions = new Set(takesValue);
  return values.map((value) => ({ value, detail, takesValue: valuedOptions.has(value) }));
}

function simpleSubcommands(
  values: readonly (readonly [value: string, detail: string, usage: string])[],
): TerminalCommandSchema[] {
  return values.map(([value, detail, usage]) => schema(value, detail, usage));
}

function cargoBuildOptions(): TerminalCommandCatalogEntry[] {
  return options(["--all-targets", "--features", "--package", "--release", "--target", "--workspace"], "Cargo build options");
}

function ansibleCommand(command: "ansible" | "ansible-playbook"): TerminalCommandSchema {
  const common = options(
    ["--ask-become-pass", "--ask-pass", "--become", "--become-user", "--check", "--diff", "--extra-vars", "--forks", "--inventory", "--limit", "--private-key", "--tags", "--vault-id", "--version", "-C", "-D", "-K", "-b", "-e", "-f", "-i", "-k", "-l", "-t", "-u"],
    `${command} options`,
    ["--become-user", "--extra-vars", "--forks", "--inventory", "--limit", "--private-key", "--tags", "--vault-id", "-e", "-f", "-i", "-l", "-t", "-u"],
  );
  return command === "ansible"
    ? schema(command, "Execute in bulk Ansible task", "ansible <host-pattern> [options]", {
      options: [...common, ...options(["--args", "--background", "--module-name", "--one-line", "--poll", "-B", "-P", "-a", "-m", "-o"], "ansible ad-hoc options", ["--args", "--background", "--module-name", "--poll", "-B", "-P", "-a", "-m"])],
    })
    : schema(command, "Execute Ansible Playbook", "ansible-playbook [options] <playbook...>", {
      options: [...common, ...options(["--flush-cache", "--force-handlers", "--list-hosts", "--list-tags", "--list-tasks", "--start-at-task", "--step", "--syntax-check"], "ansible-playbook options", ["--start-at-task"])],
      arguments: commonPathArguments,
    });
}

function bunCommand(): TerminalCommandSchema {
  return schema("bun", "Run JavaScript and manage packages", "bun [global-options] <subcommand|file> [args...]", {
    options: options(["--bun", "--cwd", "--filter", "--silent", "--version"], "Bun global-options", ["--cwd", "--filter"]),
    subcommands: [
      schema("add", "Add dependencies", "bun add [options] <package...>", { options: options(["--dev", "--exact", "--global", "--optional", "--peer", "-d", "-E", "-g", "-o", "-p"], "bun add options") }),
      schema("build", "Bundle source code", "bun build [options] <entry...>", { options: options(["--compile", "--format", "--minify", "--outdir", "--outfile", "--sourcemap", "--target"], "bun build options", ["--format", "--outdir", "--outfile", "--sourcemap", "--target"]) }),
      schema("create", "Create a project from a template", "bun create [options] <template> [directory]"),
      schema("install", "Install dependencies", "bun install [options]", { options: options(["--frozen-lockfile", "--ignore-scripts", "--no-save", "--production"], "bun install options") }),
      schema("remove", "Remove dependencies", "bun remove [options] <package...>"),
      schema("run", "Run a script or file", "bun run [options] <script|file> [args...]"),
      schema("test", "Run tests", "bun test [options] [filter...]", { options: options(["--bail", "--coverage", "--only", "--preload", "--rerun-each", "--timeout", "--watch"], "bun test options", ["--bail", "--preload", "--rerun-each", "--timeout"]) }),
      schema("update", "Update dependencies", "bun update [options] [package...]", { options: options(["--latest"], "bun update options") }),
      schema("x", "Execute a package binary", "bun x [options] <package> [args...]"),
    ],
  });
}

function checksumCommand(command: "md5sum" | "sha256sum", algorithm: string): TerminalCommandSchema {
  return schema(command, `Compute or verify ${algorithm}`, `${command} [options] [file...]`, {
    options: options(["--binary", "--check", "--ignore-missing", "--quiet", "--status", "--strict", "--tag", "--text", "--warn", "-b", "-c", "-t", "-w"], `${command} options`),
    arguments: commonPathArguments,
  });
}

function denoCommand(): TerminalCommandSchema {
  return schema("deno", "Run JavaScript and TypeScript", "deno [global-options] <subcommand> [args...]", {
    options: options(["--config", "--no-config", "--quiet", "--version", "-c", "-q"], "Deno global-options", ["--config", "-c"]),
    subcommands: [
      schema("cache", "Cache dependencies", "deno cache [options] <file...>"),
      schema("check", "Check types", "deno check [options] <file...>"),
      schema("compile", "Compile a standalone executable", "deno compile [options] <script> [args...]", { options: options(["--allow-all", "--output", "--target", "-A", "-o"], "deno compile options", ["--output", "--target", "-o"]) }),
      schema("fmt", "Format source code", "deno fmt [options] [file...]", { options: options(["--check", "--ignore", "--line-width"], "deno fmt options", ["--ignore", "--line-width"]) }),
      schema("info", "Display dependency and cache information", "deno info [options] [file]"),
      schema("install", "Install a script command", "deno install [options] <script> [args...]", { options: options(["--allow-all", "--global", "--name", "--root", "-A", "-g", "-n"], "deno install options", ["--name", "--root", "-n"]) }),
      schema("lint", "Check source code", "deno lint [options] [file...]", { options: options(["--compact", "--ignore", "--json", "--rules"], "deno lint options", ["--ignore", "--rules"]) }),
      schema("repl", "Start an interactive interpreter", "deno repl [options]"),
      schema("run", "Run a script", "deno run [options] <script> [args...]", { options: options(["--allow-all", "--allow-env", "--allow-net", "--allow-read", "--allow-run", "--allow-write", "--watch", "-A"], "deno run permission options") }),
      schema("task", "Run a configured task", "deno task [options] [task] [args...]"),
      schema("test", "Run tests", "deno test [options] [file...]", { options: options(["--allow-all", "--coverage", "--fail-fast", "--filter", "--parallel", "--watch", "-A"], "deno test options", ["--coverage", "--fail-fast", "--filter"]) }),
    ],
  });
}

function dotnetCommand(): TerminalCommandSchema {
  return schema("dotnet", "Build and run .NET project", "dotnet [global-options] <command> [args...]", {
    options: options(["--diagnostics", "--info", "--list-runtimes", "--list-sdks", "--roll-forward", "--version"], ".NET global-options", ["--roll-forward"]),
    subcommands: [
      schema("add", "Add project references or packages", "dotnet add <project> <package|reference> [args...]", { subcommands: simpleSubcommands([
        ["package", "Add NuGet package", "dotnet add package <package> [options]"],
        ["reference", "Add project references", "dotnet add reference <project...> [options]"],
      ]) }),
      schema("build", "Build the project", "dotnet build [project] [options]", { options: dotnetBuildOptions("dotnet build options") }),
      schema("clean", "Clean build output", "dotnet clean [project] [options]", { options: dotnetBuildOptions("dotnet clean options") }),
      schema("new", "Create projects or files", "dotnet new <template> [options]", { options: options(["--dry-run", "--force", "--language", "--name", "--output", "--type", "-lang", "-n", "-o"], "dotnet new options", ["--language", "--name", "--output", "--type", "-lang", "-n", "-o"]) }),
      schema("pack", "Create NuGet package", "dotnet pack [project] [options]", { options: dotnetBuildOptions("dotnet pack options") }),
      schema("publish", "Publish the application", "dotnet publish [project] [options]", { options: dotnetBuildOptions("dotnet publish options") }),
      schema("restore", "Restore dependencies", "dotnet restore [project] [options]", { options: options(["--force", "--locked-mode", "--no-cache", "--packages", "--runtime", "--source", "-r", "-s"], "dotnet restore options", ["--packages", "--runtime", "--source", "-r", "-s"]) }),
      schema("run", "Run the project", "dotnet run [options] [-- <args...>]", { options: options(["--configuration", "--framework", "--launch-profile", "--no-build", "--project", "-c", "-f", "-p"], "dotnet run options", ["--configuration", "--framework", "--launch-profile", "--project", "-c", "-f", "-p"]) }),
      schema("test", "Run tests", "dotnet test [project] [options]", { options: [...dotnetBuildOptions("dotnet test options"), ...options(["--filter", "--logger", "--settings"], "dotnet test options", ["--filter", "--logger", "--settings"])] }),
      schema("tool", "Manage .NET tool", "dotnet tool <command> [args...]", { subcommands: simpleSubcommands([
        ["install", "Install tools", "dotnet tool install <package> [options]"],
        ["list", "List tools", "dotnet tool list [options]"],
        ["restore", "Restore local tools", "dotnet tool restore [options]"],
        ["uninstall", "Uninstall tools", "dotnet tool uninstall <package> [options]"],
        ["update", "Update tools", "dotnet tool update <package> [options]"],
      ]) }),
      schema("workload", "Manage optional workloads", "dotnet workload <command> [args...]", { subcommands: simpleSubcommands([
        ["install", "Install workloads", "dotnet workload install <workload...> [options]"],
        ["list", "List workloads", "dotnet workload list [options]"],
        ["repair", "Repair workloads", "dotnet workload repair [options]"],
        ["uninstall", "Uninstall workloads", "dotnet workload uninstall <workload...> [options]"],
        ["update", "Update workloads", "dotnet workload update [options]"],
      ]) }),
    ],
  });
}

function dotnetBuildOptions(detail: string): TerminalCommandCatalogEntry[] {
  return options(["--configuration", "--framework", "--no-restore", "--output", "--runtime", "--self-contained", "--verbosity", "-c", "-f", "-o", "-r", "-v"], detail, ["--configuration", "--framework", "--output", "--runtime", "--verbosity", "-c", "-f", "-o", "-r", "-v"]);
}

function apkCommand(): TerminalCommandSchema {
  return schema("apk", "Manage Alpine Linux package", "apk [global-options] <subcommand> [args...]", {
    options: options(["--no-cache", "--repository", "--root", "--update-cache", "-p", "-X"], "apk global-options", ["--repository", "--root", "-p", "-X"]),
    subcommands: [
      schema("add", "Install packages", "apk add [options] <package...>", { options: options(["--no-cache", "--repository", "--upgrade", "--virtual", "-X"], "apk add options", ["--repository", "--virtual", "-X"]) }),
      schema("del", "Remove packages", "apk del [options] <package...>", { options: options(["--purge", "--rdepends"], "apk del options") }),
      schema("info", "Display package information", "apk info [options] [package...]", { options: options(["--all", "--contents", "--depends", "--installed", "-a", "-e", "-L", "-R"], "apk info options") }),
      schema("search", "Search packages", "apk search [options] <mode...>", { options: options(["--description", "--exact", "--origin", "-d", "-e", "-o"], "apk search options") }),
      schema("update", "Update the package index", "apk update [options]", { options: options(["--no-cache"], "apk update options") }),
      schema("upgrade", "Upgrade installed packages", "apk upgrade [options]", { options: options(["--available", "--no-cache", "--prune"], "apk upgrade options") }),
    ],
  });
}

function aptCommand(command: "apt" | "apt-get"): TerminalCommandSchema {
  const detail = command === "apt" ? "Manage Debian package" : "Use APT backend management Debian package";
  return schema(command, detail, `${command} [global-options] <subcommand> [args...]`, {
    options: options(["--option", "--quiet", "--simulate", "--yes", "-o", "-q", "-s", "-y"], `${command} global-options`, ["--option", "-o"]),
    subcommands: [
      schema("autoremove", "Remove unused dependencies", `${command} autoremove [options] [package...]`, { options: options(["--purge", "--simulate", "--yes", "-s", "-y"], `${command} autoremove options`) }),
      schema("clean", "Clean the download cache", `${command} clean [options]`, { options: options(["--simulate", "-s"], `${command} clean options`) }),
      schema("download", "Download binary packages", `${command} download [options] <package...>`, { options: options(["--download-only", "--reinstall"], `${command} download options`) }),
      schema("install", "Install packages", `${command} install [options] <package...>`, { options: options(["--download-only", "--no-install-recommends", "--reinstall", "--simulate", "--yes", "-s", "-y"], `${command} install options`) }),
      schema("remove", "Remove packages", `${command} remove [options] <package...>`, { options: options(["--purge", "--simulate", "--yes", "-s", "-y"], `${command} remove options`) }),
      schema("update", "Update the package index", `${command} update [options]`, { options: options(["--allow-insecure-repositories", "--quiet", "-q"], `${command} update options`) }),
      schema("upgrade", "Upgrade installed packages", `${command} upgrade [options]`, { options: options(["--download-only", "--simulate", "--with-new-pkgs", "--yes", "-s", "-y"], `${command} upgrade options`) }),
      ...(command === "apt" ? [
        schema("list", "List packages", "apt list [options] [mode...]", { options: options(["--all-versions", "--installed", "--upgradable"], "apt list options") }),
        schema("search", "Search packages", "apt search [options] <mode...>", { options: options(["--full"], "apt search options") }),
        schema("show", "Display package details", "apt show [options] <package...>", { options: options(["--all-versions"], "apt show options") }),
      ] : [
        schema("dist-upgrade", "Upgrade and resolve dependency changes", "apt-get dist-upgrade [options]", { options: options(["--download-only", "--simulate", "--yes", "-s", "-y"], "apt-get dist-upgrade options") }),
      ]),
    ],
  });
}

function rpmPackageCommand(command: "dnf" | "yum"): TerminalCommandSchema {
  return schema(command, `Manage RPM package（${command}）`, `${command} [global-options] <subcommand> [args...]`, {
    options: options(["--assumeno", "--assumeyes", "--disablerepo", "--enablerepo", "--releasever", "-y"], `${command} global-options`, ["--disablerepo", "--enablerepo", "--releasever"]),
    subcommands: [
      schema("clean", "Clean the cache", `${command} clean [options] <type...>`, { arguments: entries(["all", "expire-cache", "metadata", "packages"], `${command} cache-type`) }),
      schema("info", "Display package information", `${command} info [options] [package...]`, { options: options(["--available", "--installed"], `${command} info options`) }),
      schema("install", "Install packages", `${command} install [options] <package...>`, { options: options(["--allowerasing", "--downloadonly", "--nogpgcheck", "-y"], `${command} install options`) }),
      schema("list", "List packages", `${command} list [options] [package...]`, { options: options(["--available", "--installed", "--updates"], `${command} list options`) }),
      schema("remove", "Remove packages", `${command} remove [options] <package...>`, { options: options(["--noautoremove", "-y"], `${command} remove options`) }),
      schema("repolist", "List package repositories", `${command} repolist [options]`, { options: options(["--all", "--disabled", "--enabled"], `${command} repolist options`) }),
      schema("search", "Search packages", `${command} search [options] <mode...>`, { options: options(["--all"], `${command} search options`) }),
      schema("upgrade", "Upgrade packages", `${command} upgrade [options] [package...]`, { options: options(["--allowerasing", "--refresh", "-y"], `${command} upgrade options`) }),
    ],
  });
}

function goCommand(): TerminalCommandSchema {
  return schema("go", "Build and manage Go project", "go <subcommand> [args...]", {
    subcommands: [
      schema("build", "Compile packages and dependencies", "go build [options] [package...]", { options: options(["-a", "-buildvcs", "-mod", "-o", "-race", "-tags", "-v"], "go build options", ["-buildvcs", "-mod", "-o", "-tags"]) }),
      schema("clean", "Delete the build cache", "go clean [options] [package...]", { options: options(["-cache", "-modcache", "-testcache"], "go clean options") }),
      schema("env", "Display or set Go environment", "go env [options] [variable...]", { options: options(["-json", "-u", "-w"], "go env options") }),
      schema("fmt", "Format Go package", "go fmt [options] [package...]", { options: options(["-n", "-x"], "go fmt options") }),
      schema("get", "Resolve and add dependencies", "go get [options] <package...>", { options: options(["-d", "-t", "-u"], "go get options") }),
      schema("install", "Compile and install packages", "go install [options] <package...>", { options: options(["-race", "-tags", "-v"], "go install options", ["-tags"]) }),
      schema("mod", "Manage Go module", "go mod <subcommand> [args...]", {
        subcommands: simpleSubcommands([
          ["download", "Download modules", "go mod download [options] [module...]"],
          ["edit", "Edit go.mod", "go mod edit [options]"],
          ["graph", "Print the module dependency graph", "go mod graph"],
          ["init", "Create go.mod", "go mod init [module-path]"],
          ["tidy", "Tidy module dependencies", "go mod tidy [options]"],
          ["vendor", "Generate vendor directory", "go mod vendor [options]"],
          ["verify", "Verify module contents", "go mod verify"],
        ]),
      }),
      schema("run", "Compile and run Go program", "go run [options] <package|file...> [args...]", { options: options(["-exec", "-mod", "-race", "-tags"], "go run options", ["-exec", "-mod", "-tags"]) }),
      schema("test", "Test Go package", "go test [options] [package...]", { options: options(["-bench", "-count", "-cover", "-race", "-run", "-short", "-v"], "go test options", ["-bench", "-count", "-run"]) }),
      schema("version", "Display Go version", "go version [options]", { options: options(["-m", "-v"], "go version options") }),
      schema("vet", "Report suspicious code", "go vet [options] [package...]", { options: options(["-json", "-v"], "go vet options") }),
    ],
  });
}

function helmCommand(): TerminalCommandSchema {
  const kubeOptions = ["--kube-context", "--kubeconfig", "--namespace", "-n"];
  return schema("helm", "Manage Kubernetes Helm Chart", "helm [global-options] <command> [args...]", {
    options: options(kubeOptions, "Helm global-options", kubeOptions),
    subcommands: [
      schema("dependency", "Manage Chart dependency", "helm dependency <command> [args...]", { subcommands: simpleSubcommands([
        ["build", "Rebuild Chart dependency", "helm dependency build [Chart] [options]"],
        ["list", "List Chart dependency", "helm dependency list [Chart] [options]"],
        ["update", "Update Chart dependency", "helm dependency update [Chart] [options]"],
      ]) }),
      schema("get", "Read Release information", "helm get <command> <Release> [options]", { subcommands: simpleSubcommands([
        ["all", "Read all Release information", "helm get all <Release> [options]"],
        ["hooks", "Read Release Hook", "helm get hooks <Release> [options]"],
        ["manifest", "Read Release Manifest", "helm get manifest <Release> [options]"],
        ["notes", "Read Release Notes", "helm get notes <Release> [options]"],
        ["values", "Read Release Values", "helm get values <Release> [options]"],
      ]) }),
      schema("history", "View Release history", "helm history <Release> [options]"),
      schema("install", "Install Chart", "helm install <Release> <Chart> [options]", { options: helmReleaseOptions("helm install options") }),
      schema("list", "List Release", "helm list [options]", { options: options(["--all", "--all-namespaces", "--date", "--filter", "--output", "--pending", "-A", "-a", "-f", "-o"], "helm list options", ["--filter", "--output", "-f", "-o"]) }),
      schema("repo", "Manage Chart repository", "helm repo <command> [args...]", { subcommands: simpleSubcommands([
        ["add", "Add Chart repository", "helm repo add <name> <URL> [options]"],
        ["index", "Generate a repository index", "helm repo index <directory> [options]"],
        ["list", "List Chart repository", "helm repo list [options]"],
        ["remove", "Remove Chart repository", "helm repo remove <name...>"],
        ["update", "Update Chart repository", "helm repo update [name...] [options]"],
      ]) }),
      schema("rollback", "Roll back Release", "helm rollback <Release> [version] [options]", { options: options(["--cleanup-on-fail", "--dry-run", "--force", "--recreate-pods", "--timeout", "--wait"], "helm rollback options", ["--timeout"]) }),
      schema("search", "Search Chart", "helm search <hub|repo> <keyword> [options]", { arguments: entries(["hub", "repo"], "Helm search-source") }),
      schema("status", "View Release status", "helm status <Release> [options]"),
      schema("template", "Render locally Chart", "helm template [Release] <Chart> [options]", { options: helmReleaseOptions("helm template options") }),
      schema("test", "Run Release Test", "helm test <Release> [options]"),
      schema("uninstall", "Uninstall Release", "helm uninstall <Release...> [options]", { options: options(["--dry-run", "--keep-history", "--no-hooks", "--timeout", "--wait"], "helm uninstall options", ["--timeout"]) }),
      schema("upgrade", "Upgrade Release", "helm upgrade <Release> <Chart> [options]", { options: helmReleaseOptions("helm upgrade options") }),
    ],
  });
}

function helmReleaseOptions(detail: string): TerminalCommandCatalogEntry[] {
  return options(["--atomic", "--create-namespace", "--dependency-update", "--dry-run", "--set", "--set-file", "--set-string", "--timeout", "--values", "--version", "--wait", "-f"], detail, ["--set", "--set-file", "--set-string", "--timeout", "--values", "--version", "-f"]);
}

function ipCommand(): TerminalCommandSchema {
  return schema("ip", "View and configure Linux network", "ip [global-options] <object> <command> [args...]", {
    options: options(["-4", "-6", "-brief", "-details", "-json", "-oneline", "-stats"], "ip global-options"),
    subcommands: [
      schema("address", "Manage network addresses", "ip address <command> [args...]", { subcommands: ipObjectSubcommands("address") }),
      schema("addr", "Manage network addresses", "ip addr <command> [args...]", { subcommands: ipObjectSubcommands("addr") }),
      schema("link", "Manage network interfaces", "ip link <command> [args...]", { subcommands: ipObjectSubcommands("link") }),
      schema("neighbor", "Manage the neighbor table", "ip neighbor <command> [args...]", { subcommands: ipObjectSubcommands("neighbor") }),
      schema("neigh", "Manage the neighbor table", "ip neigh <command> [args...]", { subcommands: ipObjectSubcommands("neigh") }),
      schema("route", "Manage the routing table", "ip route <command> [args...]", { subcommands: ipObjectSubcommands("route") }),
      schema("rule", "Manage routing policies", "ip rule <command> [args...]", { subcommands: ipObjectSubcommands("rule") }),
    ],
  });
}

function ipObjectSubcommands(object: string): TerminalCommandSchema[] {
  return simpleSubcommands([
    ["add", `Add ${object} entry`, `ip ${object} add [args...]`],
    ["delete", `Remove ${object} entry`, `ip ${object} delete [args...]`],
    ["flush", `Clear ${object} entry`, `ip ${object} flush [args...]`],
    ["get", `query ${object} entry`, `ip ${object} get [args...]`],
    ["replace", `Replace ${object} entry`, `ip ${object} replace [args...]`],
    ["show", `Display ${object} entry`, `ip ${object} show [args...]`],
  ]);
}

function kubectlCommand(): TerminalCommandSchema {
  const namespaceOptions = ["--all-namespaces", "--namespace", "-A", "-n"];
  return schema("kubectl", "Manage Kubernetes cluster", "kubectl [global-options] <subcommand> [args...]", {
    options: options(["--context", "--kubeconfig", "--namespace", "-n"], "kubectl global-options", ["--context", "--kubeconfig", "--namespace", "-n"]),
    subcommands: [
      schema("apply", "Apply resource configuration", "kubectl apply [options] -f <file|URL>", { options: options(["--filename", "--namespace", "--prune", "--server-side", "-f", "-n"], "kubectl apply options", ["--filename", "--namespace", "-f", "-n"]) }),
      schema("config", "Manage kubeconfig", "kubectl config <subcommand> [args...]", { subcommands: simpleSubcommands([
        ["current-context", "Display the current context", "kubectl config current-context"],
        ["get-contexts", "List contexts", "kubectl config get-contexts [name...]"],
        ["set-context", "Set context properties", "kubectl config set-context <name> [options]"],
        ["use-context", "Switch the current context", "kubectl config use-context <name>"],
      ]) }),
      schema("create", "Create resources", "kubectl create [options] -f <file|URL>", { options: options(["--filename", "--namespace", "--save-config", "-f", "-n"], "kubectl create options", ["--filename", "--namespace", "-f", "-n"]) }),
      schema("delete", "Delete resources", "kubectl delete [options] <type> <name...>", { options: options(["--all", "--filename", "--force", "--namespace", "--wait", "-f", "-n"], "kubectl delete options", ["--filename", "--namespace", "-f", "-n"]) }),
      schema("describe", "Display resource details", "kubectl describe [options] <type> [name]", { options: options(namespaceOptions, "kubectl describe options", ["--namespace", "-n"]) }),
      schema("exec", "Execute a command in a container", "kubectl exec [options] <Pod> -- <command...>", { options: options(["--container", "--namespace", "--stdin", "--tty", "-c", "-i", "-n", "-t"], "kubectl exec options", ["--container", "--namespace", "-c", "-n"]) }),
      schema("get", "List Kubernetes resource", "kubectl get [options] <type> [name]", { options: options([...namespaceOptions, "--output", "--selector", "--show-labels", "-o", "-l"], "kubectl get options", ["--namespace", "--output", "--selector", "-n", "-o", "-l"]) }),
      schema("logs", "View container logs", "kubectl logs [options] <Pod> [container]", { options: options(["--container", "--follow", "--namespace", "--previous", "--since", "--tail", "-c", "-f", "-n"], "kubectl logs options", ["--container", "--namespace", "--since", "--tail", "-c", "-n"]) }),
      schema("rollout", "Manage workload rollouts", "kubectl rollout <subcommand> [args...]", { subcommands: simpleSubcommands([
        ["history", "View rollout history", "kubectl rollout history <resource> [options]"],
        ["restart", "Restart workloads", "kubectl rollout restart <resource> [options]"],
        ["status", "View rollout status", "kubectl rollout status <resource> [options]"],
        ["undo", "Roll back a rollout", "kubectl rollout undo <resource> [options]"],
      ]) }),
      schema("scale", "Adjust the replica count", "kubectl scale [options] <resource>", { options: options(["--current-replicas", "--replicas", "--timeout"], "kubectl scale options", ["--current-replicas", "--replicas", "--timeout"]) }),
      schema("top", "Display resource usage", "kubectl top <pod|node> [name] [options]", { arguments: entries(["node", "pod"], "kubectl top resource-type") }),
    ],
  });
}

function pipCommand(command: "pip" | "pip3"): TerminalCommandSchema {
  return schema(command, "Manage Python package", `${command} [global-options] <subcommand> [args...]`, {
    options: options(["--isolated", "--no-input", "--proxy", "--python", "--require-virtualenv", "--timeout", "--version"], `${command} global-options`, ["--proxy", "--python", "--timeout"]),
    subcommands: [
      schema("check", "Check dependency compatibility", `${command} check`, {}),
      schema("download", "Download packages", `${command} download [options] <package...>`, { options: pipIndexOptions(`${command} download options`) }),
      schema("freeze", "Print installed packages", `${command} freeze [options]`, { options: options(["--all", "--exclude", "--local", "--user"], `${command} freeze options`, ["--exclude"]) }),
      schema("install", "Install Python package", `${command} install [options] <package...>`, { options: [
        ...pipIndexOptions(`${command} install options`),
        ...options(["--break-system-packages", "--editable", "--no-cache-dir", "--requirement", "--upgrade", "-e", "-r", "-U"], `${command} install options`, ["--editable", "--requirement", "-e", "-r"]),
      ] }),
      schema("list", "List installed packages", `${command} list [options]`, { options: options(["--editable", "--format", "--not-required", "--outdated", "--uptodate"], `${command} list options`, ["--format"]) }),
      schema("show", "Display package details", `${command} show [options] <package...>`, { options: options(["--files", "--verbose", "-f", "-v"], `${command} show options`) }),
      schema("uninstall", "Uninstall Python package", `${command} uninstall [options] <package...>`, { options: options(["--requirement", "--yes", "-r", "-y"], `${command} uninstall options`, ["--requirement", "-r"]) }),
      schema("wheel", "Build Wheel", `${command} wheel [options] <package...>`, { options: pipIndexOptions(`${command} wheel options`) }),
    ],
  });
}

function pipIndexOptions(detail: string): TerminalCommandCatalogEntry[] {
  return options(["--extra-index-url", "--find-links", "--index-url", "--no-index", "-f", "-i"], detail, ["--extra-index-url", "--find-links", "--index-url", "-f", "-i"]);
}

function powershellCommand(command: "powershell" | "pwsh"): TerminalCommandSchema {
  return schema(command, "Run PowerShell", `${command} [options] [-Command command | -File script] [args...]`, {
    options: options(
      ["-Command", "-CommandWithArgs", "-ConfigurationName", "-EncodedCommand", "-ExecutionPolicy", "-File", "-InputFormat", "-Interactive", "-Login", "-NoExit", "-NoLogo", "-NonInteractive", "-NoProfile", "-OutputFormat", "-SettingsFile", "-Version", "-WindowStyle", "-WorkingDirectory"],
      "PowerShell options",
      ["-Command", "-CommandWithArgs", "-ConfigurationName", "-EncodedCommand", "-ExecutionPolicy", "-File", "-InputFormat", "-OutputFormat", "-SettingsFile", "-Version", "-WindowStyle", "-WorkingDirectory"],
    ),
    arguments: commonPathArguments,
  });
}

function podmanCommand(): TerminalCommandSchema {
  return schema("podman", "Manage OCI containers and images", "podman [global-options] <subcommand> [args...]", {
    options: options(["--connection", "--log-level", "--remote", "--root", "--runtime"], "Podman global-options", ["--connection", "--log-level", "--root", "--runtime"]),
    subcommands: [
      schema("build", "Build container images", "podman build [options] <context>", { options: options(["--build-arg", "--file", "--no-cache", "--pull", "--tag"], "podman build options", ["--build-arg", "--file", "--pull", "--tag"]) }),
      schema("exec", "Execute a command in a container", "podman exec [options] <container> <command...>", { options: options(["--detach", "--env", "--interactive", "--tty", "--user", "--workdir"], "podman exec options", ["--env", "--user", "--workdir"]) }),
      schema("images", "List images", "podman images [options] [image]", { options: options(["--all", "--filter", "--format", "--quiet"], "podman images options", ["--filter", "--format"]) }),
      schema("inspect", "Inspect objects", "podman inspect [options] <object...>", { options: options(["--format", "--size", "--type"], "podman inspect options", ["--format", "--type"]) }),
      schema("logs", "View container logs", "podman logs [options] <container>", { options: options(["--follow", "--since", "--tail", "--timestamps", "--until"], "podman logs options", ["--since", "--tail", "--until"]) }),
      schema("ps", "List containers", "podman ps [options]", { options: options(["--all", "--filter", "--format", "--latest", "--quiet", "--size"], "podman ps options", ["--filter", "--format"]) }),
      schema("pull", "Pull an image", "podman pull [options] <image>", { options: options(["--all-tags", "--arch", "--os", "--quiet"], "podman pull options", ["--arch", "--os"]) }),
      schema("push", "Push an image", "podman push [options] <image> [target]", { options: options(["--compression-format", "--digestfile", "--quiet"], "podman push options", ["--compression-format", "--digestfile"]) }),
      schema("run", "Create and run a container", "podman run [options] <image> [command...]", { options: options(["--detach", "--env", "--name", "--network", "--publish", "--rm", "--volume"], "podman run options", ["--env", "--name", "--network", "--publish", "--volume"]) }),
    ],
  });
}

function pythonCommand(command: "python" | "python3"): TerminalCommandSchema {
  return schema(command, "Run Python interpreter", `${command} [options] [-c command | -m module | script] [args...]`, {
    options: options(["--help", "--version", "-B", "-c", "-E", "-I", "-m", "-O", "-q", "-u", "-V", "-W", "-X"], "Python options", ["-c", "-m", "-W", "-X"]),
    arguments: commonPathArguments,
  });
}

function terraformCommand(): TerminalCommandSchema {
  return schema("terraform", "Manage infrastructure configuration", "terraform [global-options] <command> [args...]", {
    options: options(["-chdir", "-help", "-version"], "Terraform global-options", ["-chdir"]),
    subcommands: [
      schema("apply", "Apply an execution plan", "terraform apply [options] [plan-file]", { options: terraformPlanOptions("terraform apply options") }),
      schema("destroy", "Destroy managed infrastructure", "terraform destroy [options]", { options: terraformPlanOptions("terraform destroy options") }),
      schema("fmt", "Format configuration", "terraform fmt [options] [target...]", { options: options(["-check", "-diff", "-list", "-recursive", "-write"], "terraform fmt options") }),
      schema("force-unlock", "Release a state lock", "terraform force-unlock [options] <lock ID>"),
      schema("get", "Install or update modules", "terraform get [options]", { options: options(["-update"], "terraform get options") }),
      schema("graph", "Generate a dependency graph", "terraform graph [options]", { options: options(["-draw-cycles", "-plan", "-type"], "terraform graph options", ["-type"]) }),
      schema("import", "Import existing resources", "terraform import [options] <address> <ID>", { options: terraformVariableOptions("terraform import options") }),
      schema("init", "Initialize the working directory", "terraform init [options]", { options: options(["-backend", "-backend-config", "-force-copy", "-from-module", "-get", "-lockfile", "-migrate-state", "-plugin-dir", "-reconfigure", "-upgrade"], "terraform init options", ["-backend-config", "-from-module", "-lockfile", "-plugin-dir"]) }),
      schema("output", "Read output values", "terraform output [options] [name]", { options: options(["-json", "-raw", "-state"], "terraform output options", ["-state"]) }),
      schema("plan", "Create an execution plan", "terraform plan [options]", { options: terraformPlanOptions("terraform plan options") }),
      schema("providers", "Display Provider requirements", "terraform providers [options]"),
      schema("refresh", "Refresh state", "terraform refresh [options]", { options: terraformVariableOptions("terraform refresh options") }),
      schema("show", "Show state or a plan", "terraform show [options] [file]", { options: options(["-json", "-no-color"], "terraform show options") }),
      schema("state", "Manage Terraform status", "terraform state <command> [args...]", { subcommands: simpleSubcommands([
        ["list", "List state resources", "terraform state list [options] [address...]"],
        ["mv", "Move a state address", "terraform state mv [options] <source> <target>"],
        ["pull", "Download remote state", "terraform state pull"],
        ["push", "Upload local state", "terraform state push [options] <file>"],
        ["replace-provider", "Replace Provider address", "terraform state replace-provider [options] <source> <target>"],
        ["rm", "Remove resources from state", "terraform state rm [options] <address...>"],
        ["show", "Show state resources", "terraform state show [options] <address>"],
      ]) }),
      schema("test", "Execute Terraform Test", "terraform test [options]", { options: options(["-filter", "-json", "-test-directory", "-verbose"], "terraform test options", ["-filter", "-test-directory"]) }),
      schema("validate", "Validate configuration", "terraform validate [options]", { options: options(["-json", "-no-color"], "terraform validate options") }),
      schema("version", "Display Terraform version", "terraform version [options]", { options: options(["-json"], "terraform version options") }),
      schema("workspace", "Manage workspaces", "terraform workspace <command> [args...]", { subcommands: simpleSubcommands([
        ["delete", "Delete a workspace", "terraform workspace delete [options] <name>"],
        ["list", "List workspaces", "terraform workspace list"],
        ["new", "Create a workspace", "terraform workspace new [options] <name>"],
        ["select", "Select a workspace", "terraform workspace select [options] <name>"],
        ["show", "Display the current workspace", "terraform workspace show"],
      ]) }),
    ],
  });
}

function terraformPlanOptions(detail: string): TerminalCommandCatalogEntry[] {
  return [
    ...terraformVariableOptions(detail),
    ...options(["-auto-approve", "-compact-warnings", "-destroy", "-input", "-lock", "-lock-timeout", "-no-color", "-out", "-parallelism", "-refresh", "-refresh-only", "-replace", "-target"], detail, ["-input", "-lock", "-lock-timeout", "-out", "-parallelism", "-refresh", "-replace", "-target"]),
  ];
}

function terraformVariableOptions(detail: string): TerminalCommandCatalogEntry[] {
  return options(["-var", "-var-file"], detail, ["-var", "-var-file"]);
}

function tmuxCommand(): TerminalCommandSchema {
  return schema("tmux", "Manage Tmux terminal sessions", "tmux [global-options] <command> [args...]", {
    options: options(["-2", "-C", "-D", "-L", "-S", "-f", "-l", "-u", "-v"], "Tmux global-options", ["-L", "-S", "-f", "-l"]),
    subcommands: [
      schema("attach-session", "Attach to a session", "tmux attach-session [options]", { options: tmuxTargetOptions("tmux attach-session options") }),
      schema("has-session", "Check whether a session exists", "tmux has-session [options]", { options: tmuxTargetOptions("tmux has-session options") }),
      schema("kill-pane", "Close pane", "tmux kill-pane [options]", { options: tmuxTargetOptions("tmux kill-pane options") }),
      schema("kill-server", "Close Tmux server", "tmux kill-server"),
      schema("kill-session", "Close a session", "tmux kill-session [options]", { options: tmuxTargetOptions("tmux kill-session options") }),
      schema("kill-window", "Close a window", "tmux kill-window [options]", { options: tmuxTargetOptions("tmux kill-window options") }),
      schema("list-clients", "List clients", "tmux list-clients [options]"),
      schema("list-keys", "List key bindings", "tmux list-keys [options]"),
      schema("list-panes", "List pane", "tmux list-panes [options]", { options: options(["-F", "-a", "-f", "-s", "-t"], "tmux list-panes options", ["-F", "-f", "-t"]) }),
      schema("list-sessions", "List sessions", "tmux list-sessions [options]", { options: options(["-F", "-f"], "tmux list-sessions options", ["-F", "-f"]) }),
      schema("list-windows", "List windows", "tmux list-windows [options]", { options: options(["-F", "-a", "-f", "-t"], "tmux list-windows options", ["-F", "-f", "-t"]) }),
      schema("new-session", "Create a session", "tmux new-session [options] [command]", { options: options(["-A", "-D", "-P", "-c", "-d", "-e", "-F", "-n", "-s", "-x", "-y"], "tmux new-session options", ["-c", "-e", "-F", "-n", "-s", "-x", "-y"]) }),
      schema("new-window", "Create a window", "tmux new-window [options] [command]", { options: options(["-P", "-a", "-c", "-d", "-e", "-F", "-n", "-t"], "tmux new-window options", ["-c", "-e", "-F", "-n", "-t"]) }),
      schema("rename-session", "Rename a session", "tmux rename-session [options] <name>", { options: tmuxTargetOptions("tmux rename-session options") }),
      schema("rename-window", "Rename a window", "tmux rename-window [options] <name>", { options: tmuxTargetOptions("tmux rename-window options") }),
      schema("select-pane", "Select pane", "tmux select-pane [options]", { options: tmuxTargetOptions("tmux select-pane options") }),
      schema("select-window", "Select a window", "tmux select-window [options]", { options: tmuxTargetOptions("tmux select-window options") }),
      schema("send-keys", "To pane Send keys", "tmux send-keys [options] <keys...>", { options: tmuxTargetOptions("tmux send-keys options") }),
      schema("split-window", "Split a window", "tmux split-window [options] [command]", { options: options(["-P", "-b", "-c", "-d", "-e", "-F", "-h", "-l", "-p", "-t", "-v"], "tmux split-window options", ["-c", "-e", "-F", "-l", "-p", "-t"]) }),
      schema("switch-client", "Switch client sessions", "tmux switch-client [options]", { options: tmuxTargetOptions("tmux switch-client options") }),
    ],
  });
}

function tmuxTargetOptions(detail: string): TerminalCommandCatalogEntry[] {
  return options(["-a", "-d", "-t"], detail, ["-t"]);
}

function wingetCommand(): TerminalCommandSchema {
  const globalOptions = ["--accept-source-agreements", "--authentication-account", "--disable-interactivity", "--logs", "--nowarn", "--open-logs", "--proxy", "--source", "--verbose", "--wait"];
  return schema("winget", "Manage Windows package", "winget [global-options] <command> [args...]", {
    options: options(globalOptions, "winget global-options", ["--authentication-account", "--proxy", "--source"]),
    subcommands: [
      schema("configure", "Apply system configuration", "winget configure [options] <file>", { options: options(["--accept-configuration-agreements", "--enable", "--file", "--module-path"], "winget configure options", ["--enable", "--file", "--module-path"]) }),
      schema("download", "Download an installer", "winget download [options] <query>", { options: wingetPackageOptions("winget download options") }),
      schema("export", "Export installed packages", "winget export [options] -o <file>", { options: options(["--include-versions", "--output", "-o"], "winget export options", ["--output", "-o"]) }),
      schema("hash", "Compute installer hashes", "winget hash [options] <file>", { options: options(["--file", "--msix", "-f", "-m"], "winget hash options", ["--file", "-f"]) }),
      schema("import", "Import a package list", "winget import [options] -i <file>", { options: options(["--accept-package-agreements", "--accept-source-agreements", "--ignore-unavailable", "--import-file", "--no-upgrade", "-i"], "winget import options", ["--import-file", "-i"]) }),
      schema("install", "Install packages", "winget install [options] <query>", { options: wingetPackageOptions("winget install options") }),
      schema("list", "List installed packages", "winget list [options] [query]", { options: wingetPackageOptions("winget list options") }),
      schema("pin", "Manage package pins", "winget pin <command> [args...]", { subcommands: simpleSubcommands([
        ["add", "Add a pin rule", "winget pin add [options] <query>"],
        ["list", "List pin rules", "winget pin list [options]"],
        ["remove", "Remove a pin rule", "winget pin remove [options] <query>"],
        ["reset", "Reset pin rules", "winget pin reset [options]"],
      ]) }),
      schema("search", "Search packages", "winget search [options] <query>", { options: wingetPackageOptions("winget search options") }),
      schema("settings", "Open winget settings", "winget settings [options]"),
      schema("show", "Display package details", "winget show [options] <query>", { options: wingetPackageOptions("winget show options") }),
      schema("source", "Manage package sources", "winget source <command> [args...]", { subcommands: simpleSubcommands([
        ["add", "Add a package source", "winget source add [options]"],
        ["export", "Export package sources", "winget source export [options]"],
        ["list", "List package sources", "winget source list [options]"],
        ["remove", "Remove a package source", "winget source remove [options]"],
        ["reset", "Reset package sources", "winget source reset [options]"],
        ["update", "Update package sources", "winget source update [options]"],
      ]) }),
      schema("uninstall", "Uninstall packages", "winget uninstall [options] <query>", { options: wingetPackageOptions("winget uninstall options") }),
      schema("upgrade", "Upgrade packages", "winget upgrade [options] [query]", { options: wingetPackageOptions("winget upgrade options") }),
      schema("validate", "Validate a package manifest", "winget validate [options] <manifest>", { options: options(["--manifest", "-m"], "winget validate options", ["--manifest", "-m"]) }),
    ],
  });
}

function wingetPackageOptions(detail: string): TerminalCommandCatalogEntry[] {
  return options(["--accept-package-agreements", "--architecture", "--custom", "--exact", "--force", "--id", "--interactive", "--location", "--manifest", "--moniker", "--name", "--override", "--scope", "--silent", "--source", "--tag", "--version", "-e", "-h", "-i", "-m", "-s", "-v"], detail, ["--architecture", "--custom", "--id", "--location", "--manifest", "--moniker", "--name", "--override", "--scope", "--source", "--tag", "--version", "-m", "-s", "-v"]);
}

function yarnCommand(): TerminalCommandSchema {
  return schema("yarn", "Manage JavaScript dependencies and scripts", "yarn [global-options] <subcommand> [args...]", {
    options: options(["--cwd", "--help", "--silent", "--version"], "Yarn global-options", ["--cwd"]),
    subcommands: [
      schema("add", "Add dependencies", "yarn add [options] <package...>", { options: options(["--dev", "--exact", "--peer", "--tilde", "-D", "-E", "-P", "-T"], "yarn add options") }),
      schema("install", "Install dependencies", "yarn install [options]", { options: options(["--immutable", "--mode", "--refresh-lockfile"], "yarn install options", ["--mode"]) }),
      schema("remove", "Remove dependencies", "yarn remove [options] <package...>", { options: options(["--mode"], "yarn remove options", ["--mode"]) }),
      schema("run", "Run package script", "yarn run <script> [args...]", {}),
      schema("set", "Modify Yarn configuration", "yarn set <subcommand> [args...]", { subcommands: simpleSubcommands([
        ["resolution", "Override dependency resolution", "yarn set resolution <descriptor> <ref>"],
        ["version", "settings Yarn version", "yarn set version [options] <version>"],
      ]) }),
      schema("up", "Upgrade dependencies", "yarn up [options] <package...>", { options: options(["--exact", "--interactive", "--mode", "-E", "-i"], "yarn up options", ["--mode"]) }),
    ],
  });
}
