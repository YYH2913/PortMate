import { Code2, ExternalLink, FileText, Github, Scale, X } from "lucide-react";
import packageJson from "../package.json";
import { normalizeTerminalWebLink, openIsolatedWebLink } from "./terminal-web-link";

const PROJECT_URL = "https://github.com/YYH2913/PortMate";

const projectLinks = [
  { label: "项目主页", detail: "源代码、发布说明与贡献指南", url: PROJECT_URL, icon: Github },
  { label: "使用文档", detail: "README 与 MCP API 参考", url: `${PROJECT_URL}#readme`, icon: FileText },
  { label: "问题反馈", detail: "报告 Bug 或提出功能建议", url: `${PROJECT_URL}/issues`, icon: Code2 },
  { label: "版本发布", detail: "查看发行包与更新记录", url: `${PROJECT_URL}/releases`, icon: ExternalLink },
  { label: "开源许可", detail: "Apache License 2.0 完整文本", url: `${PROJECT_URL}/blob/main/LICENSE`, icon: Scale },
] as const;

const openSourceComponents = [
  { name: "Tauri", role: "跨平台桌面运行时", license: "Apache-2.0 / MIT", url: "https://github.com/tauri-apps/tauri" },
  { name: "React", role: "工作区界面框架", license: "MIT", url: "https://github.com/facebook/react" },
  { name: "xterm.js", role: "终端仿真与渲染", license: "MIT", url: "https://github.com/xtermjs/xterm.js" },
  { name: "libssh", role: "SSH 传输支持", license: "LGPL-2.1", url: "https://www.libssh.org/" },
  { name: "russh", role: "Rust SSH 协议实现", license: "Apache-2.0", url: "https://github.com/warp-tech/russh" },
  { name: "russh-sftp", role: "Rust SFTP 子系统", license: "Apache-2.0", url: "https://github.com/AspectUnk/russh-sftp" },
  { name: "JetBrains Mono", role: "随应用分发的等宽字体", license: "SIL OFL-1.1", url: "https://github.com/JetBrains/JetBrainsMono" },
] as const;

export default function AboutDialog({ onClose }: { onClose: () => void }) {
  const version = typeof packageJson.version === "string" ? packageJson.version : "unknown";

  function openLink(value: string) {
    const safeLink = normalizeTerminalWebLink(value);
    if (safeLink) openIsolatedWebLink(safeLink);
  }

  return (
    <div className="dialog-backdrop about-backdrop" onMouseDown={(event) => {
      if (event.target === event.currentTarget) onClose();
    }}>
      <section className="wind-dialog about-dialog" role="dialog" aria-modal="true" aria-labelledby="about-title">
        <header className="dialog-title">
          <span className="app-icon" aria-hidden="true" />
          <strong id="about-title">关于 PortMate</strong>
          <button type="button" title="关闭" aria-label="关闭关于 PortMate" onClick={onClose}><X size={20} /></button>
        </header>

        <div className="about-content">
          <section className="about-hero" aria-label="PortMate 产品信息">
            <div className="about-mark" aria-hidden="true"><span>PM</span></div>
            <div className="about-hero-copy">
              <div className="about-name-row"><h1>PortMate</h1><code>v{version}</code></div>
              <p>面向 SSH、串口和远程运维场景的跨平台终端工作台。</p>
              <small>将会话、终端、文件传输、诊断工具与受控 MCP Bridge 集中在一个工作区。</small>
            </div>
          </section>

          <dl className="about-meta" aria-label="PortMate 元数据">
            <div><dt>版本</dt><dd>{version}</dd></div>
            <div><dt>许可证</dt><dd>Apache License 2.0</dd></div>
            <div><dt>产品类型</dt><dd>Tauri v2 桌面应用</dd></div>
            <div><dt>维护者</dt><dd>PortMate Contributors</dd></div>
          </dl>

          <section className="about-section" aria-labelledby="about-links-title">
            <header><div><h2 id="about-links-title">项目链接</h2><p>访问源代码、文档、问题跟踪和发行版本。</p></div><ExternalLink size={15} aria-hidden="true" /></header>
            <div className="about-link-grid">
              {projectLinks.map(({ label, detail, url, icon: Icon }) => (
                <button type="button" key={label} className="about-link" data-url={url} onClick={() => openLink(url)}>
                  <Icon size={16} aria-hidden="true" />
                  <span><strong>{label}</strong><small>{detail}</small></span>
                  <ExternalLink size={13} aria-hidden="true" />
                </button>
              ))}
            </div>
          </section>

          <section className="about-section" aria-labelledby="about-open-source-title">
            <header><div><h2 id="about-open-source-title">使用的开源代码</h2><p>PortMate 依赖并致谢以下开源项目；各组件遵循其各自许可证。</p></div><Code2 size={15} aria-hidden="true" /></header>
            <div className="about-components" role="list" aria-label="开源组件列表">
              {openSourceComponents.map(({ name, role, license, url }) => (
                <div className="about-component" role="listitem" key={name}>
                  <button type="button" data-url={url} onClick={() => openLink(url)} title={`打开 ${name} 项目主页`}><strong>{name}</strong><ExternalLink size={12} aria-hidden="true" /></button>
                  <span>{role}</span>
                  <code>{license}</code>
                </div>
              ))}
            </div>
            <button type="button" className="about-license-link" data-url={`${PROJECT_URL}/tree/main/THIRD_PARTY_LICENSES`} onClick={() => openLink(`${PROJECT_URL}/tree/main/THIRD_PARTY_LICENSES`)}><Scale size={14} aria-hidden="true" />查看完整第三方许可证清单</button>
          </section>

          <p className="about-notice">PortMate 处于持续开发阶段。使用前请根据目标平台和运行环境完成相应的安全、兼容性与网络访问验证。<br />Copyright © 2026 PortMate Contributors</p>
        </div>

        <footer className="about-actions"><button type="button" onClick={onClose}>关闭</button></footer>
      </section>
    </div>
  );
}
