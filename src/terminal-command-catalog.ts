export type TerminalCommandCatalogEntry = {
  value: string;
  detail: string;
  takesValue?: boolean;
};

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

const commonPathArguments = entries([".", "..", "~", "/tmp"], "常用路径");

export const terminalCommandCatalog: readonly TerminalCommandSchema[] = [
  ansibleCommand("ansible"),
  ansibleCommand("ansible-playbook"),
  apkCommand(),
  aptCommand("apt"),
  aptCommand("apt-get"),
  schema("awk", "处理结构化文本", "awk [选项] <程序> [文件...]", {
    options: options(["-F", "-f", "-v"], "awk 选项", ["-F", "-f", "-v"]),
  }),
  schema("bash", "启动 Bash shell", "bash [选项] [脚本 [参数...]]", {
    options: options(["--help", "--version", "-c", "-i", "-l", "-n", "-x"], "Bash 选项"),
  }),
  bunCommand(),
  schema("cargo", "Rust 构建与包管理", "cargo [全局选项] <子命令> [参数...]", {
    options: options(["--help", "--version", "--locked", "--offline", "-q"], "Cargo 全局选项"),
    subcommands: [
      schema("add", "添加依赖", "cargo add [选项] <依赖...>", { options: options(["--dev", "--optional", "--rename", "--features"], "cargo add 选项") }),
      schema("build", "构建项目", "cargo build [选项]", { options: cargoBuildOptions() }),
      schema("check", "检查项目", "cargo check [选项]", { options: cargoBuildOptions() }),
      schema("clean", "清理构建产物", "cargo clean [选项]", { options: options(["--doc", "--package", "--release", "--target"], "cargo clean 选项") }),
      schema("clippy", "运行 Clippy", "cargo clippy [选项] [-- <Clippy 参数...>]", { options: cargoBuildOptions() }),
      schema("doc", "生成文档", "cargo doc [选项]", { options: options(["--document-private-items", "--no-deps", "--open", "--package"], "cargo doc 选项") }),
      schema("fmt", "格式化源码", "cargo fmt [选项] [-- <rustfmt 参数...>]", { options: options(["--all", "--check", "--package"], "cargo fmt 选项") }),
      schema("run", "运行二进制", "cargo run [选项] [-- <程序参数...>]", { options: options(["--bin", "--example", "--features", "--package", "--release"], "cargo run 选项") }),
      schema("test", "运行测试", "cargo test [选项] [过滤器] [-- <测试参数...>]", { options: options(["--all-targets", "--features", "--lib", "--no-fail-fast", "--package", "--release", "--test"], "cargo test 选项") }),
      schema("update", "更新依赖锁定", "cargo update [选项]", { options: options(["--aggressive", "--dry-run", "--package", "--precise"], "cargo update 选项") }),
    ],
  }),
  schema("cat", "输出文件内容", "cat [选项] [文件...]", {
    options: options(["--number", "--show-all", "-A", "-b", "-n", "-s", "-v"], "cat 选项"),
    arguments: commonPathArguments,
  }),
  schema("cd", "切换当前目录", "cd [目录]", { arguments: commonPathArguments }),
  schema("chmod", "修改文件权限", "chmod [选项] <模式> <路径...>", {
    options: options(["--reference", "-R", "-f", "-v"], "chmod 选项"),
    arguments: entries(["600", "644", "700", "755", "a+r", "u+x"], "常用权限模式"),
  }),
  schema("chown", "修改文件所有者", "chown [选项] <所有者[:组]> <路径...>", {
    options: options(["--from", "--reference", "-R", "-h", "-v"], "chown 选项"),
  }),
  schema("cmake", "配置或构建 CMake 项目", "cmake [选项] <源码目录|构建目录>", {
    options: options(
      ["--build", "--install", "--preset", "-B", "-D", "-G", "-S"],
      "CMake 选项",
      ["--build", "--install", "--preset", "-B", "-D", "-G", "-S"],
    ),
  }),
  schema("cmd", "运行 Windows 命令解释器", "cmd [选项] [命令]", {
    options: options(["/c", "/d", "/e:off", "/e:on", "/k", "/q", "/s", "/u", "/v:off", "/v:on"], "cmd 选项", ["/c", "/k"]),
  }),
  schema("clear", "清空终端屏幕", "clear"),
  schema("cp", "复制文件或目录", "cp [选项] <源...> <目标>", {
    options: options(["--parents", "-a", "-f", "-i", "-n", "-r", "-v"], "cp 选项"),
    arguments: commonPathArguments,
  }),
  schema("curl", "传输 URL 数据", "curl [选项] <URL...>", {
    options: options(["--connect-timeout", "--fail", "--max-time", "-H", "-I", "-L", "-o", "-X", "-d"], "curl 选项"),
  }),
  schema("cut", "按字段或字符提取文本", "cut <模式> [选项] [文件...]", {
    options: options(["--characters", "--delimiter", "--fields", "--only-delimited", "-b", "-c", "-d", "-f", "-s"], "cut 选项", ["--characters", "--delimiter", "--fields", "-b", "-c", "-d", "-f"]),
    arguments: commonPathArguments,
  }),
  schema("date", "显示或设置日期时间", "date [选项] [+格式]", {
    options: options(["--date", "--iso-8601", "--reference", "--rfc-3339", "--set", "--utc", "-d", "-I", "-r", "-R", "-s", "-u"], "date 选项", ["--date", "--iso-8601", "--reference", "--rfc-3339", "--set", "-d", "-I", "-r", "-s"]),
  }),
  denoCommand(),
  schema("df", "显示文件系统空间", "df [选项] [文件系统...]", {
    options: options(["--human-readable", "--inodes", "--total", "-h", "-i", "-T"], "df 选项"),
  }),
  schema("dig", "查询 DNS 记录", "dig [@服务器] <名称> [类型] [选项]", {
    options: options(["+short", "+tcp", "+trace", "-4", "-6", "-p", "-x"], "dig 选项", ["-p", "-x"]),
    arguments: entries(["A", "AAAA", "CNAME", "MX", "NS", "TXT"], "DNS 记录类型"),
  }),
  schema("dmesg", "读取内核消息", "dmesg [选项]", {
    options: options(["--follow", "--human", "--level", "--since", "--time-format", "-H", "-T", "-w"], "dmesg 选项", ["--level", "--since", "--time-format"]),
  }),
  rpmPackageCommand("dnf"),
  schema("docker", "管理容器与镜像", "docker [全局选项] <子命令> [参数...]", {
    options: options(
      ["--config", "--context", "--help", "--host", "--version"],
      "Docker 全局选项",
      ["--config", "--context", "--host"],
    ),
    subcommands: [
      schema("build", "构建镜像", "docker build [选项] <上下文>", { options: options(["--build-arg", "--file", "--no-cache", "--pull", "--tag"], "docker build 选项") }),
      schema("compose", "管理 Compose 应用", "docker compose [选项] <子命令> [参数...]", {
        options: options(
          ["--env-file", "--file", "--profile", "--project-name"],
          "docker compose 选项",
          ["--env-file", "--file", "--profile", "--project-name"],
        ),
        subcommands: simpleSubcommands([
          ["build", "构建服务", "docker compose build [选项] [服务...]"],
          ["down", "停止并删除资源", "docker compose down [选项]"],
          ["exec", "在服务中执行命令", "docker compose exec [选项] <服务> <命令...>"],
          ["logs", "查看服务日志", "docker compose logs [选项] [服务...]"],
          ["ps", "列出服务容器", "docker compose ps [选项]"],
          ["pull", "拉取服务镜像", "docker compose pull [选项] [服务...]"],
          ["restart", "重启服务", "docker compose restart [选项] [服务...]"],
          ["start", "启动已有服务", "docker compose start [服务...]"],
          ["stop", "停止服务", "docker compose stop [选项] [服务...]"],
          ["up", "创建并启动服务", "docker compose up [选项] [服务...]"],
        ]),
      }),
      schema("exec", "在容器中执行命令", "docker exec [选项] <容器> <命令...>", { options: options(["--detach", "--env", "--interactive", "--tty", "--user", "--workdir"], "docker exec 选项") }),
      schema("images", "列出镜像", "docker images [选项] [仓库[:标签]]", { options: options(["--all", "--digests", "--filter", "--format", "--quiet"], "docker images 选项") }),
      schema("inspect", "检查对象", "docker inspect [选项] <对象...>", { options: options(["--format", "--size", "--type"], "docker inspect 选项") }),
      schema("logs", "查看容器日志", "docker logs [选项] <容器>", { options: options(["--follow", "--since", "--tail", "--timestamps", "--until"], "docker logs 选项") }),
      schema("ps", "列出容器", "docker ps [选项]", { options: options(["--all", "--filter", "--format", "--latest", "--quiet", "--size"], "docker ps 选项") }),
      schema("pull", "拉取镜像", "docker pull [选项] <镜像>", { options: options(["--all-tags", "--platform", "--quiet"], "docker pull 选项") }),
      schema("push", "推送镜像", "docker push [选项] <镜像>", { options: options(["--all-tags", "--quiet"], "docker push 选项") }),
      schema("run", "创建并运行容器", "docker run [选项] <镜像> [命令...]", { options: options(["--detach", "--env", "--name", "--network", "--publish", "--rm", "--volume"], "docker run 选项") }),
    ],
  }),
  dotnetCommand(),
  schema("du", "统计文件和目录空间", "du [选项] [路径...]", {
    options: options(["--max-depth", "--summarize", "-a", "-h", "-s", "-x"], "du 选项", ["--max-depth"]),
    arguments: commonPathArguments,
  }),
  schema("echo", "输出文本", "echo [选项] [文本...]", { options: options(["-E", "-e", "-n"], "echo 选项") }),
  schema("env", "查看或设置命令环境", "env [选项] [名称=值...] [命令 [参数...]]", {
    options: options(["--chdir", "--ignore-environment", "--unset", "-C", "-i", "-u"], "env 选项", ["--chdir", "--unset", "-C", "-u"]),
  }),
  schema("find", "查找文件", "find [起始路径...] [表达式]", {
    options: options(["-maxdepth", "-mindepth", "-mtime", "-name", "-path", "-size", "-type"], "find 条件"),
    arguments: commonPathArguments,
  }),
  schema("free", "显示内存使用情况", "free [选项]", {
    options: options(["--bytes", "--giga", "--human", "--mega", "--seconds", "-b", "-g", "-h", "-m", "-s"], "free 选项", ["--seconds", "-s"]),
  }),
  schema("git", "管理 Git 仓库", "git [全局选项] <子命令> [参数...]", {
    options: options(
      ["--help", "--no-pager", "--version", "-C", "-c"],
      "Git 全局选项",
      ["-C", "-c"],
    ),
    subcommands: [
      schema("add", "暂存文件", "git add [选项] <路径...>", { options: options(["--all", "--intent-to-add", "--patch", "--update", "-A", "-p", "-u"], "git add 选项") }),
      schema("branch", "管理分支", "git branch [选项] [分支名]", { options: options(["--all", "--delete", "--move", "--remotes", "-D", "-a", "-d", "-m", "-r"], "git branch 选项") }),
      schema("checkout", "切换分支或还原文件", "git checkout [选项] <分支|路径>", { options: options(["--detach", "--force", "-B", "-b", "-f"], "git checkout 选项") }),
      schema("commit", "提交暂存变更", "git commit [选项] [路径...]", { options: options(["--amend", "--no-edit", "--signoff", "-S", "-a", "-m"], "git commit 选项") }),
      schema("diff", "比较变更", "git diff [选项] [提交] [--] [路径...]", { options: options(["--cached", "--name-only", "--stat", "--word-diff"], "git diff 选项") }),
      schema("fetch", "获取远端引用", "git fetch [选项] [远端 [引用...]]", { options: options(["--all", "--prune", "--tags", "-p"], "git fetch 选项") }),
      schema("log", "查看提交历史", "git log [选项] [修订范围] [--] [路径...]", { options: options(["--all", "--decorate", "--graph", "--oneline", "--stat", "-n"], "git log 选项") }),
      schema("pull", "获取并整合远端变更", "git pull [选项] [远端 [分支]]", { options: options(["--ff-only", "--no-rebase", "--rebase", "--tags"], "git pull 选项") }),
      schema("push", "推送本地引用", "git push [选项] [远端 [引用...]]", { options: options(["--delete", "--force-with-lease", "--set-upstream", "--tags", "-u"], "git push 选项") }),
      schema("rebase", "变基提交", "git rebase [选项] [上游 [分支]]", { options: options(["--abort", "--continue", "--interactive", "--onto", "--skip", "-i"], "git rebase 选项") }),
      schema("restore", "还原工作树文件", "git restore [选项] <路径...>", { options: options(["--source", "--staged", "--worktree", "-S", "-W"], "git restore 选项") }),
      schema("stash", "管理临时变更", "git stash <子命令> [参数...]", {
        subcommands: simpleSubcommands([
          ["apply", "应用 stash", "git stash apply [stash]"],
          ["drop", "删除 stash", "git stash drop [stash]"],
          ["list", "列出 stash", "git stash list [选项]"],
          ["pop", "应用并删除 stash", "git stash pop [stash]"],
          ["push", "保存工作区变更", "git stash push [选项] [--] [路径...]"],
          ["show", "查看 stash", "git stash show [选项] [stash]"],
        ]),
      }),
      schema("status", "查看工作树状态", "git status [选项] [--] [路径...]", { options: options(["--branch", "--porcelain", "--short", "--show-stash", "-b", "-s"], "git status 选项") }),
      schema("switch", "切换分支", "git switch [选项] <分支>", { options: options(["--create", "--detach", "--force-create", "-C", "-c"], "git switch 选项") }),
      schema("tag", "管理标签", "git tag [选项] [标签 [对象]]", { options: options(["--delete", "--list", "--sign", "-a", "-d", "-l", "-s"], "git tag 选项") }),
    ],
  }),
  goCommand(),
  schema("gradle", "运行 Gradle 构建", "gradle [选项] [任务...]", {
    options: options(["--build-cache", "--console", "--continue", "--daemon", "--debug", "--dry-run", "--info", "--no-daemon", "--offline", "--parallel", "--project-dir", "--quiet", "--refresh-dependencies", "--scan", "--stacktrace", "-b", "-p", "-q", "-x"], "Gradle 选项", ["--console", "--project-dir", "-b", "-p", "-x"]),
  }),
  schema("grep", "搜索文本", "grep [选项] <模式> [文件...]", { options: options(["--color=auto", "-E", "-F", "-i", "-n", "-r", "-v"], "grep 选项") }),
  schema("gzip", "压缩或解压 gzip 文件", "gzip [选项] [文件...]", {
    options: options(["--decompress", "--force", "--keep", "--recursive", "--stdout", "-c", "-d", "-f", "-k", "-r"], "gzip 选项"),
    arguments: commonPathArguments,
  }),
  schema("head", "输出文件开头", "head [选项] [文件...]", { options: options(["--bytes", "--lines", "-c", "-n", "-q", "-v"], "head 选项"), arguments: commonPathArguments }),
  helmCommand(),
  schema("hostname", "显示或设置主机名", "hostname [选项] [名称]", {
    options: options(["--all-fqdns", "--fqdn", "--ip-address", "--short", "-A", "-I", "-f", "-s"], "hostname 选项"),
  }),
  schema("htop", "交互式查看系统进程", "htop [选项]", {
    options: options(["--delay", "--filter", "--pid", "--sort-key", "--tree", "-d", "-p", "-s", "-t"], "htop 选项", ["--delay", "--filter", "--pid", "--sort-key", "-d", "-p", "-s"]),
  }),
  schema("id", "显示用户和组标识", "id [选项] [用户]", {
    options: options(["--group", "--groups", "--name", "--real", "--user", "-G", "-g", "-n", "-r", "-u"], "id 选项"),
  }),
  ipCommand(),
  schema("ipconfig", "查看 Windows 网络配置", "ipconfig [选项]", {
    options: options(["/all", "/displaydns", "/flushdns", "/registerdns", "/release", "/release6", "/renew", "/renew6", "/showclassid", "/showclassid6"], "ipconfig 选项"),
  }),
  schema("java", "运行 Java 应用", "java [选项] <类|jar|模块> [参数...]", {
    options: options(["--class-path", "--enable-preview", "--jar", "--module", "--module-path", "--show-version", "--version", "-cp", "-jar", "-m", "-p"], "Java 选项", ["--class-path", "--module", "--module-path", "-cp", "-jar", "-m", "-p"]),
  }),
  schema("javac", "编译 Java 源码", "javac [选项] <源文件...>", {
    options: options(["--class-path", "--enable-preview", "--module-path", "--release", "--source", "--target", "-classpath", "-cp", "-d", "-encoding", "-g"], "javac 选项", ["--class-path", "--module-path", "--release", "--source", "--target", "-classpath", "-cp", "-d", "-encoding"]),
    arguments: commonPathArguments,
  }),
  schema("journalctl", "查询 systemd 日志", "journalctl [选项] [匹配条件...]", { options: options(["--boot", "--follow", "--no-pager", "--since", "--until", "-b", "-f", "-n", "-u"], "journalctl 选项") }),
  schema("jq", "查询和转换 JSON", "jq [选项] <过滤器> [文件...]", {
    options: options(["--arg", "--argjson", "--compact-output", "--exit-status", "--join-output", "--null-input", "--raw-output", "--slurp", "-c", "-e", "-j", "-n", "-r", "-s"], "jq 选项", ["--arg", "--argjson"]),
    arguments: commonPathArguments,
  }),
  kubectlCommand(),
  schema("kill", "向进程发送信号", "kill [选项] <PID...>", { options: options(["--list", "-HUP", "-INT", "-KILL", "-TERM", "-l", "-s"], "kill 信号或选项") }),
  schema("less", "分页查看文本", "less [选项] [文件...]", {
    options: options(["--ignore-case", "--quit-if-one-screen", "--RAW-CONTROL-CHARS", "-F", "-N", "-R", "-S", "-i"], "less 选项"),
    arguments: commonPathArguments,
  }),
  schema("ln", "创建文件链接", "ln [选项] <目标...> [链接名]", {
    options: options(["--force", "--relative", "--symbolic", "-f", "-n", "-r", "-s", "-v"], "ln 选项"),
    arguments: commonPathArguments,
  }),
  schema("ls", "列出目录内容", "ls [选项] [路径...]", { options: options(["--color=auto", "-R", "-a", "-h", "-l", "-t"], "ls 选项"), arguments: commonPathArguments }),
  schema("lsof", "列出打开的文件和套接字", "lsof [选项] [名称...]", {
    options: options(["-i", "-n", "-p", "-P", "-u"], "lsof 选项", ["-i", "-p", "-u"]),
  }),
  schema("make", "运行 Make 构建目标", "make [选项] [目标...]", {
    options: options(["--directory", "--file", "--jobs", "--keep-going", "--silent", "-C", "-f", "-j", "-k", "-n", "-s"], "Make 选项", ["--directory", "--file", "--jobs", "-C", "-f", "-j"]),
  }),
  schema("man", "查看命令手册", "man [选项] [章节] <名称...>", {
    options: options(["--apropos", "--html", "--where", "-a", "-f", "-k", "-w"], "man 选项"),
  }),
  checksumCommand("md5sum", "MD5"),
  schema("mkdir", "创建目录", "mkdir [选项] <目录...>", { options: options(["--mode", "--parents", "-m", "-p", "-v"], "mkdir 选项") }),
  schema("mount", "挂载文件系统", "mount [选项] [设备] [目录]", {
    options: options(["--all", "--bind", "--options", "--read-only", "--types", "-B", "-L", "-U", "-a", "-o", "-r", "-t", "-v"], "mount 选项", ["--options", "--types", "-B", "-L", "-U", "-o", "-t"]),
    arguments: commonPathArguments,
  }),
  schema("mv", "移动或重命名文件", "mv [选项] <源...> <目标>", { options: options(["--backup", "--target-directory", "-f", "-i", "-n", "-t", "-v"], "mv 选项"), arguments: commonPathArguments }),
  schema("mvn", "运行 Maven 构建", "mvn [选项] [阶段|目标...]", {
    options: options(["--activate-profiles", "--also-make", "--batch-mode", "--define", "--file", "--offline", "--projects", "--quiet", "--settings", "--threads", "-B", "-D", "-P", "-T", "-f", "-o", "-pl", "-q", "-s"], "Maven 选项", ["--activate-profiles", "--define", "--file", "--projects", "--settings", "--threads", "-D", "-P", "-T", "-f", "-pl", "-s"]),
  }),
  schema("nano", "使用 Nano 编辑文件", "nano [选项] [文件...]", {
    options: options(["--linenumbers", "--mouse", "--nowrap", "--restricted", "-B", "-l", "-m", "-v"], "Nano 选项"),
    arguments: commonPathArguments,
  }),
  schema("nc", "建立 TCP 或 UDP 连接", "nc [选项] <主机> <端口>", {
    options: options(["-4", "-6", "-l", "-n", "-p", "-s", "-u", "-v", "-w", "-z"], "netcat 选项", ["-p", "-s", "-w"]),
  }),
  schema("netstat", "显示网络连接和路由", "netstat [选项]", {
    options: options(["--listening", "--numeric", "--program", "--route", "--tcp", "--udp", "-a", "-l", "-n", "-p", "-r", "-t", "-u"], "netstat 选项"),
  }),
  schema("node", "运行 Node.js 程序", "node [选项] [脚本 [参数...]]", {
    options: options(["--check", "--eval", "--inspect", "--print", "--require", "--test", "--version", "-c", "-e", "-p", "-r", "-v"], "Node.js 选项", ["--eval", "--inspect", "--print", "--require", "-e", "-p", "-r"]),
  }),
  schema("npx", "执行 npm 包命令", "npx [选项] <包|命令> [参数...]", {
    options: options(["--call", "--package", "--shell", "--yes", "-c", "-p", "-y"], "npx 选项", ["--call", "--package", "--shell", "-c", "-p"]),
  }),
  schema("npm", "Node.js 包管理", "npm [全局选项] <子命令> [参数...]", {
    options: options(
      ["--help", "--silent", "--version", "--workspace"],
      "npm 全局选项",
      ["--workspace"],
    ),
    subcommands: simpleSubcommands([
      ["audit", "审计依赖", "npm audit [选项]"],
      ["exec", "执行包命令", "npm exec [选项] -- <命令...>"],
      ["install", "安装依赖", "npm install [选项] [包...]"],
      ["outdated", "检查过期依赖", "npm outdated [选项]"],
      ["publish", "发布包", "npm publish [选项]"],
      ["run", "运行 package script", "npm run <脚本> [-- <参数...>]"],
      ["start", "运行 start script", "npm start [-- <参数...>]"],
      ["test", "运行 test script", "npm test [-- <参数...>]"],
      ["update", "更新依赖", "npm update [选项] [包...]"],
    ]),
  }),
  schema("pnpm", "高效 Node.js 包管理", "pnpm [全局选项] <子命令> [参数...]", {
    options: options(
      ["--dir", "--filter", "--help", "--silent", "--version"],
      "pnpm 全局选项",
      ["--dir", "--filter"],
    ),
    subcommands: simpleSubcommands([
      ["add", "添加依赖", "pnpm add [选项] <包...>"],
      ["audit", "审计依赖", "pnpm audit [选项]"],
      ["build", "运行 build script", "pnpm build [-- <参数...>]"],
      ["exec", "执行命令", "pnpm exec <命令...>"],
      ["install", "安装依赖", "pnpm install [选项]"],
      ["remove", "移除依赖", "pnpm remove [选项] <包...>"],
      ["run", "运行 package script", "pnpm run <脚本> [-- <参数...>]"],
      ["test", "运行 test script", "pnpm test [-- <参数...>]"],
      ["update", "更新依赖", "pnpm update [选项] [包...]"],
    ]),
  }),
  schema("openssl", "使用 OpenSSL 密码工具", "openssl <子命令> [选项]", {
    subcommands: simpleSubcommands([
      ["dgst", "计算消息摘要", "openssl dgst [选项] [文件...]"],
      ["rand", "生成随机数据", "openssl rand [选项] <字节数>"],
      ["req", "管理证书请求", "openssl req [选项]"],
      ["s_client", "运行 TLS 客户端", "openssl s_client [选项]"],
      ["s_server", "运行 TLS 服务端", "openssl s_server [选项]"],
      ["version", "显示 OpenSSL 版本", "openssl version [选项]"],
      ["x509", "管理 X.509 证书", "openssl x509 [选项]"],
    ]),
  }),
  schema("pacman", "管理 Arch Linux 软件包", "pacman <操作> [选项] [软件包...]", {
    options: options(["--noconfirm", "--needed", "-Q", "-R", "-S", "-Syu", "-U", "-Ss", "-Sy"], "pacman 操作或选项"),
  }),
  schema("ping", "测试网络连通性", "ping [选项] <目标>", {
    options: options(["-4", "-6", "-c", "-i", "-I", "-s", "-t", "-W"], "ping 选项", ["-c", "-i", "-I", "-s", "-t", "-W"]),
  }),
  pipCommand("pip"),
  pipCommand("pip3"),
  podmanCommand(),
  powershellCommand("powershell"),
  schema("ps", "显示进程状态", "ps [选项]", {
    options: options(["--forest", "--pid", "--sort", "-A", "-a", "-e", "-f", "-o", "-u", "-x"], "ps 选项", ["--pid", "--sort", "-o", "-u"]),
  }),
  schema("pwd", "显示当前目录", "pwd [选项]", { options: options(["-L", "-P"], "pwd 选项") }),
  powershellCommand("pwsh"),
  pythonCommand("python"),
  pythonCommand("python3"),
  schema("rg", "使用 ripgrep 搜索文本", "rg [选项] <模式> [路径...]", {
    options: options(["--files", "--glob", "--hidden", "--ignore-case", "--type", "-F", "-g", "-i", "-l", "-n", "-S", "-t", "-v"], "ripgrep 选项", ["--glob", "--type", "-g", "-t"]),
    arguments: commonPathArguments,
  }),
  schema("rm", "删除文件或目录", "rm [选项] <路径...>", { options: options(["--one-file-system", "-d", "-f", "-i", "-r", "-v"], "rm 选项"), arguments: commonPathArguments }),
  schema("rsync", "同步文件和目录", "rsync [选项] <源...> <目标>", { options: options(["--delete", "--dry-run", "--exclude", "--progress", "-a", "-e", "-n", "-v", "-z"], "rsync 选项") }),
  schema("rustc", "编译 Rust 程序", "rustc [选项] <输入文件>", {
    options: options(["--crate-name", "--crate-type", "--edition", "--emit", "--extern", "--out-dir", "-C", "-L", "-O", "-o"], "rustc 选项", ["--crate-name", "--crate-type", "--edition", "--emit", "--extern", "--out-dir", "-C", "-L", "-o"]),
    arguments: commonPathArguments,
  }),
  schema("scp", "通过 SSH 复制文件", "scp [选项] <源...> <目标>", { options: options(["-3", "-C", "-J", "-P", "-i", "-p", "-r", "-v"], "SCP 选项") }),
  schema("screen", "管理 GNU Screen 会话", "screen [选项] [命令 [参数...]]", {
    options: options(["-D", "-L", "-R", "-S", "-d", "-ls", "-r", "-x", "-X"], "Screen 选项", ["-S", "-r", "-x", "-X"]),
  }),
  schema("sed", "流式编辑文本", "sed [选项] <脚本> [文件...]", {
    options: options(["--expression", "--file", "--in-place", "--regexp-extended", "-E", "-e", "-f", "-i", "-n"], "sed 选项", ["--expression", "--file", "--in-place", "-e", "-f", "-i"]),
  }),
  schema("service", "管理 SysV 服务", "service <服务> <操作> [参数...]", {
    options: options(["--status-all", "-h"], "service 选项"),
  }),
  schema("sftp", "交互式传输 SSH 文件", "sftp [选项] <目标>", {
    options: options(["-4", "-6", "-B", "-C", "-F", "-J", "-P", "-b", "-i", "-o", "-R", "-v"], "SFTP 选项", ["-B", "-F", "-J", "-P", "-b", "-i", "-o", "-R"]),
  }),
  schema("sh", "启动 POSIX shell", "sh [选项] [脚本 [参数...]]", {
    options: options(["-c", "-e", "-n", "-s", "-u", "-x"], "Shell 选项", ["-c"]),
  }),
  checksumCommand("sha256sum", "SHA-256"),
  schema("sort", "排序文本行", "sort [选项] [文件...]", {
    options: options(["--field-separator", "--key", "--numeric-sort", "--reverse", "--unique", "-k", "-n", "-r", "-t", "-u"], "sort 选项", ["--field-separator", "--key", "-k", "-t"]),
    arguments: commonPathArguments,
  }),
  schema("ss", "显示套接字状态", "ss [选项] [过滤器]", {
    options: options(["--all", "--listening", "--numeric", "--processes", "--tcp", "--udp", "-a", "-l", "-n", "-p", "-t", "-u"], "ss 选项"),
  }),
  schema("ssh", "连接 SSH 主机", "ssh [选项] [用户@]主机 [命令...]", { options: options(["-4", "-6", "-A", "-D", "-J", "-L", "-R", "-i", "-o", "-p", "-t", "-v"], "SSH 选项") }),
  schema("ssh-add", "管理 ssh-agent 身份", "ssh-add [选项] [私钥文件...]", {
    options: options(["-D", "-K", "-L", "-T", "-c", "-d", "-l", "-t", "-x"], "ssh-add 选项", ["-T", "-t"]),
    arguments: commonPathArguments,
  }),
  schema("ssh-keygen", "生成和管理 SSH 密钥", "ssh-keygen [选项]", {
    options: options(["-C", "-E", "-F", "-N", "-R", "-b", "-f", "-l", "-p", "-q", "-t", "-y"], "ssh-keygen 选项", ["-C", "-E", "-F", "-N", "-R", "-b", "-f", "-t"]),
  }),
  schema("stat", "显示文件或文件系统状态", "stat [选项] <路径...>", {
    options: options(["--dereference", "--file-system", "--format", "--printf", "-L", "-c", "-f"], "stat 选项", ["--format", "--printf", "-c"]),
    arguments: commonPathArguments,
  }),
  schema("sudo", "以其他用户身份执行命令", "sudo [选项] <命令> [参数...]", {
    options: options(["--chdir", "--group", "--preserve-env", "--user", "-E", "-H", "-S", "-g", "-i", "-u"], "sudo 选项", ["--chdir", "--group", "--user", "-g", "-u"]),
  }),
  schema("systemctl", "管理 systemd 单元", "systemctl [选项] <操作> [单元...]", {
    options: options(["--no-pager", "--now", "--system", "--user"], "systemctl 全局选项"),
    subcommands: simpleSubcommands([
      ["daemon-reload", "重新加载单元定义", "systemctl daemon-reload"],
      ["disable", "禁用单元", "systemctl disable [选项] <单元...>"],
      ["enable", "启用单元", "systemctl enable [选项] <单元...>"],
      ["is-active", "检查活动状态", "systemctl is-active [选项] <单元...>"],
      ["list-units", "列出单元", "systemctl list-units [选项] [模式...]"],
      ["reload", "重新加载单元", "systemctl reload [选项] <单元...>"],
      ["restart", "重启单元", "systemctl restart [选项] <单元...>"],
      ["start", "启动单元", "systemctl start [选项] <单元...>"],
      ["status", "查看单元状态", "systemctl status [选项] [单元...]"],
      ["stop", "停止单元", "systemctl stop [选项] <单元...>"],
    ]),
  }),
  schema("taskkill", "终止 Windows 进程", "taskkill [选项]", {
    options: options(["/f", "/fi", "/im", "/pid", "/t"], "taskkill 选项", ["/fi", "/im", "/pid"]),
  }),
  schema("tasklist", "列出 Windows 进程", "tasklist [选项]", {
    options: options(["/fi", "/fo", "/m", "/nh", "/svc", "/v"], "tasklist 选项", ["/fi", "/fo", "/m"]),
  }),
  schema("tail", "输出文件末尾", "tail [选项] [文件...]", { options: options(["--follow", "--lines", "--pid", "--retry", "-F", "-f", "-n"], "tail 选项"), arguments: commonPathArguments }),
  schema("tar", "归档文件", "tar <操作> [选项] [文件...]", { options: options(["--directory", "--exclude", "-C", "-c", "-f", "-j", "-t", "-v", "-x", "-z"], "tar 操作或选项") }),
  schema("telnet", "连接 Telnet 服务", "telnet [选项] <主机> [端口]", {
    options: options(["-4", "-6", "-E", "-K", "-a", "-e", "-l", "-n"], "Telnet 选项", ["-e", "-l", "-n"]),
  }),
  schema("tee", "复制标准输入到文件", "tee [选项] [文件...]", {
    options: options(["--append", "--ignore-interrupts", "--output-error", "-a", "-i", "-p"], "tee 选项", ["--output-error"]),
    arguments: commonPathArguments,
  }),
  terraformCommand(),
  tmuxCommand(),
  schema("touch", "创建文件或更新时间戳", "touch [选项] <文件...>", {
    options: options(["--date", "--reference", "-a", "-c", "-d", "-m", "-r", "-t"], "touch 选项", ["--date", "--reference", "-d", "-r", "-t"]),
    arguments: commonPathArguments,
  }),
  schema("top", "查看系统进程", "top [选项]", { options: options(["-H", "-b", "-d", "-n", "-p", "-u"], "top 选项") }),
  schema("traceroute", "跟踪网络路由", "traceroute [选项] <目标> [包长度]", {
    options: options(["-4", "-6", "-I", "-m", "-n", "-p", "-q", "-T", "-w"], "traceroute 选项", ["-m", "-p", "-q", "-w"]),
  }),
  schema("tree", "以树形列出目录", "tree [选项] [路径...]", {
    options: options(["--dirsfirst", "--filelimit", "--prune", "-a", "-d", "-L", "-p", "-s"], "tree 选项", ["--filelimit", "-L"]),
    arguments: commonPathArguments,
  }),
  schema("umount", "卸载文件系统", "umount [选项] <设备|目录...>", {
    options: options(["--all", "--force", "--lazy", "--recursive", "--types", "-R", "-a", "-f", "-l", "-t", "-v"], "umount 选项", ["--types", "-t"]),
    arguments: commonPathArguments,
  }),
  schema("uname", "显示系统信息", "uname [选项]", { options: options(["--all", "--kernel-release", "--machine", "--operating-system", "-a", "-m", "-r", "-s"], "uname 选项") }),
  schema("unzip", "解压 ZIP 归档", "unzip [选项] <归档> [文件...]", {
    options: options(["-d", "-j", "-l", "-n", "-o", "-q", "-t"], "unzip 选项", ["-d"]),
    arguments: commonPathArguments,
  }),
  schema("vi", "启动 Vi 编辑器", "vi [选项] [文件...]", { options: options(["-R", "-c", "-d", "-n", "-u"], "Vi 选项"), arguments: commonPathArguments }),
  schema("vim", "启动 Vim 编辑器", "vim [选项] [文件...]", { options: options(["--clean", "-R", "-c", "-d", "-n", "-u"], "Vim 选项"), arguments: commonPathArguments }),
  schema("watch", "周期执行并显示命令", "watch [选项] <命令...>", {
    options: options(["--beep", "--color", "--differences", "--errexit", "--interval", "--no-title", "--precise", "-b", "-c", "-d", "-e", "-n", "-p", "-t"], "watch 选项", ["--differences", "--interval", "-d", "-n"]),
  }),
  schema("wc", "统计行、词和字节", "wc [选项] [文件...]", {
    options: options(["--bytes", "--chars", "--lines", "--words", "-c", "-l", "-m", "-w"], "wc 选项"),
    arguments: commonPathArguments,
  }),
  schema("wget", "下载网络资源", "wget [选项] <URL...>", { options: options(["--continue", "--directory-prefix", "--output-document", "--quiet", "-O", "-P", "-c", "-q"], "wget 选项") }),
  schema("where", "定位 Windows 可执行文件", "where [选项] <模式...>", {
    options: options(["/f", "/q", "/r", "/t"], "where 选项", ["/r"]),
  }),
  schema("which", "定位可执行命令", "which [选项] <命令...>", {
    options: options(["--all", "--read-alias", "-a", "-i"], "which 选项"),
  }),
  schema("whoami", "显示当前用户", "whoami [选项]", { options: options(["--help", "--version"], "whoami 选项") }),
  wingetCommand(),
  schema("xargs", "从标准输入构造命令", "xargs [选项] [命令 [初始参数...]]", {
    options: options(["--max-args", "--max-procs", "--null", "--replace", "-0", "-I", "-n", "-P", "-r"], "xargs 选项", ["--max-args", "--max-procs", "--replace", "-I", "-n", "-P"]),
  }),
  yarnCommand(),
  rpmPackageCommand("yum"),
  schema("zip", "创建或更新 ZIP 归档", "zip [选项] <归档> [文件...]", {
    options: options(["--recurse-paths", "-1", "-9", "-d", "-e", "-j", "-q", "-r", "-u"], "zip 选项"),
    arguments: commonPathArguments,
  }),
  schema("zsh", "启动 Z shell", "zsh [选项] [脚本 [参数...]]", {
    options: options(["--no-rcs", "-c", "-d", "-f", "-i", "-l", "-n", "-x"], "Zsh 选项", ["-c"]),
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
  return options(["--all-targets", "--features", "--package", "--release", "--target", "--workspace"], "Cargo 构建选项");
}

function ansibleCommand(command: "ansible" | "ansible-playbook"): TerminalCommandSchema {
  const common = options(
    ["--ask-become-pass", "--ask-pass", "--become", "--become-user", "--check", "--diff", "--extra-vars", "--forks", "--inventory", "--limit", "--private-key", "--tags", "--vault-id", "--version", "-C", "-D", "-K", "-b", "-e", "-f", "-i", "-k", "-l", "-t", "-u"],
    `${command} 选项`,
    ["--become-user", "--extra-vars", "--forks", "--inventory", "--limit", "--private-key", "--tags", "--vault-id", "-e", "-f", "-i", "-l", "-t", "-u"],
  );
  return command === "ansible"
    ? schema(command, "批量执行 Ansible 任务", "ansible <主机模式> [选项]", {
      options: [...common, ...options(["--args", "--background", "--module-name", "--one-line", "--poll", "-B", "-P", "-a", "-m", "-o"], "ansible ad-hoc 选项", ["--args", "--background", "--module-name", "--poll", "-B", "-P", "-a", "-m"])],
    })
    : schema(command, "执行 Ansible Playbook", "ansible-playbook [选项] <playbook...>", {
      options: [...common, ...options(["--flush-cache", "--force-handlers", "--list-hosts", "--list-tags", "--list-tasks", "--start-at-task", "--step", "--syntax-check"], "ansible-playbook 选项", ["--start-at-task"])],
      arguments: commonPathArguments,
    });
}

function bunCommand(): TerminalCommandSchema {
  return schema("bun", "运行 JavaScript 与管理软件包", "bun [全局选项] <子命令|文件> [参数...]", {
    options: options(["--bun", "--cwd", "--filter", "--silent", "--version"], "Bun 全局选项", ["--cwd", "--filter"]),
    subcommands: [
      schema("add", "添加依赖", "bun add [选项] <包...>", { options: options(["--dev", "--exact", "--global", "--optional", "--peer", "-d", "-E", "-g", "-o", "-p"], "bun add 选项") }),
      schema("build", "打包源码", "bun build [选项] <入口...>", { options: options(["--compile", "--format", "--minify", "--outdir", "--outfile", "--sourcemap", "--target"], "bun build 选项", ["--format", "--outdir", "--outfile", "--sourcemap", "--target"]) }),
      schema("create", "从模板创建项目", "bun create [选项] <模板> [目录]"),
      schema("install", "安装依赖", "bun install [选项]", { options: options(["--frozen-lockfile", "--ignore-scripts", "--no-save", "--production"], "bun install 选项") }),
      schema("remove", "移除依赖", "bun remove [选项] <包...>"),
      schema("run", "运行脚本或文件", "bun run [选项] <脚本|文件> [参数...]"),
      schema("test", "运行测试", "bun test [选项] [过滤器...]", { options: options(["--bail", "--coverage", "--only", "--preload", "--rerun-each", "--timeout", "--watch"], "bun test 选项", ["--bail", "--preload", "--rerun-each", "--timeout"]) }),
      schema("update", "更新依赖", "bun update [选项] [包...]", { options: options(["--latest"], "bun update 选项") }),
      schema("x", "执行软件包二进制", "bun x [选项] <包> [参数...]"),
    ],
  });
}

function checksumCommand(command: "md5sum" | "sha256sum", algorithm: string): TerminalCommandSchema {
  return schema(command, `计算或校验 ${algorithm}`, `${command} [选项] [文件...]`, {
    options: options(["--binary", "--check", "--ignore-missing", "--quiet", "--status", "--strict", "--tag", "--text", "--warn", "-b", "-c", "-t", "-w"], `${command} 选项`),
    arguments: commonPathArguments,
  });
}

function denoCommand(): TerminalCommandSchema {
  return schema("deno", "运行 JavaScript 与 TypeScript", "deno [全局选项] <子命令> [参数...]", {
    options: options(["--config", "--no-config", "--quiet", "--version", "-c", "-q"], "Deno 全局选项", ["--config", "-c"]),
    subcommands: [
      schema("cache", "缓存依赖", "deno cache [选项] <文件...>"),
      schema("check", "类型检查", "deno check [选项] <文件...>"),
      schema("compile", "编译独立可执行文件", "deno compile [选项] <脚本> [参数...]", { options: options(["--allow-all", "--output", "--target", "-A", "-o"], "deno compile 选项", ["--output", "--target", "-o"]) }),
      schema("fmt", "格式化源码", "deno fmt [选项] [文件...]", { options: options(["--check", "--ignore", "--line-width"], "deno fmt 选项", ["--ignore", "--line-width"]) }),
      schema("info", "显示依赖与缓存信息", "deno info [选项] [文件]"),
      schema("install", "安装脚本命令", "deno install [选项] <脚本> [参数...]", { options: options(["--allow-all", "--global", "--name", "--root", "-A", "-g", "-n"], "deno install 选项", ["--name", "--root", "-n"]) }),
      schema("lint", "检查源码", "deno lint [选项] [文件...]", { options: options(["--compact", "--ignore", "--json", "--rules"], "deno lint 选项", ["--ignore", "--rules"]) }),
      schema("repl", "启动交互解释器", "deno repl [选项]"),
      schema("run", "运行脚本", "deno run [选项] <脚本> [参数...]", { options: options(["--allow-all", "--allow-env", "--allow-net", "--allow-read", "--allow-run", "--allow-write", "--watch", "-A"], "deno run 权限选项") }),
      schema("task", "运行配置任务", "deno task [选项] [任务] [参数...]"),
      schema("test", "运行测试", "deno test [选项] [文件...]", { options: options(["--allow-all", "--coverage", "--fail-fast", "--filter", "--parallel", "--watch", "-A"], "deno test 选项", ["--coverage", "--fail-fast", "--filter"]) }),
    ],
  });
}

function dotnetCommand(): TerminalCommandSchema {
  return schema("dotnet", "构建和运行 .NET 项目", "dotnet [全局选项] <命令> [参数...]", {
    options: options(["--diagnostics", "--info", "--list-runtimes", "--list-sdks", "--roll-forward", "--version"], ".NET 全局选项", ["--roll-forward"]),
    subcommands: [
      schema("add", "添加项目引用或包", "dotnet add <项目> <package|reference> [参数...]", { subcommands: simpleSubcommands([
        ["package", "添加 NuGet 包", "dotnet add package <包> [选项]"],
        ["reference", "添加项目引用", "dotnet add reference <项目...> [选项]"],
      ]) }),
      schema("build", "构建项目", "dotnet build [项目] [选项]", { options: dotnetBuildOptions("dotnet build 选项") }),
      schema("clean", "清理构建输出", "dotnet clean [项目] [选项]", { options: dotnetBuildOptions("dotnet clean 选项") }),
      schema("new", "创建项目或文件", "dotnet new <模板> [选项]", { options: options(["--dry-run", "--force", "--language", "--name", "--output", "--type", "-lang", "-n", "-o"], "dotnet new 选项", ["--language", "--name", "--output", "--type", "-lang", "-n", "-o"]) }),
      schema("pack", "创建 NuGet 包", "dotnet pack [项目] [选项]", { options: dotnetBuildOptions("dotnet pack 选项") }),
      schema("publish", "发布应用", "dotnet publish [项目] [选项]", { options: dotnetBuildOptions("dotnet publish 选项") }),
      schema("restore", "还原依赖", "dotnet restore [项目] [选项]", { options: options(["--force", "--locked-mode", "--no-cache", "--packages", "--runtime", "--source", "-r", "-s"], "dotnet restore 选项", ["--packages", "--runtime", "--source", "-r", "-s"]) }),
      schema("run", "运行项目", "dotnet run [选项] [-- <参数...>]", { options: options(["--configuration", "--framework", "--launch-profile", "--no-build", "--project", "-c", "-f", "-p"], "dotnet run 选项", ["--configuration", "--framework", "--launch-profile", "--project", "-c", "-f", "-p"]) }),
      schema("test", "运行测试", "dotnet test [项目] [选项]", { options: [...dotnetBuildOptions("dotnet test 选项"), ...options(["--filter", "--logger", "--settings"], "dotnet test 选项", ["--filter", "--logger", "--settings"])] }),
      schema("tool", "管理 .NET 工具", "dotnet tool <命令> [参数...]", { subcommands: simpleSubcommands([
        ["install", "安装工具", "dotnet tool install <包> [选项]"],
        ["list", "列出工具", "dotnet tool list [选项]"],
        ["restore", "还原本地工具", "dotnet tool restore [选项]"],
        ["uninstall", "卸载工具", "dotnet tool uninstall <包> [选项]"],
        ["update", "更新工具", "dotnet tool update <包> [选项]"],
      ]) }),
      schema("workload", "管理可选工作负载", "dotnet workload <命令> [参数...]", { subcommands: simpleSubcommands([
        ["install", "安装工作负载", "dotnet workload install <工作负载...> [选项]"],
        ["list", "列出工作负载", "dotnet workload list [选项]"],
        ["repair", "修复工作负载", "dotnet workload repair [选项]"],
        ["uninstall", "卸载工作负载", "dotnet workload uninstall <工作负载...> [选项]"],
        ["update", "更新工作负载", "dotnet workload update [选项]"],
      ]) }),
    ],
  });
}

function dotnetBuildOptions(detail: string): TerminalCommandCatalogEntry[] {
  return options(["--configuration", "--framework", "--no-restore", "--output", "--runtime", "--self-contained", "--verbosity", "-c", "-f", "-o", "-r", "-v"], detail, ["--configuration", "--framework", "--output", "--runtime", "--verbosity", "-c", "-f", "-o", "-r", "-v"]);
}

function apkCommand(): TerminalCommandSchema {
  return schema("apk", "管理 Alpine Linux 软件包", "apk [全局选项] <子命令> [参数...]", {
    options: options(["--no-cache", "--repository", "--root", "--update-cache", "-p", "-X"], "apk 全局选项", ["--repository", "--root", "-p", "-X"]),
    subcommands: [
      schema("add", "安装软件包", "apk add [选项] <软件包...>", { options: options(["--no-cache", "--repository", "--upgrade", "--virtual", "-X"], "apk add 选项", ["--repository", "--virtual", "-X"]) }),
      schema("del", "删除软件包", "apk del [选项] <软件包...>", { options: options(["--purge", "--rdepends"], "apk del 选项") }),
      schema("info", "显示软件包信息", "apk info [选项] [软件包...]", { options: options(["--all", "--contents", "--depends", "--installed", "-a", "-e", "-L", "-R"], "apk info 选项") }),
      schema("search", "搜索软件包", "apk search [选项] <模式...>", { options: options(["--description", "--exact", "--origin", "-d", "-e", "-o"], "apk search 选项") }),
      schema("update", "更新软件包索引", "apk update [选项]", { options: options(["--no-cache"], "apk update 选项") }),
      schema("upgrade", "升级已安装软件包", "apk upgrade [选项]", { options: options(["--available", "--no-cache", "--prune"], "apk upgrade 选项") }),
    ],
  });
}

function aptCommand(command: "apt" | "apt-get"): TerminalCommandSchema {
  const detail = command === "apt" ? "管理 Debian 软件包" : "使用 APT 后端管理 Debian 软件包";
  return schema(command, detail, `${command} [全局选项] <子命令> [参数...]`, {
    options: options(["--option", "--quiet", "--simulate", "--yes", "-o", "-q", "-s", "-y"], `${command} 全局选项`, ["--option", "-o"]),
    subcommands: [
      schema("autoremove", "移除无用依赖", `${command} autoremove [选项] [软件包...]`, { options: options(["--purge", "--simulate", "--yes", "-s", "-y"], `${command} autoremove 选项`) }),
      schema("clean", "清理下载缓存", `${command} clean [选项]`, { options: options(["--simulate", "-s"], `${command} clean 选项`) }),
      schema("download", "下载二进制软件包", `${command} download [选项] <软件包...>`, { options: options(["--download-only", "--reinstall"], `${command} download 选项`) }),
      schema("install", "安装软件包", `${command} install [选项] <软件包...>`, { options: options(["--download-only", "--no-install-recommends", "--reinstall", "--simulate", "--yes", "-s", "-y"], `${command} install 选项`) }),
      schema("remove", "删除软件包", `${command} remove [选项] <软件包...>`, { options: options(["--purge", "--simulate", "--yes", "-s", "-y"], `${command} remove 选项`) }),
      schema("update", "更新软件包索引", `${command} update [选项]`, { options: options(["--allow-insecure-repositories", "--quiet", "-q"], `${command} update 选项`) }),
      schema("upgrade", "升级已安装软件包", `${command} upgrade [选项]`, { options: options(["--download-only", "--simulate", "--with-new-pkgs", "--yes", "-s", "-y"], `${command} upgrade 选项`) }),
      ...(command === "apt" ? [
        schema("list", "列出软件包", "apt list [选项] [模式...]", { options: options(["--all-versions", "--installed", "--upgradable"], "apt list 选项") }),
        schema("search", "搜索软件包", "apt search [选项] <模式...>", { options: options(["--full"], "apt search 选项") }),
        schema("show", "显示软件包详情", "apt show [选项] <软件包...>", { options: options(["--all-versions"], "apt show 选项") }),
      ] : [
        schema("dist-upgrade", "升级并处理依赖变化", "apt-get dist-upgrade [选项]", { options: options(["--download-only", "--simulate", "--yes", "-s", "-y"], "apt-get dist-upgrade 选项") }),
      ]),
    ],
  });
}

function rpmPackageCommand(command: "dnf" | "yum"): TerminalCommandSchema {
  return schema(command, `管理 RPM 软件包（${command}）`, `${command} [全局选项] <子命令> [参数...]`, {
    options: options(["--assumeno", "--assumeyes", "--disablerepo", "--enablerepo", "--releasever", "-y"], `${command} 全局选项`, ["--disablerepo", "--enablerepo", "--releasever"]),
    subcommands: [
      schema("clean", "清理缓存", `${command} clean [选项] <类型...>`, { arguments: entries(["all", "expire-cache", "metadata", "packages"], `${command} 缓存类型`) }),
      schema("info", "显示软件包信息", `${command} info [选项] [软件包...]`, { options: options(["--available", "--installed"], `${command} info 选项`) }),
      schema("install", "安装软件包", `${command} install [选项] <软件包...>`, { options: options(["--allowerasing", "--downloadonly", "--nogpgcheck", "-y"], `${command} install 选项`) }),
      schema("list", "列出软件包", `${command} list [选项] [软件包...]`, { options: options(["--available", "--installed", "--updates"], `${command} list 选项`) }),
      schema("remove", "删除软件包", `${command} remove [选项] <软件包...>`, { options: options(["--noautoremove", "-y"], `${command} remove 选项`) }),
      schema("repolist", "列出软件仓库", `${command} repolist [选项]`, { options: options(["--all", "--disabled", "--enabled"], `${command} repolist 选项`) }),
      schema("search", "搜索软件包", `${command} search [选项] <模式...>`, { options: options(["--all"], `${command} search 选项`) }),
      schema("upgrade", "升级软件包", `${command} upgrade [选项] [软件包...]`, { options: options(["--allowerasing", "--refresh", "-y"], `${command} upgrade 选项`) }),
    ],
  });
}

function goCommand(): TerminalCommandSchema {
  return schema("go", "构建和管理 Go 项目", "go <子命令> [参数...]", {
    subcommands: [
      schema("build", "编译包和依赖", "go build [选项] [包...]", { options: options(["-a", "-buildvcs", "-mod", "-o", "-race", "-tags", "-v"], "go build 选项", ["-buildvcs", "-mod", "-o", "-tags"]) }),
      schema("clean", "删除构建缓存", "go clean [选项] [包...]", { options: options(["-cache", "-modcache", "-testcache"], "go clean 选项") }),
      schema("env", "显示或设置 Go 环境", "go env [选项] [变量...]", { options: options(["-json", "-u", "-w"], "go env 选项") }),
      schema("fmt", "格式化 Go 包", "go fmt [选项] [包...]", { options: options(["-n", "-x"], "go fmt 选项") }),
      schema("get", "解析并添加依赖", "go get [选项] <包...>", { options: options(["-d", "-t", "-u"], "go get 选项") }),
      schema("install", "编译并安装包", "go install [选项] <包...>", { options: options(["-race", "-tags", "-v"], "go install 选项", ["-tags"]) }),
      schema("mod", "管理 Go 模块", "go mod <子命令> [参数...]", {
        subcommands: simpleSubcommands([
          ["download", "下载模块", "go mod download [选项] [模块...]"],
          ["edit", "编辑 go.mod", "go mod edit [选项]"],
          ["graph", "打印模块依赖图", "go mod graph"],
          ["init", "创建 go.mod", "go mod init [模块路径]"],
          ["tidy", "整理模块依赖", "go mod tidy [选项]"],
          ["vendor", "生成 vendor 目录", "go mod vendor [选项]"],
          ["verify", "验证模块内容", "go mod verify"],
        ]),
      }),
      schema("run", "编译并运行 Go 程序", "go run [选项] <包|文件...> [参数...]", { options: options(["-exec", "-mod", "-race", "-tags"], "go run 选项", ["-exec", "-mod", "-tags"]) }),
      schema("test", "测试 Go 包", "go test [选项] [包...]", { options: options(["-bench", "-count", "-cover", "-race", "-run", "-short", "-v"], "go test 选项", ["-bench", "-count", "-run"]) }),
      schema("version", "显示 Go 版本", "go version [选项]", { options: options(["-m", "-v"], "go version 选项") }),
      schema("vet", "报告可疑代码", "go vet [选项] [包...]", { options: options(["-json", "-v"], "go vet 选项") }),
    ],
  });
}

function helmCommand(): TerminalCommandSchema {
  const kubeOptions = ["--kube-context", "--kubeconfig", "--namespace", "-n"];
  return schema("helm", "管理 Kubernetes Helm Chart", "helm [全局选项] <命令> [参数...]", {
    options: options(kubeOptions, "Helm 全局选项", kubeOptions),
    subcommands: [
      schema("dependency", "管理 Chart 依赖", "helm dependency <命令> [参数...]", { subcommands: simpleSubcommands([
        ["build", "重建 Chart 依赖", "helm dependency build [Chart] [选项]"],
        ["list", "列出 Chart 依赖", "helm dependency list [Chart] [选项]"],
        ["update", "更新 Chart 依赖", "helm dependency update [Chart] [选项]"],
      ]) }),
      schema("get", "读取 Release 信息", "helm get <命令> <Release> [选项]", { subcommands: simpleSubcommands([
        ["all", "读取全部 Release 信息", "helm get all <Release> [选项]"],
        ["hooks", "读取 Release Hook", "helm get hooks <Release> [选项]"],
        ["manifest", "读取 Release Manifest", "helm get manifest <Release> [选项]"],
        ["notes", "读取 Release Notes", "helm get notes <Release> [选项]"],
        ["values", "读取 Release Values", "helm get values <Release> [选项]"],
      ]) }),
      schema("history", "查看 Release 历史", "helm history <Release> [选项]"),
      schema("install", "安装 Chart", "helm install <Release> <Chart> [选项]", { options: helmReleaseOptions("helm install 选项") }),
      schema("list", "列出 Release", "helm list [选项]", { options: options(["--all", "--all-namespaces", "--date", "--filter", "--output", "--pending", "-A", "-a", "-f", "-o"], "helm list 选项", ["--filter", "--output", "-f", "-o"]) }),
      schema("repo", "管理 Chart 仓库", "helm repo <命令> [参数...]", { subcommands: simpleSubcommands([
        ["add", "添加 Chart 仓库", "helm repo add <名称> <URL> [选项]"],
        ["index", "生成仓库索引", "helm repo index <目录> [选项]"],
        ["list", "列出 Chart 仓库", "helm repo list [选项]"],
        ["remove", "删除 Chart 仓库", "helm repo remove <名称...>"],
        ["update", "更新 Chart 仓库", "helm repo update [名称...] [选项]"],
      ]) }),
      schema("rollback", "回滚 Release", "helm rollback <Release> [版本] [选项]", { options: options(["--cleanup-on-fail", "--dry-run", "--force", "--recreate-pods", "--timeout", "--wait"], "helm rollback 选项", ["--timeout"]) }),
      schema("search", "搜索 Chart", "helm search <hub|repo> <关键词> [选项]", { arguments: entries(["hub", "repo"], "Helm 搜索来源") }),
      schema("status", "查看 Release 状态", "helm status <Release> [选项]"),
      schema("template", "本地渲染 Chart", "helm template [Release] <Chart> [选项]", { options: helmReleaseOptions("helm template 选项") }),
      schema("test", "运行 Release 测试", "helm test <Release> [选项]"),
      schema("uninstall", "卸载 Release", "helm uninstall <Release...> [选项]", { options: options(["--dry-run", "--keep-history", "--no-hooks", "--timeout", "--wait"], "helm uninstall 选项", ["--timeout"]) }),
      schema("upgrade", "升级 Release", "helm upgrade <Release> <Chart> [选项]", { options: helmReleaseOptions("helm upgrade 选项") }),
    ],
  });
}

function helmReleaseOptions(detail: string): TerminalCommandCatalogEntry[] {
  return options(["--atomic", "--create-namespace", "--dependency-update", "--dry-run", "--set", "--set-file", "--set-string", "--timeout", "--values", "--version", "--wait", "-f"], detail, ["--set", "--set-file", "--set-string", "--timeout", "--values", "--version", "-f"]);
}

function ipCommand(): TerminalCommandSchema {
  return schema("ip", "查看和配置 Linux 网络", "ip [全局选项] <对象> <命令> [参数...]", {
    options: options(["-4", "-6", "-brief", "-details", "-json", "-oneline", "-stats"], "ip 全局选项"),
    subcommands: [
      schema("address", "管理网络地址", "ip address <命令> [参数...]", { subcommands: ipObjectSubcommands("address") }),
      schema("addr", "管理网络地址", "ip addr <命令> [参数...]", { subcommands: ipObjectSubcommands("addr") }),
      schema("link", "管理网络接口", "ip link <命令> [参数...]", { subcommands: ipObjectSubcommands("link") }),
      schema("neighbor", "管理邻居表", "ip neighbor <命令> [参数...]", { subcommands: ipObjectSubcommands("neighbor") }),
      schema("neigh", "管理邻居表", "ip neigh <命令> [参数...]", { subcommands: ipObjectSubcommands("neigh") }),
      schema("route", "管理路由表", "ip route <命令> [参数...]", { subcommands: ipObjectSubcommands("route") }),
      schema("rule", "管理路由策略", "ip rule <命令> [参数...]", { subcommands: ipObjectSubcommands("rule") }),
    ],
  });
}

function ipObjectSubcommands(object: string): TerminalCommandSchema[] {
  return simpleSubcommands([
    ["add", `添加 ${object} 项`, `ip ${object} add [参数...]`],
    ["delete", `删除 ${object} 项`, `ip ${object} delete [参数...]`],
    ["flush", `清空 ${object} 项`, `ip ${object} flush [参数...]`],
    ["get", `查询 ${object} 项`, `ip ${object} get [参数...]`],
    ["replace", `替换 ${object} 项`, `ip ${object} replace [参数...]`],
    ["show", `显示 ${object} 项`, `ip ${object} show [参数...]`],
  ]);
}

function kubectlCommand(): TerminalCommandSchema {
  const namespaceOptions = ["--all-namespaces", "--namespace", "-A", "-n"];
  return schema("kubectl", "管理 Kubernetes 集群", "kubectl [全局选项] <子命令> [参数...]", {
    options: options(["--context", "--kubeconfig", "--namespace", "-n"], "kubectl 全局选项", ["--context", "--kubeconfig", "--namespace", "-n"]),
    subcommands: [
      schema("apply", "应用资源配置", "kubectl apply [选项] -f <文件|URL>", { options: options(["--filename", "--namespace", "--prune", "--server-side", "-f", "-n"], "kubectl apply 选项", ["--filename", "--namespace", "-f", "-n"]) }),
      schema("config", "管理 kubeconfig", "kubectl config <子命令> [参数...]", { subcommands: simpleSubcommands([
        ["current-context", "显示当前上下文", "kubectl config current-context"],
        ["get-contexts", "列出上下文", "kubectl config get-contexts [名称...]"],
        ["set-context", "设置上下文属性", "kubectl config set-context <名称> [选项]"],
        ["use-context", "切换当前上下文", "kubectl config use-context <名称>"],
      ]) }),
      schema("create", "创建资源", "kubectl create [选项] -f <文件|URL>", { options: options(["--filename", "--namespace", "--save-config", "-f", "-n"], "kubectl create 选项", ["--filename", "--namespace", "-f", "-n"]) }),
      schema("delete", "删除资源", "kubectl delete [选项] <类型> <名称...>", { options: options(["--all", "--filename", "--force", "--namespace", "--wait", "-f", "-n"], "kubectl delete 选项", ["--filename", "--namespace", "-f", "-n"]) }),
      schema("describe", "显示资源详情", "kubectl describe [选项] <类型> [名称]", { options: options(namespaceOptions, "kubectl describe 选项", ["--namespace", "-n"]) }),
      schema("exec", "在容器中执行命令", "kubectl exec [选项] <Pod> -- <命令...>", { options: options(["--container", "--namespace", "--stdin", "--tty", "-c", "-i", "-n", "-t"], "kubectl exec 选项", ["--container", "--namespace", "-c", "-n"]) }),
      schema("get", "列出 Kubernetes 资源", "kubectl get [选项] <类型> [名称]", { options: options([...namespaceOptions, "--output", "--selector", "--show-labels", "-o", "-l"], "kubectl get 选项", ["--namespace", "--output", "--selector", "-n", "-o", "-l"]) }),
      schema("logs", "查看容器日志", "kubectl logs [选项] <Pod> [容器]", { options: options(["--container", "--follow", "--namespace", "--previous", "--since", "--tail", "-c", "-f", "-n"], "kubectl logs 选项", ["--container", "--namespace", "--since", "--tail", "-c", "-n"]) }),
      schema("rollout", "管理工作负载发布", "kubectl rollout <子命令> [参数...]", { subcommands: simpleSubcommands([
        ["history", "查看发布历史", "kubectl rollout history <资源> [选项]"],
        ["restart", "重启工作负载", "kubectl rollout restart <资源> [选项]"],
        ["status", "查看发布状态", "kubectl rollout status <资源> [选项]"],
        ["undo", "回滚发布", "kubectl rollout undo <资源> [选项]"],
      ]) }),
      schema("scale", "调整副本数量", "kubectl scale [选项] <资源>", { options: options(["--current-replicas", "--replicas", "--timeout"], "kubectl scale 选项", ["--current-replicas", "--replicas", "--timeout"]) }),
      schema("top", "显示资源使用量", "kubectl top <pod|node> [名称] [选项]", { arguments: entries(["node", "pod"], "kubectl top 资源类型") }),
    ],
  });
}

function pipCommand(command: "pip" | "pip3"): TerminalCommandSchema {
  return schema(command, "管理 Python 软件包", `${command} [全局选项] <子命令> [参数...]`, {
    options: options(["--isolated", "--no-input", "--proxy", "--python", "--require-virtualenv", "--timeout", "--version"], `${command} 全局选项`, ["--proxy", "--python", "--timeout"]),
    subcommands: [
      schema("check", "检查依赖兼容性", `${command} check`, {}),
      schema("download", "下载软件包", `${command} download [选项] <包...>`, { options: pipIndexOptions(`${command} download 选项`) }),
      schema("freeze", "输出已安装软件包", `${command} freeze [选项]`, { options: options(["--all", "--exclude", "--local", "--user"], `${command} freeze 选项`, ["--exclude"]) }),
      schema("install", "安装 Python 软件包", `${command} install [选项] <包...>`, { options: [
        ...pipIndexOptions(`${command} install 选项`),
        ...options(["--break-system-packages", "--editable", "--no-cache-dir", "--requirement", "--upgrade", "-e", "-r", "-U"], `${command} install 选项`, ["--editable", "--requirement", "-e", "-r"]),
      ] }),
      schema("list", "列出已安装软件包", `${command} list [选项]`, { options: options(["--editable", "--format", "--not-required", "--outdated", "--uptodate"], `${command} list 选项`, ["--format"]) }),
      schema("show", "显示软件包详情", `${command} show [选项] <包...>`, { options: options(["--files", "--verbose", "-f", "-v"], `${command} show 选项`) }),
      schema("uninstall", "卸载 Python 软件包", `${command} uninstall [选项] <包...>`, { options: options(["--requirement", "--yes", "-r", "-y"], `${command} uninstall 选项`, ["--requirement", "-r"]) }),
      schema("wheel", "构建 Wheel", `${command} wheel [选项] <包...>`, { options: pipIndexOptions(`${command} wheel 选项`) }),
    ],
  });
}

function pipIndexOptions(detail: string): TerminalCommandCatalogEntry[] {
  return options(["--extra-index-url", "--find-links", "--index-url", "--no-index", "-f", "-i"], detail, ["--extra-index-url", "--find-links", "--index-url", "-f", "-i"]);
}

function powershellCommand(command: "powershell" | "pwsh"): TerminalCommandSchema {
  return schema(command, "运行 PowerShell", `${command} [选项] [-Command 命令 | -File 脚本] [参数...]`, {
    options: options(
      ["-Command", "-CommandWithArgs", "-ConfigurationName", "-EncodedCommand", "-ExecutionPolicy", "-File", "-InputFormat", "-Interactive", "-Login", "-NoExit", "-NoLogo", "-NonInteractive", "-NoProfile", "-OutputFormat", "-SettingsFile", "-Version", "-WindowStyle", "-WorkingDirectory"],
      "PowerShell 选项",
      ["-Command", "-CommandWithArgs", "-ConfigurationName", "-EncodedCommand", "-ExecutionPolicy", "-File", "-InputFormat", "-OutputFormat", "-SettingsFile", "-Version", "-WindowStyle", "-WorkingDirectory"],
    ),
    arguments: commonPathArguments,
  });
}

function podmanCommand(): TerminalCommandSchema {
  return schema("podman", "管理 OCI 容器和镜像", "podman [全局选项] <子命令> [参数...]", {
    options: options(["--connection", "--log-level", "--remote", "--root", "--runtime"], "Podman 全局选项", ["--connection", "--log-level", "--root", "--runtime"]),
    subcommands: [
      schema("build", "构建容器镜像", "podman build [选项] <上下文>", { options: options(["--build-arg", "--file", "--no-cache", "--pull", "--tag"], "podman build 选项", ["--build-arg", "--file", "--pull", "--tag"]) }),
      schema("exec", "在容器中执行命令", "podman exec [选项] <容器> <命令...>", { options: options(["--detach", "--env", "--interactive", "--tty", "--user", "--workdir"], "podman exec 选项", ["--env", "--user", "--workdir"]) }),
      schema("images", "列出镜像", "podman images [选项] [镜像]", { options: options(["--all", "--filter", "--format", "--quiet"], "podman images 选项", ["--filter", "--format"]) }),
      schema("inspect", "检查对象", "podman inspect [选项] <对象...>", { options: options(["--format", "--size", "--type"], "podman inspect 选项", ["--format", "--type"]) }),
      schema("logs", "查看容器日志", "podman logs [选项] <容器>", { options: options(["--follow", "--since", "--tail", "--timestamps", "--until"], "podman logs 选项", ["--since", "--tail", "--until"]) }),
      schema("ps", "列出容器", "podman ps [选项]", { options: options(["--all", "--filter", "--format", "--latest", "--quiet", "--size"], "podman ps 选项", ["--filter", "--format"]) }),
      schema("pull", "拉取镜像", "podman pull [选项] <镜像>", { options: options(["--all-tags", "--arch", "--os", "--quiet"], "podman pull 选项", ["--arch", "--os"]) }),
      schema("push", "推送镜像", "podman push [选项] <镜像> [目标]", { options: options(["--compression-format", "--digestfile", "--quiet"], "podman push 选项", ["--compression-format", "--digestfile"]) }),
      schema("run", "创建并运行容器", "podman run [选项] <镜像> [命令...]", { options: options(["--detach", "--env", "--name", "--network", "--publish", "--rm", "--volume"], "podman run 选项", ["--env", "--name", "--network", "--publish", "--volume"]) }),
    ],
  });
}

function pythonCommand(command: "python" | "python3"): TerminalCommandSchema {
  return schema(command, "运行 Python 解释器", `${command} [选项] [-c 命令 | -m 模块 | 脚本] [参数...]`, {
    options: options(["--help", "--version", "-B", "-c", "-E", "-I", "-m", "-O", "-q", "-u", "-V", "-W", "-X"], "Python 选项", ["-c", "-m", "-W", "-X"]),
    arguments: commonPathArguments,
  });
}

function terraformCommand(): TerminalCommandSchema {
  return schema("terraform", "管理基础设施配置", "terraform [全局选项] <命令> [参数...]", {
    options: options(["-chdir", "-help", "-version"], "Terraform 全局选项", ["-chdir"]),
    subcommands: [
      schema("apply", "应用执行计划", "terraform apply [选项] [计划文件]", { options: terraformPlanOptions("terraform apply 选项") }),
      schema("destroy", "销毁受管基础设施", "terraform destroy [选项]", { options: terraformPlanOptions("terraform destroy 选项") }),
      schema("fmt", "格式化配置", "terraform fmt [选项] [目标...]", { options: options(["-check", "-diff", "-list", "-recursive", "-write"], "terraform fmt 选项") }),
      schema("force-unlock", "解除状态锁", "terraform force-unlock [选项] <锁 ID>"),
      schema("get", "安装或更新模块", "terraform get [选项]", { options: options(["-update"], "terraform get 选项") }),
      schema("graph", "生成依赖关系图", "terraform graph [选项]", { options: options(["-draw-cycles", "-plan", "-type"], "terraform graph 选项", ["-type"]) }),
      schema("import", "导入现有资源", "terraform import [选项] <地址> <ID>", { options: terraformVariableOptions("terraform import 选项") }),
      schema("init", "初始化工作目录", "terraform init [选项]", { options: options(["-backend", "-backend-config", "-force-copy", "-from-module", "-get", "-lockfile", "-migrate-state", "-plugin-dir", "-reconfigure", "-upgrade"], "terraform init 选项", ["-backend-config", "-from-module", "-lockfile", "-plugin-dir"]) }),
      schema("output", "读取输出值", "terraform output [选项] [名称]", { options: options(["-json", "-raw", "-state"], "terraform output 选项", ["-state"]) }),
      schema("plan", "创建执行计划", "terraform plan [选项]", { options: terraformPlanOptions("terraform plan 选项") }),
      schema("providers", "显示 Provider 需求", "terraform providers [选项]"),
      schema("refresh", "刷新状态", "terraform refresh [选项]", { options: terraformVariableOptions("terraform refresh 选项") }),
      schema("show", "显示状态或计划", "terraform show [选项] [文件]", { options: options(["-json", "-no-color"], "terraform show 选项") }),
      schema("state", "管理 Terraform 状态", "terraform state <命令> [参数...]", { subcommands: simpleSubcommands([
        ["list", "列出状态资源", "terraform state list [选项] [地址...]"],
        ["mv", "移动状态地址", "terraform state mv [选项] <源> <目标>"],
        ["pull", "下载远端状态", "terraform state pull"],
        ["push", "上传本地状态", "terraform state push [选项] <文件>"],
        ["replace-provider", "替换 Provider 地址", "terraform state replace-provider [选项] <源> <目标>"],
        ["rm", "从状态移除资源", "terraform state rm [选项] <地址...>"],
        ["show", "显示状态资源", "terraform state show [选项] <地址>"],
      ]) }),
      schema("test", "执行 Terraform 测试", "terraform test [选项]", { options: options(["-filter", "-json", "-test-directory", "-verbose"], "terraform test 选项", ["-filter", "-test-directory"]) }),
      schema("validate", "校验配置", "terraform validate [选项]", { options: options(["-json", "-no-color"], "terraform validate 选项") }),
      schema("version", "显示 Terraform 版本", "terraform version [选项]", { options: options(["-json"], "terraform version 选项") }),
      schema("workspace", "管理工作区", "terraform workspace <命令> [参数...]", { subcommands: simpleSubcommands([
        ["delete", "删除工作区", "terraform workspace delete [选项] <名称>"],
        ["list", "列出工作区", "terraform workspace list"],
        ["new", "创建工作区", "terraform workspace new [选项] <名称>"],
        ["select", "选择工作区", "terraform workspace select [选项] <名称>"],
        ["show", "显示当前工作区", "terraform workspace show"],
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
  return schema("tmux", "管理 Tmux 终端会话", "tmux [全局选项] <命令> [参数...]", {
    options: options(["-2", "-C", "-D", "-L", "-S", "-f", "-l", "-u", "-v"], "Tmux 全局选项", ["-L", "-S", "-f", "-l"]),
    subcommands: [
      schema("attach-session", "连接会话", "tmux attach-session [选项]", { options: tmuxTargetOptions("tmux attach-session 选项") }),
      schema("has-session", "检查会话是否存在", "tmux has-session [选项]", { options: tmuxTargetOptions("tmux has-session 选项") }),
      schema("kill-pane", "关闭 pane", "tmux kill-pane [选项]", { options: tmuxTargetOptions("tmux kill-pane 选项") }),
      schema("kill-server", "关闭 Tmux 服务端", "tmux kill-server"),
      schema("kill-session", "关闭会话", "tmux kill-session [选项]", { options: tmuxTargetOptions("tmux kill-session 选项") }),
      schema("kill-window", "关闭窗口", "tmux kill-window [选项]", { options: tmuxTargetOptions("tmux kill-window 选项") }),
      schema("list-clients", "列出客户端", "tmux list-clients [选项]"),
      schema("list-keys", "列出按键绑定", "tmux list-keys [选项]"),
      schema("list-panes", "列出 pane", "tmux list-panes [选项]", { options: options(["-F", "-a", "-f", "-s", "-t"], "tmux list-panes 选项", ["-F", "-f", "-t"]) }),
      schema("list-sessions", "列出会话", "tmux list-sessions [选项]", { options: options(["-F", "-f"], "tmux list-sessions 选项", ["-F", "-f"]) }),
      schema("list-windows", "列出窗口", "tmux list-windows [选项]", { options: options(["-F", "-a", "-f", "-t"], "tmux list-windows 选项", ["-F", "-f", "-t"]) }),
      schema("new-session", "创建会话", "tmux new-session [选项] [命令]", { options: options(["-A", "-D", "-P", "-c", "-d", "-e", "-F", "-n", "-s", "-x", "-y"], "tmux new-session 选项", ["-c", "-e", "-F", "-n", "-s", "-x", "-y"]) }),
      schema("new-window", "创建窗口", "tmux new-window [选项] [命令]", { options: options(["-P", "-a", "-c", "-d", "-e", "-F", "-n", "-t"], "tmux new-window 选项", ["-c", "-e", "-F", "-n", "-t"]) }),
      schema("rename-session", "重命名会话", "tmux rename-session [选项] <名称>", { options: tmuxTargetOptions("tmux rename-session 选项") }),
      schema("rename-window", "重命名窗口", "tmux rename-window [选项] <名称>", { options: tmuxTargetOptions("tmux rename-window 选项") }),
      schema("select-pane", "选择 pane", "tmux select-pane [选项]", { options: tmuxTargetOptions("tmux select-pane 选项") }),
      schema("select-window", "选择窗口", "tmux select-window [选项]", { options: tmuxTargetOptions("tmux select-window 选项") }),
      schema("send-keys", "向 pane 发送按键", "tmux send-keys [选项] <按键...>", { options: tmuxTargetOptions("tmux send-keys 选项") }),
      schema("split-window", "拆分窗口", "tmux split-window [选项] [命令]", { options: options(["-P", "-b", "-c", "-d", "-e", "-F", "-h", "-l", "-p", "-t", "-v"], "tmux split-window 选项", ["-c", "-e", "-F", "-l", "-p", "-t"]) }),
      schema("switch-client", "切换客户端会话", "tmux switch-client [选项]", { options: tmuxTargetOptions("tmux switch-client 选项") }),
    ],
  });
}

function tmuxTargetOptions(detail: string): TerminalCommandCatalogEntry[] {
  return options(["-a", "-d", "-t"], detail, ["-t"]);
}

function wingetCommand(): TerminalCommandSchema {
  const globalOptions = ["--accept-source-agreements", "--authentication-account", "--disable-interactivity", "--logs", "--nowarn", "--open-logs", "--proxy", "--source", "--verbose", "--wait"];
  return schema("winget", "管理 Windows 软件包", "winget [全局选项] <命令> [参数...]", {
    options: options(globalOptions, "winget 全局选项", ["--authentication-account", "--proxy", "--source"]),
    subcommands: [
      schema("configure", "应用系统配置", "winget configure [选项] <文件>", { options: options(["--accept-configuration-agreements", "--enable", "--file", "--module-path"], "winget configure 选项", ["--enable", "--file", "--module-path"]) }),
      schema("download", "下载安装包", "winget download [选项] <查询>", { options: wingetPackageOptions("winget download 选项") }),
      schema("export", "导出已安装软件包", "winget export [选项] -o <文件>", { options: options(["--include-versions", "--output", "-o"], "winget export 选项", ["--output", "-o"]) }),
      schema("hash", "计算安装包哈希", "winget hash [选项] <文件>", { options: options(["--file", "--msix", "-f", "-m"], "winget hash 选项", ["--file", "-f"]) }),
      schema("import", "导入软件包列表", "winget import [选项] -i <文件>", { options: options(["--accept-package-agreements", "--accept-source-agreements", "--ignore-unavailable", "--import-file", "--no-upgrade", "-i"], "winget import 选项", ["--import-file", "-i"]) }),
      schema("install", "安装软件包", "winget install [选项] <查询>", { options: wingetPackageOptions("winget install 选项") }),
      schema("list", "列出已安装软件包", "winget list [选项] [查询]", { options: wingetPackageOptions("winget list 选项") }),
      schema("pin", "管理软件包固定", "winget pin <命令> [参数...]", { subcommands: simpleSubcommands([
        ["add", "添加固定规则", "winget pin add [选项] <查询>"],
        ["list", "列出固定规则", "winget pin list [选项]"],
        ["remove", "删除固定规则", "winget pin remove [选项] <查询>"],
        ["reset", "重置固定规则", "winget pin reset [选项]"],
      ]) }),
      schema("search", "搜索软件包", "winget search [选项] <查询>", { options: wingetPackageOptions("winget search 选项") }),
      schema("settings", "打开 winget 设置", "winget settings [选项]"),
      schema("show", "显示软件包详情", "winget show [选项] <查询>", { options: wingetPackageOptions("winget show 选项") }),
      schema("source", "管理软件源", "winget source <命令> [参数...]", { subcommands: simpleSubcommands([
        ["add", "添加软件源", "winget source add [选项]"],
        ["export", "导出软件源", "winget source export [选项]"],
        ["list", "列出软件源", "winget source list [选项]"],
        ["remove", "删除软件源", "winget source remove [选项]"],
        ["reset", "重置软件源", "winget source reset [选项]"],
        ["update", "更新软件源", "winget source update [选项]"],
      ]) }),
      schema("uninstall", "卸载软件包", "winget uninstall [选项] <查询>", { options: wingetPackageOptions("winget uninstall 选项") }),
      schema("upgrade", "升级软件包", "winget upgrade [选项] [查询]", { options: wingetPackageOptions("winget upgrade 选项") }),
      schema("validate", "校验软件包清单", "winget validate [选项] <清单>", { options: options(["--manifest", "-m"], "winget validate 选项", ["--manifest", "-m"]) }),
    ],
  });
}

function wingetPackageOptions(detail: string): TerminalCommandCatalogEntry[] {
  return options(["--accept-package-agreements", "--architecture", "--custom", "--exact", "--force", "--id", "--interactive", "--location", "--manifest", "--moniker", "--name", "--override", "--scope", "--silent", "--source", "--tag", "--version", "-e", "-h", "-i", "-m", "-s", "-v"], detail, ["--architecture", "--custom", "--id", "--location", "--manifest", "--moniker", "--name", "--override", "--scope", "--source", "--tag", "--version", "-m", "-s", "-v"]);
}

function yarnCommand(): TerminalCommandSchema {
  return schema("yarn", "管理 JavaScript 依赖和脚本", "yarn [全局选项] <子命令> [参数...]", {
    options: options(["--cwd", "--help", "--silent", "--version"], "Yarn 全局选项", ["--cwd"]),
    subcommands: [
      schema("add", "添加依赖", "yarn add [选项] <包...>", { options: options(["--dev", "--exact", "--peer", "--tilde", "-D", "-E", "-P", "-T"], "yarn add 选项") }),
      schema("install", "安装依赖", "yarn install [选项]", { options: options(["--immutable", "--mode", "--refresh-lockfile"], "yarn install 选项", ["--mode"]) }),
      schema("remove", "移除依赖", "yarn remove [选项] <包...>", { options: options(["--mode"], "yarn remove 选项", ["--mode"]) }),
      schema("run", "运行 package script", "yarn run <脚本> [参数...]", {}),
      schema("set", "修改 Yarn 配置", "yarn set <子命令> [参数...]", { subcommands: simpleSubcommands([
        ["resolution", "覆盖依赖解析", "yarn set resolution <描述符> <引用>"],
        ["version", "设置 Yarn 版本", "yarn set version [选项] <版本>"],
      ]) }),
      schema("up", "升级依赖", "yarn up [选项] <包...>", { options: options(["--exact", "--interactive", "--mode", "-E", "-i"], "yarn up 选项", ["--mode"]) }),
    ],
  });
}
