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
  schema("bash", "启动 Bash shell", "bash [选项] [脚本 [参数...]]", {
    options: options(["--help", "--version", "-c", "-i", "-l", "-n", "-x"], "Bash 选项"),
  }),
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
  schema("clear", "清空终端屏幕", "clear"),
  schema("cp", "复制文件或目录", "cp [选项] <源...> <目标>", {
    options: options(["--parents", "-a", "-f", "-i", "-n", "-r", "-v"], "cp 选项"),
    arguments: commonPathArguments,
  }),
  schema("curl", "传输 URL 数据", "curl [选项] <URL...>", {
    options: options(["--connect-timeout", "--fail", "--max-time", "-H", "-I", "-L", "-o", "-X", "-d"], "curl 选项"),
  }),
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
  schema("echo", "输出文本", "echo [选项] [文本...]", { options: options(["-E", "-e", "-n"], "echo 选项") }),
  schema("find", "查找文件", "find [起始路径...] [表达式]", {
    options: options(["-maxdepth", "-mindepth", "-mtime", "-name", "-path", "-size", "-type"], "find 条件"),
    arguments: commonPathArguments,
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
  schema("grep", "搜索文本", "grep [选项] <模式> [文件...]", { options: options(["--color=auto", "-E", "-F", "-i", "-n", "-r", "-v"], "grep 选项") }),
  schema("head", "输出文件开头", "head [选项] [文件...]", { options: options(["--bytes", "--lines", "-c", "-n", "-q", "-v"], "head 选项"), arguments: commonPathArguments }),
  schema("journalctl", "查询 systemd 日志", "journalctl [选项] [匹配条件...]", { options: options(["--boot", "--follow", "--no-pager", "--since", "--until", "-b", "-f", "-n", "-u"], "journalctl 选项") }),
  schema("kill", "向进程发送信号", "kill [选项] <PID...>", { options: options(["--list", "-HUP", "-INT", "-KILL", "-TERM", "-l", "-s"], "kill 信号或选项") }),
  schema("ls", "列出目录内容", "ls [选项] [路径...]", { options: options(["--color=auto", "-R", "-a", "-h", "-l", "-t"], "ls 选项"), arguments: commonPathArguments }),
  schema("mkdir", "创建目录", "mkdir [选项] <目录...>", { options: options(["--mode", "--parents", "-m", "-p", "-v"], "mkdir 选项") }),
  schema("mv", "移动或重命名文件", "mv [选项] <源...> <目标>", { options: options(["--backup", "--target-directory", "-f", "-i", "-n", "-t", "-v"], "mv 选项"), arguments: commonPathArguments }),
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
  schema("pwd", "显示当前目录", "pwd [选项]", { options: options(["-L", "-P"], "pwd 选项") }),
  schema("rm", "删除文件或目录", "rm [选项] <路径...>", { options: options(["--one-file-system", "-d", "-f", "-i", "-r", "-v"], "rm 选项"), arguments: commonPathArguments }),
  schema("rsync", "同步文件和目录", "rsync [选项] <源...> <目标>", { options: options(["--delete", "--dry-run", "--exclude", "--progress", "-a", "-e", "-n", "-v", "-z"], "rsync 选项") }),
  schema("scp", "通过 SSH 复制文件", "scp [选项] <源...> <目标>", { options: options(["-3", "-C", "-J", "-P", "-i", "-p", "-r", "-v"], "SCP 选项") }),
  schema("ssh", "连接 SSH 主机", "ssh [选项] [用户@]主机 [命令...]", { options: options(["-4", "-6", "-A", "-D", "-J", "-L", "-R", "-i", "-o", "-p", "-t", "-v"], "SSH 选项") }),
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
  schema("tail", "输出文件末尾", "tail [选项] [文件...]", { options: options(["--follow", "--lines", "--pid", "--retry", "-F", "-f", "-n"], "tail 选项"), arguments: commonPathArguments }),
  schema("tar", "归档文件", "tar <操作> [选项] [文件...]", { options: options(["--directory", "--exclude", "-C", "-c", "-f", "-j", "-t", "-v", "-x", "-z"], "tar 操作或选项") }),
  schema("top", "查看系统进程", "top [选项]", { options: options(["-H", "-b", "-d", "-n", "-p", "-u"], "top 选项") }),
  schema("uname", "显示系统信息", "uname [选项]", { options: options(["--all", "--kernel-release", "--machine", "--operating-system", "-a", "-m", "-r", "-s"], "uname 选项") }),
  schema("vi", "启动 Vi 编辑器", "vi [选项] [文件...]", { options: options(["-R", "-c", "-d", "-n", "-u"], "Vi 选项"), arguments: commonPathArguments }),
  schema("vim", "启动 Vim 编辑器", "vim [选项] [文件...]", { options: options(["--clean", "-R", "-c", "-d", "-n", "-u"], "Vim 选项"), arguments: commonPathArguments }),
  schema("wget", "下载网络资源", "wget [选项] <URL...>", { options: options(["--continue", "--directory-prefix", "--output-document", "--quiet", "-O", "-P", "-c", "-q"], "wget 选项") }),
  schema("whoami", "显示当前用户", "whoami [选项]", { options: options(["--help", "--version"], "whoami 选项") }),
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
