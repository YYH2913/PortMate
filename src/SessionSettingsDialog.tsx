import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { Lock, Plus, Trash2, X } from "lucide-react";
import { invokeBackend } from "./api";
import type { ProxyPasswordUpdate } from "./proxy-settings";
import {
  convertDraftProtocol,
  createDefaultTrigger,
  createIdentityRef,
  createJumpHostKeyPolicy,
  createSerialConnection,
  createShellConnection,
  createSshConnection,
  createTcpConnection,
  describeHostKeyEvaluation,
  formatSshTarget,
  parseSshTarget,
  profileCredentialSecretRefs,
  protocolFromKind,
  serialPortOptions,
} from "./session-profile-helpers";
import {
  flattenSessionTree,
  MAX_SESSION_PROFILE_GROUP_CHARACTERS,
  MAX_SESSION_PROFILE_NAME_CHARACTERS,
  MAX_SESSION_PROFILE_TAG_INPUT_CHARACTERS,
  normalizeSessionMetadataText,
  protocolTabs,
  sessionSettingTrees,
} from "./session-settings-state";
import type { ProtocolTab } from "./session-settings-state";
import { serialConnectionBounds } from "./serial-connection-settings";
import { SSH_AUTH_ORDER_OPTIONS, sshConnectionBounds } from "./ssh-connection-settings";
import { tcpConnectionBounds } from "./tcp-connection-settings";
import {
  MAX_TERMINAL_FONT_FAMILY_CHARACTERS,
  MAX_TERMINAL_NAME_BYTES,
  TERMINAL_PROFILE_BOUNDS,
} from "./terminal-settings-state";
import { normalizeTerminalTheme, TERMINAL_THEME_OPTIONS } from "./terminal-theme";
import {
  canAddTrigger,
  canAddTriggerAction,
  defaultTriggerAction,
  MAX_TRIGGER_ACTION_VALUE_CHARACTERS,
  MAX_TRIGGER_LABEL_CHARACTERS,
  MAX_TRIGGER_MATCHER_CHARACTERS,
  patchTriggerAction,
  triggerActionValue,
} from "./trigger-state";
import type {
  AuthMethod,
  HostKeyPolicy,
  HostKeyScanResult,
  IdentityRef,
  JumpHop,
  ProxyConfig,
  SessionProfile,
  SessionSummary,
  TriggerAction,
  TriggerSpec,
  TrustedHostKey,
} from "./types";

type HostKeyDecisionValue = "trust-once" | "append-to-profile" | "append-to-project" | "replace-for-profile";

export default function SessionSettingsDialog({
  draft,
  prepareProfile,
  serialPorts,
  initialSection,
  onDraftChange,
  onSave,
  onConnect,
  onClose,
}: {
  draft: SessionProfile;
  prepareProfile: (profile: SessionProfile) => SessionProfile;
  serialPorts: string[];
  initialSection: string;
  onDraftChange: (draft: SessionProfile) => void;
  onSave: (proxyPasswordUpdate: ProxyPasswordUpdate) => Promise<SessionSummary | null>;
  onConnect: (saved: SessionSummary) => void;
  onClose: () => void;
}) {
  const [activeProtocol, setActiveProtocol] = useState<ProtocolTab>(() => protocolFromKind(draft.kind));
  const [activeSection, setActiveSection] = useState(initialSection);
  const [proxyPasswordUpdate, setProxyPasswordUpdate] = useState<ProxyPasswordUpdate>(null);
  const [secretWriteCount, setSecretWriteCount] = useState(0);
  const [submitBusy, setSubmitBusy] = useState(false);
  const [secretCleanupError, setSecretCleanupError] = useState("");
  const stagedSecretRefs = useRef(new Set<string>());
  const sessionTree = sessionSettingTrees[activeProtocol];
  const allowedSections = useMemo(() => flattenSessionTree(sessionTree), [sessionTree]);
  const busy = submitBusy || secretWriteCount > 0;

  useEffect(() => {
    if (!allowedSections.includes(activeSection)) {
      setActiveSection("会话");
    }
  }, [activeSection, allowedSections]);

  useEffect(() => {
    setActiveProtocol(protocolFromKind(draft.kind));
    setActiveSection(initialSection);
  }, [draft.id, draft.kind, initialSection]);

  function changeProtocol(tab: ProtocolTab) {
    if (busy) return;
    setActiveProtocol(tab);
    setActiveSection("会话");
    setProxyPasswordUpdate(null);
    onDraftChange(convertDraftProtocol(draft, tab));
  }

  async function cleanupStagedSecrets(retained = new Set<string>()) {
    for (const secretRef of retained) stagedSecretRefs.current.delete(secretRef);
    const failures: string[] = [];
    for (const secretRef of [...stagedSecretRefs.current]) {
      try {
        await invokeBackend("delete_secret", { secretRef });
        stagedSecretRefs.current.delete(secretRef);
      } catch (error) {
        failures.push(formatError(error));
      }
    }
    setSecretCleanupError(failures.length ? `暂存凭据清理失败: ${failures.join("；")}` : "");
    return failures.length === 0;
  }

  async function submit(connectAfterSave: boolean) {
    if (busy) return;
    setSubmitBusy(true);
    setSecretCleanupError("");
    const saved = await onSave(proxyPasswordUpdate);
    if (!saved) {
      setSubmitBusy(false);
      return;
    }
    const cleaned = await cleanupStagedSecrets(profileCredentialSecretRefs(saved.profile));
    if (!cleaned) {
      setSubmitBusy(false);
      return;
    }
    onClose();
    if (connectAfterSave) onConnect(saved);
  }

  async function cancel() {
    if (busy) return;
    setSubmitBusy(true);
    setSecretCleanupError("");
    if (await cleanupStagedSecrets()) onClose();
    else setSubmitBusy(false);
  }

  return (
    <DialogFrame
      title="会话设置"
      className={`session-settings-dialog ${activeSection === "会话" ? "compact" : activeSection === "传输" ? "medium" : ""}`}
      onClose={() => void cancel()}
      closeDisabled={busy}
    >
      <div className="session-settings-nav">
        <label>
          <span>会话类型</span>
          <select aria-label="会话类型" value={activeProtocol} onChange={(event) => changeProtocol(event.target.value as ProtocolTab)} disabled={busy}>
            {protocolTabs.map((tab) => <option key={tab} value={tab}>{tab}</option>)}
          </select>
        </label>
        <label>
          <span>配置项</span>
          <select aria-label="会话配置项" value={activeSection} onChange={(event) => setActiveSection(event.target.value)} disabled={busy}>
            {allowedSections.map((section) => <option key={section} value={section}>{section}</option>)}
          </select>
        </label>
      </div>
      <section className="session-form">
        <SessionSettingsContent
          activeProtocol={activeProtocol}
          activeSection={activeSection}
          draft={draft}
          prepareProfile={prepareProfile}
          serialPorts={serialPorts}
          onDraftChange={onDraftChange}
          proxyPasswordUpdate={proxyPasswordUpdate}
          onProxyPasswordUpdateChange={setProxyPasswordUpdate}
          secretWriteBusy={secretWriteCount > 0}
          onSecretWriteStart={() => setSecretWriteCount((current) => current + 1)}
          onSecretCreated={(secretRef) => stagedSecretRefs.current.add(secretRef)}
          onSecretWriteFinish={() => setSecretWriteCount((current) => Math.max(0, current - 1))}
        />
      </section>
      {secretCleanupError ? <div className="utility-error">{secretCleanupError}</div> : null}
      <div className="dialog-actions">
        <button onClick={() => void submit(false)} disabled={busy}>保存</button>
        <button onClick={() => void submit(true)} disabled={busy}>保存并连接</button>
        <button onClick={() => void cancel()} disabled={busy}>取消</button>
      </div>
    </DialogFrame>
  );
}

function SessionSettingsContent({
  activeProtocol,
  activeSection,
  draft,
  prepareProfile,
  serialPorts,
  onDraftChange,
  proxyPasswordUpdate,
  onProxyPasswordUpdateChange,
  secretWriteBusy,
  onSecretWriteStart,
  onSecretCreated,
  onSecretWriteFinish,
}: {
  activeProtocol: ProtocolTab;
  activeSection: string;
  draft: SessionProfile;
  prepareProfile: (profile: SessionProfile) => SessionProfile;
  serialPorts: string[];
  onDraftChange: (draft: SessionProfile) => void;
  proxyPasswordUpdate: ProxyPasswordUpdate;
  onProxyPasswordUpdateChange: (update: ProxyPasswordUpdate) => void;
  secretWriteBusy: boolean;
  onSecretWriteStart: () => void;
  onSecretCreated: (secretRef: string) => void;
  onSecretWriteFinish: () => void;
}) {
  if (activeSection === "会话") {
    return <SessionCommonOverviewFields draft={draft} onDraftChange={onDraftChange} />;
  }

  if (activeSection === "终端") {
    return (
      <>
        <DialogField label="终端:(T)">
          <input value={draft.terminal.term} maxLength={MAX_TERMINAL_NAME_BYTES} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, term: event.target.value } })} />
        </DialogField>
        <DialogField label="行:(R)">
          <input type="number" min={TERMINAL_PROFILE_BOUNDS.rows.min} max={TERMINAL_PROFILE_BOUNDS.rows.max} step={1} value={draft.terminal.rows} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, rows: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label="列:(C)">
          <input type="number" min={TERMINAL_PROFILE_BOUNDS.cols.min} max={TERMINAL_PROFILE_BOUNDS.cols.max} step={1} value={draft.terminal.cols} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, cols: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label="滚屏:(S)">
          <input type="number" min={TERMINAL_PROFILE_BOUNDS.scrollback.min} max={TERMINAL_PROFILE_BOUNDS.scrollback.max} step={1} value={draft.terminal.scrollback} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, scrollback: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label="字体:(F)">
          <input value={draft.terminal.fontFamily} maxLength={MAX_TERMINAL_FONT_FAMILY_CHARACTERS} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, fontFamily: event.target.value } })} />
        </DialogField>
        <DialogField label="字号:(Z)">
          <input type="number" min={TERMINAL_PROFILE_BOUNDS.fontSize.min} max={TERMINAL_PROFILE_BOUNDS.fontSize.max} step={1} value={draft.terminal.fontSize} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, fontSize: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label="主题:(M)">
          <select value={normalizeTerminalTheme(draft.terminal.theme)} onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, theme: event.target.value } })}>
            {TERMINAL_THEME_OPTIONS.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
          </select>
        </DialogField>
        <DialogField label="背景不透明度:(O)">
          <div className="terminal-opacity-control">
            <input
              type="range"
              min={TERMINAL_PROFILE_BOUNDS.backgroundOpacity.min}
              max={TERMINAL_PROFILE_BOUNDS.backgroundOpacity.max}
              step={5}
              value={draft.terminal.backgroundOpacity ?? TERMINAL_PROFILE_BOUNDS.backgroundOpacity.fallback}
              onChange={(event) => onDraftChange({ ...draft, terminal: { ...draft.terminal, backgroundOpacity: Number(event.target.value) } })}
            />
            <output>{draft.terminal.backgroundOpacity ?? TERMINAL_PROFILE_BOUNDS.backgroundOpacity.fallback}%</output>
          </div>
        </DialogField>
      </>
    );
  }

  if (activeSection === "日志") {
    return (
      <>
        <DialogField label="启用:(E)">
          <select value={draft.logging.enabled ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, enabled: event.target.value === "on" } })}>
            <option value="on">开启</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
        <DialogField label="Raw（不脱敏）:(R)">
          <select value={draft.logging.raw ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, raw: event.target.value === "on" } })}>
            <option value="on">开启</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
        <DialogField label="Text:(T)">
          <select value={draft.logging.text ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, text: event.target.value === "on" } })}>
            <option value="on">开启</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
        <DialogField label="JSONL:(J)">
          <select value={draft.logging.jsonl ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, jsonl: event.target.value === "on" } })}>
            <option value="on">开启</option>
            <option value="off">关闭</option>
          </select>
        </DialogField>
        <DialogField label="敏感字段:(S)">
          <select value={draft.logging.redactSecrets ? "redact" : "plain"} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, redactSecrets: event.target.value === "redact" } })}>
            <option value="redact">隐藏</option>
            <option value="plain">完整记录</option>
          </select>
        </DialogField>
        <DialogField label="路径:(P)">
          <input value={draft.logging.pathTemplate} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, pathTemplate: event.target.value } })} />
        </DialogField>
        <DialogField label="保留天数:(D)">
          <input type="number" min={0} max={3650} value={draft.logging.retentionDays ?? 0} onChange={(event) => onDraftChange({ ...draft, logging: { ...draft.logging, retentionDays: Math.min(3650, Math.max(0, Math.trunc(Number(event.target.value) || 0))) } })} />
        </DialogField>
      </>
    );
  }

  if (activeSection === "触发器") {
    return <TriggerFields draft={draft} onDraftChange={onDraftChange} />;
  }

  if (activeSection === "传输") {
    return <SessionTransferFields activeProtocol={activeProtocol} draft={draft} onDraftChange={onDraftChange} />;
  }

  if (activeProtocol === "Shell" && activeSection === "Shell") {
    return <ShellProcessFields draft={draft} onDraftChange={onDraftChange} />;
  }

  if ((activeProtocol === "SSH" || activeProtocol === "Tmux") && (activeSection === "SSH" || activeSection === "Tmux")) {
    return <SshAdvancedFields section="连接" draft={draft} prepareProfile={prepareProfile} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} secretWriteBusy={secretWriteBusy} onSecretWriteStart={onSecretWriteStart} onSecretCreated={onSecretCreated} onSecretWriteFinish={onSecretWriteFinish} />;
  }

  if ((activeProtocol === "SSH" || activeProtocol === "Tmux") && ["代理", "验证", "代理人", "密码", "公钥"].includes(activeSection)) {
    return <SshAdvancedFields section={activeSection} draft={draft} prepareProfile={prepareProfile} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} secretWriteBusy={secretWriteBusy} onSecretWriteStart={onSecretWriteStart} onSecretCreated={onSecretCreated} onSecretWriteFinish={onSecretWriteFinish} />;
  }

  if (activeProtocol === "Telnet" && activeSection === "Telnet") {
    return <TcpLikeAdvancedFields protocol="Telnet" section="连接" draft={draft} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} />;
  }

  if (activeProtocol === "Tcp" && activeSection === "Tcp") {
    return <TcpLikeAdvancedFields protocol="Tcp" section="连接" draft={draft} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} />;
  }

  if ((activeProtocol === "Telnet" || activeProtocol === "Tcp") && activeSection === "代理") {
    return <TcpLikeAdvancedFields protocol={activeProtocol} section="代理" draft={draft} onDraftChange={onDraftChange} proxyPasswordUpdate={proxyPasswordUpdate} onProxyPasswordUpdateChange={onProxyPasswordUpdateChange} />;
  }

  if (activeProtocol === "Serial" && activeSection === "串口") {
    return <SerialAdvancedFields draft={draft} serialPorts={serialPorts} onDraftChange={onDraftChange} />;
  }

  return null;
}

function SessionCommonOverviewFields({
  draft,
  onDraftChange,
}: {
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
}) {
  const [tagsText, setTagsText] = useState(() => draft.tags.join(", "));
  useEffect(() => {
    setTagsText(draft.tags.join(", "));
  }, [draft.id]);
  return (
    <>
      <DialogField label="名称:(N)">
        <input value={draft.name} onChange={(event) => onDraftChange({ ...draft, name: normalizeSessionMetadataText(event.target.value, MAX_SESSION_PROFILE_NAME_CHARACTERS) })} />
      </DialogField>
      <DialogField label="分组:(G)">
        <input value={draft.group} onChange={(event) => onDraftChange({ ...draft, group: normalizeSessionMetadataText(event.target.value, MAX_SESSION_PROFILE_GROUP_CHARACTERS) })} placeholder="[嵌套组] a>b>c" />
      </DialogField>
      <DialogField label="标签:(L)">
        <input value={tagsText} onChange={(event) => {
          const nextText = normalizeSessionMetadataText(event.target.value, MAX_SESSION_PROFILE_TAG_INPUT_CHARACTERS);
          setTagsText(nextText);
          onDraftChange({ ...draft, tags: nextText.split(",").map((item) => item.trim()).filter(Boolean) });
        }} />
      </DialogField>
    </>
  );
}

function SessionTransferFields({
  activeProtocol,
  draft,
  onDraftChange,
}: {
  activeProtocol: ProtocolTab;
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
}) {
  const update = (patch: Partial<SessionProfile["transfer"]>) => onDraftChange({
    ...draft,
    transfer: { ...draft.transfer, ...patch },
  });
  const sshLike = activeProtocol === "SSH" || activeProtocol === "Tmux";

  return (
    <>
      {sshLike ? <DialogToggleField label="SFTP:" checked={draft.transfer.sftp} onChange={(sftp) => update({ sftp })} /> : null}
      {sshLike ? <DialogToggleField label="SCP:" checked={draft.transfer.scp} onChange={(scp) => update({ scp })} /> : null}
      <DialogToggleField label="XModem:" checked={draft.transfer.xmodem} onChange={(xmodem) => update({ xmodem })} />
      <DialogToggleField label="YModem:" checked={draft.transfer.ymodem} onChange={(ymodem) => update({ ymodem })} />
      <DialogToggleField label="ZModem:" checked={draft.transfer.zmodem} onChange={(zmodem) => update({ zmodem })} />
      <DialogField label="限速 B/s:">
        <input type="number" min={0} value={draft.transfer.rateLimitBytesPerSecond ?? 0} onChange={(event) => update({ rateLimitBytesPerSecond: Number(event.target.value) > 0 ? Number(event.target.value) : null })} />
      </DialogField>
      <DialogField label="默认目录:(D)">
        <input value={draft.transfer.defaultLocalDir ?? ""} onChange={(event) => update({ defaultLocalDir: event.target.value || null })} />
      </DialogField>
    </>
  );
}

function ShellProcessFields({
  draft,
  onDraftChange,
}: {
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
}) {
  const shell = draft.connection.kind === "shell" ? draft.connection : createShellConnection();
  return (
    <>
      <DialogField label="程序:(P)">
        <input value={shell.program} onChange={(event) => onDraftChange({ ...draft, kind: "shell", connection: { ...shell, program: event.target.value } })} />
      </DialogField>
      <DialogField label="参数:(A)">
        <input value={shell.args.join(" ")} onChange={(event) => onDraftChange({ ...draft, kind: "shell", connection: { ...shell, args: event.target.value.split(" ").filter(Boolean) } })} />
      </DialogField>
      <DialogField label="目录:(W)">
        <input value={shell.cwd ?? ""} onChange={(event) => onDraftChange({ ...draft, kind: "shell", connection: { ...shell, cwd: event.target.value || null } })} />
      </DialogField>
    </>
  );
}

function TriggerFields({ draft, onDraftChange }: { draft: SessionProfile; onDraftChange: (draft: SessionProfile) => void }) {
  function setTriggers(triggers: TriggerSpec[]) {
    onDraftChange({ ...draft, triggers });
  }

  function updateTrigger(index: number, patch: Partial<TriggerSpec>) {
    setTriggers(draft.triggers.map((trigger, triggerIndex) => (
      triggerIndex === index ? { ...trigger, ...patch } : trigger
    )));
  }

  function updateAction(triggerIndex: number, actionIndex: number, action: TriggerAction) {
    const trigger = draft.triggers[triggerIndex];
    updateTrigger(triggerIndex, {
      actions: trigger.actions.map((item, index) => (index === actionIndex ? action : item)),
    });
  }

  return (
    <div className="trigger-editor">
      {draft.triggers.map((trigger, triggerIndex) => {
        const matcherValue = trigger.matcher.type === "regex" ? trigger.matcher.pattern : trigger.matcher.text;
        return (
          <section className="trigger-item" key={trigger.id}>
            <header className="trigger-item-header">
              <label className="trigger-enabled">
                <input type="checkbox" checked={trigger.enabled} onChange={(event) => updateTrigger(triggerIndex, { enabled: event.target.checked })} />
                <span>启用</span>
              </label>
              <input aria-label="触发器名称" maxLength={MAX_TRIGGER_LABEL_CHARACTERS} value={trigger.label} onChange={(event) => updateTrigger(triggerIndex, { label: event.target.value })} />
              <button type="button" className="icon-button" title="删除触发器" aria-label="删除触发器" onClick={() => setTriggers(draft.triggers.filter((_, index) => index !== triggerIndex))}><Trash2 size={14} /></button>
            </header>
            <div className="trigger-matcher-row">
              <select
                aria-label="匹配类型"
                value={trigger.matcher.type}
                onChange={(event) => updateTrigger(triggerIndex, {
                  matcher: event.target.value === "regex"
                    ? { type: "regex", pattern: matcherValue }
                    : { type: "contains", text: matcherValue, case_sensitive: false },
                })}
              >
                <option value="contains">包含文本</option>
                <option value="regex">正则表达式</option>
              </select>
              <input
                aria-label="匹配内容"
                maxLength={MAX_TRIGGER_MATCHER_CHARACTERS}
                value={matcherValue}
                onChange={(event) => updateTrigger(triggerIndex, {
                  matcher: trigger.matcher.type === "regex"
                    ? { type: "regex", pattern: event.target.value }
                    : { ...trigger.matcher, text: event.target.value },
                })}
              />
              <label className="trigger-case-toggle">
                <input
                  type="checkbox"
                  checked={trigger.matcher.type === "contains" && trigger.matcher.case_sensitive}
                  disabled={trigger.matcher.type === "regex"}
                  onChange={(event) => {
                    if (trigger.matcher.type === "contains") {
                      updateTrigger(triggerIndex, { matcher: { ...trigger.matcher, case_sensitive: event.target.checked } });
                    }
                  }}
                />
                <span>区分大小写</span>
              </label>
            </div>
            <div className="trigger-action-list">
              {trigger.actions.map((action, actionIndex) => (
                <div className="trigger-action-row" key={`${trigger.id}-${actionIndex}`}>
                  <select
                    aria-label="动作类型"
                    value={action.type}
                    onChange={(event) => updateAction(triggerIndex, actionIndex, defaultTriggerAction(event.target.value as TriggerAction["type"]))}
                  >
                    <option value="timeline-mark">时间线标记</option>
                    <option value="notification">通知</option>
                    <option value="highlight">高亮</option>
                    <option value="send-text">发送文本</option>
                    <option value="local-command">本地命令</option>
                    <option value="custom-link">自定义链接</option>
                    <option value="sound">声音</option>
                  </select>
                  {action.type === "sound" ? (
                    <select aria-label="声音" value={action.name} onChange={(event) => updateAction(triggerIndex, actionIndex, { type: "sound", name: event.target.value })}>
                      <option value="bell">Bell</option>
                      <option value="chime">Chime</option>
                      <option value="alert">Alert</option>
                    </select>
                  ) : (
                    <input
                      aria-label="动作参数"
                      maxLength={MAX_TRIGGER_ACTION_VALUE_CHARACTERS}
                      value={triggerActionValue(action)}
                      onChange={(event) => updateAction(triggerIndex, actionIndex, patchTriggerAction(action.type, event.target.value))}
                    />
                  )}
                  <button type="button" className="icon-button" title="删除动作" aria-label="删除动作" onClick={() => updateTrigger(triggerIndex, { actions: trigger.actions.filter((_, index) => index !== actionIndex) })}><Trash2 size={14} /></button>
                </div>
              ))}
              <button type="button" className="trigger-add-action" disabled={!canAddTriggerAction(trigger.actions.length)} title={canAddTriggerAction(trigger.actions.length) ? "添加动作" : "每条触发器最多 16 个动作"} onClick={() => updateTrigger(triggerIndex, { actions: [...trigger.actions, defaultTriggerAction("timeline-mark")] })}><Plus size={14} />添加动作</button>
            </div>
          </section>
        );
      })}
      <button type="button" className="trigger-add" disabled={!canAddTrigger(draft.triggers.length)} title={canAddTrigger(draft.triggers.length) ? "添加触发器" : "每个会话最多 64 条触发器"} onClick={() => setTriggers([...draft.triggers, createDefaultTrigger()])}><Plus size={14} />添加触发器</button>
    </div>
  );
}

function SshAdvancedFields({
  section,
  draft,
  prepareProfile,
  onDraftChange,
  proxyPasswordUpdate,
  onProxyPasswordUpdateChange,
  secretWriteBusy,
  onSecretWriteStart,
  onSecretCreated,
  onSecretWriteFinish,
}: {
  section: string;
  draft: SessionProfile;
  prepareProfile: (profile: SessionProfile) => SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
  proxyPasswordUpdate: ProxyPasswordUpdate;
  onProxyPasswordUpdateChange: (update: ProxyPasswordUpdate) => void;
  secretWriteBusy: boolean;
  onSecretWriteStart: () => void;
  onSecretCreated: (secretRef: string) => void;
  onSecretWriteFinish: () => void;
}) {
  const ssh = draft.connection.kind === "ssh" || draft.connection.kind === "tmux" ? draft.connection : createSshConnection();
  const kind = draft.connection.kind === "tmux" ? "tmux" : "ssh";
  const [vaultPrivateKey, setVaultPrivateKey] = useState("");
  const [vaultStatus, setVaultStatus] = useState("");
  const [vaultBusy, setVaultBusy] = useState(false);
  const [secretStatus, setSecretStatus] = useState("");
  const [hostKeyScan, setHostKeyScan] = useState<HostKeyScanResult | null>(null);
  const [hostKeyStatus, setHostKeyStatus] = useState("");
  const [jumpSecretDrafts, setJumpSecretDrafts] = useState<Record<string, string>>({});
  const [jumpStatus, setJumpStatus] = useState("");

  if (section === "连接") {
    const updateSsh = (patch: Partial<typeof ssh>) => onDraftChange({
      ...draft,
      kind,
      connection: { ...ssh, ...patch, kind },
    });
    const updateJump = (index: number, patch: Partial<JumpHop>) => {
      const jumps = ssh.jumps.map((jump, jumpIndex) => (jumpIndex === index ? { ...jump, ...patch } : jump));
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, jumps } });
    };
    const addJump = () => {
      const next: JumpHop = { host: "", port: 22, username: ssh.username, passwordSecretRef: null, passphraseSecretRef: null, identityRef: null, hostKeyPolicy: null };
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, jumps: [...ssh.jumps, next] } });
    };
    const removeJump = (index: number) => {
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, jumps: ssh.jumps.filter((_, jumpIndex) => jumpIndex !== index) } });
    };
    const updateJumpPolicy = (index: number, patch: Partial<HostKeyPolicy>) => {
      const jump = ssh.jumps[index];
      if (!jump) return;
      updateJump(index, { hostKeyPolicy: { ...createJumpHostKeyPolicy(jump), ...(jump.hostKeyPolicy ?? {}), ...patch } });
    };
    const jumpSecretKey = (index: number, field: "passwordSecretRef" | "passphraseSecretRef") => `${index}:${field}`;
    const setJumpSecretDraft = (index: number, field: "passwordSecretRef" | "passphraseSecretRef", value: string) => {
      setJumpSecretDrafts((current) => ({ ...current, [jumpSecretKey(index, field)]: value }));
    };
    const saveJumpSecret = async (index: number, field: "passwordSecretRef" | "passphraseSecretRef") => {
      if (secretWriteBusy) return;
      const jump = ssh.jumps[index];
      if (!jump) return;
      const secret = jumpSecretDrafts[jumpSecretKey(index, field)] ?? "";
      if (!secret.trim()) return;
      onSecretWriteStart();
      setJumpStatus("");
      try {
        const response = await invokeBackend<{ secretRef: string }>("save_secret", {
          request: { secretRef: null, secret },
        });
        onSecretCreated(response.secretRef);
        const patch: Partial<JumpHop> = field === "passwordSecretRef" ? { passwordSecretRef: response.secretRef } : { passphraseSecretRef: response.secretRef };
        updateJump(index, patch);
        setJumpSecretDrafts((current) => ({ ...current, [jumpSecretKey(index, field)]: "" }));
        setJumpStatus("已保存跳板凭据");
      } catch (error) {
        setJumpStatus(formatError(error));
      } finally {
        onSecretWriteFinish();
      }
    };
    const deleteJumpSecret = (index: number, field: "passwordSecretRef" | "passphraseSecretRef") => {
      const jump = ssh.jumps[index];
      const secretRef = field === "passwordSecretRef" ? jump?.passwordSecretRef : jump?.passphraseSecretRef;
      if (!secretRef) return;
      setJumpStatus("");
      const patch: Partial<JumpHop> = field === "passwordSecretRef" ? { passwordSecretRef: null } : { passphraseSecretRef: null };
      updateJump(index, patch);
      setJumpStatus("保存 Profile 后清理未引用凭据");
    };
    return (
      <>
        <DialogField label="主机:(H)">
          <input
            value={formatSshTarget(ssh)}
            placeholder="[用户@]主机地址"
            onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...parseSshTarget(event.target.value, ssh), kind } })}
          />
        </DialogField>
        <DialogField label="端口:(P)">
          <input type="number" min={1} max={65535} value={ssh.endpoint.port} onChange={(event) => updateSsh({ endpoint: { ...ssh.endpoint, port: Number(event.target.value) } })} />
        </DialogField>
        <DialogField label="别名:(A)">
          <input value={ssh.hostKeyPolicy.alias ?? ""} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, alias: event.target.value || null } } })} />
        </DialogField>
        <DialogField label="Jump Host:">
          <div className="jump-list">
            {ssh.jumps.map((jump, index) => {
              const policy = jump.hostKeyPolicy ?? createJumpHostKeyPolicy(jump);
              return (
                <div className="jump-hop" key={index}>
                  <div className="jump-hop-row">
                    <span className="jump-hop-index">{index + 1}</span>
                    <input value={jump.host} onChange={(event) => updateJump(index, { host: event.target.value })} placeholder="host" />
                    <input type="number" value={jump.port} onChange={(event) => updateJump(index, { port: Number(event.target.value) || 22 })} aria-label={`Jump ${index + 1} port`} />
                    <input value={jump.username} onChange={(event) => updateJump(index, { username: event.target.value })} placeholder="user" />
                    <input value={jump.identityRef ?? ""} onChange={(event) => updateJump(index, { identityRef: event.target.value || null })} placeholder="identity id" />
                    <button type="button" className="icon-button" onClick={() => removeJump(index)} title="删除跳板">
                      <X size={14} />
                    </button>
                  </div>
                  <div className="jump-hop-extra">
                    <input type="password" value={jumpSecretDrafts[jumpSecretKey(index, "passwordSecretRef")] ?? ""} onChange={(event) => setJumpSecretDraft(index, "passwordSecretRef", event.target.value)} placeholder="password" />
                    <button type="button" className="icon-button" onClick={() => void saveJumpSecret(index, "passwordSecretRef")} title="保存跳板密码" disabled={secretWriteBusy || !(jumpSecretDrafts[jumpSecretKey(index, "passwordSecretRef")] ?? "").trim()}>
                      <Lock size={14} />
                    </button>
                    <input value={jump.passwordSecretRef ?? ""} onChange={(event) => updateJump(index, { passwordSecretRef: event.target.value || null })} placeholder="password secretRef" />
                    <input type="password" value={jumpSecretDrafts[jumpSecretKey(index, "passphraseSecretRef")] ?? ""} onChange={(event) => setJumpSecretDraft(index, "passphraseSecretRef", event.target.value)} placeholder="passphrase" />
                    <button type="button" className="icon-button" onClick={() => void saveJumpSecret(index, "passphraseSecretRef")} title="保存跳板口令" disabled={secretWriteBusy || !(jumpSecretDrafts[jumpSecretKey(index, "passphraseSecretRef")] ?? "").trim()}>
                      <Lock size={14} />
                    </button>
                    <input value={jump.passphraseSecretRef ?? ""} onChange={(event) => updateJump(index, { passphraseSecretRef: event.target.value || null })} placeholder="passphrase secretRef" />
                    <button type="button" className="icon-button" onClick={() => void deleteJumpSecret(index, "passwordSecretRef")} disabled={!jump.passwordSecretRef} title="删除跳板密码">
                      <X size={14} />
                    </button>
                    <button type="button" className="icon-button" onClick={() => void deleteJumpSecret(index, "passphraseSecretRef")} disabled={!jump.passphraseSecretRef} title="删除跳板口令">
                      <X size={14} />
                    </button>
                  </div>
                  <div className="jump-hop-policy">
                    <select value={jump.hostKeyPolicy ? "custom" : "inherit"} onChange={(event) => updateJump(index, { hostKeyPolicy: event.target.value === "custom" ? createJumpHostKeyPolicy(jump) : null })}>
                      <option value="inherit">继承</option>
                      <option value="custom">自定义</option>
                    </select>
                    {jump.hostKeyPolicy ? (
                      <>
                        <select value={policy.mode} onChange={(event) => updateJumpPolicy(index, { mode: event.target.value as HostKeyPolicy["mode"] })}>
                          <option value="strict">strict</option>
                          <option value="trust-on-first-use">trust-on-first-use</option>
                          <option value="ask-every-time">ask-every-time</option>
                        </select>
                        <input value={policy.alias ?? ""} onChange={(event) => updateJumpPolicy(index, { alias: event.target.value || null })} placeholder="host-key alias" />
                        <select value={policy.trustScope} onChange={(event) => updateJumpPolicy(index, { trustScope: event.target.value as HostKeyPolicy["trustScope"] })}>
                          <option value="profile">profile</option>
                          <option value="project">project</option>
                          <option value="user">user</option>
                        </select>
                        <label className="jump-hop-check">
                          <input type="checkbox" checked={policy.allowRotation} onChange={(event) => updateJumpPolicy(index, { allowRotation: event.target.checked })} />
                          <span>轮换</span>
                        </label>
                        <label className="jump-hop-check">
                          <input type="checkbox" checked={policy.checkIp} onChange={(event) => updateJumpPolicy(index, { checkIp: event.target.checked })} />
                          <span>IP</span>
                        </label>
                      </>
                    ) : null}
                  </div>
                </div>
              );
            })}
            {jumpStatus ? <span className="settings-inline-status">{jumpStatus}</span> : null}
            <button type="button" className="settings-secondary-button jump-add-button" onClick={addJump}>
              <Plus size={14} />
              <span>添加跳板</span>
            </button>
          </div>
        </DialogField>
        <DialogToggleField label="SSH 保活:" checked={ssh.keepaliveEnabled} onChange={(keepaliveEnabled) => updateSsh({ keepaliveEnabled })} />
        {ssh.keepaliveEnabled ? (
          <>
            <DialogField label="探测间隔(s):">
              <input
                type="number"
                min={sshConnectionBounds.keepaliveIntervalSeconds.min}
                max={sshConnectionBounds.keepaliveIntervalSeconds.max}
                value={ssh.keepaliveIntervalSeconds}
                onChange={(event) => updateSsh({ keepaliveIntervalSeconds: Number(event.target.value) })}
              />
            </DialogField>
            <DialogField label="未响应上限 (0=不自动断开):">
              <input
                type="number"
                min={sshConnectionBounds.keepaliveMaxMissed.min}
                max={sshConnectionBounds.keepaliveMaxMissed.max}
                value={ssh.keepaliveMaxMissed}
                onChange={(event) => updateSsh({ keepaliveMaxMissed: Number(event.target.value) })}
              />
            </DialogField>
          </>
        ) : null}
        <DialogField label="TCP KeepAlive:">
          <select
            value={ssh.tcpKeepaliveEnabled === null ? "system" : ssh.tcpKeepaliveEnabled ? "enabled" : "disabled"}
            onChange={(event) => updateSsh({
              tcpKeepaliveEnabled: event.target.value === "system"
                ? null
                : event.target.value === "enabled",
            })}
          >
            <option value="system">系统默认</option>
            <option value="enabled">开启</option>
            <option value="disabled">关闭</option>
          </select>
        </DialogField>
        <DialogToggleField label="自动重连:" checked={ssh.reconnect} onChange={(reconnect) => updateSsh({ reconnect })} />
        <DialogField label="重连延迟(ms):">
          <input
            type="number"
            min={sshConnectionBounds.reconnectDelayMs.min}
            max={sshConnectionBounds.reconnectDelayMs.max}
            step={100}
            disabled={!ssh.reconnect}
            value={ssh.reconnectDelayMs}
            onChange={(event) => updateSsh({ reconnectDelayMs: Number(event.target.value) })}
          />
        </DialogField>
      </>
    );
  }

  if (section === "代理") {
    return <ProxyAdvancedFields proxy={ssh.proxy} onChange={(proxy) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, proxy } })} passwordUpdate={proxyPasswordUpdate} onPasswordUpdateChange={onProxyPasswordUpdateChange} />;
  }

  if (section === "验证") {
    const scanHostKey = async () => {
      setHostKeyStatus("");
      setHostKeyScan(null);
      try {
        const result = await invokeBackend<HostKeyScanResult>("scan_ssh_host_key", { profile: prepareProfile(draft), password: null, passphrase: null });
        setHostKeyScan(result);
      } catch (error) {
        setHostKeyStatus(formatError(error));
      }
    };
    const trustHostKey = async (decision: HostKeyDecisionValue) => {
      if (!hostKeyScan) return;
      setHostKeyStatus("");
      try {
        const trusted = await invokeBackend<TrustedHostKey | null>("trust_scanned_host_key", {
          request: { profile: prepareProfile(draft), observation: hostKeyScan.observation, decision },
        });
        if (trusted) {
          onDraftChange({ ...draft, kind, connection: { ...ssh, kind, trustedHostKeys: [trusted, ...ssh.trustedHostKeys.filter((key) => key.id !== trusted.id)] } });
        }
        setHostKeyStatus(decision === "trust-once" ? "已临时信任，下一次连接有效" : trusted ? `已信任 ${trusted.fingerprintSha256}` : "未写入配置");
      } catch (error) {
        setHostKeyStatus(formatError(error));
      }
    };
    return (
      <>
        <DialogField label="HostKey:">
          <select value={ssh.hostKeyPolicy.mode} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, mode: event.target.value as "strict" | "trust-on-first-use" | "ask-every-time" } } })}>
            <option value="strict">strict</option>
            <option value="trust-on-first-use">trust-on-first-use</option>
            <option value="ask-every-time">ask-every-time</option>
          </select>
        </DialogField>
        <DialogField label="轮换:(R)">
          <select value={ssh.hostKeyPolicy.allowRotation ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, allowRotation: event.target.value === "on" } } })}>
            <option value="off">阻断变更</option>
            <option value="on">允许追加</option>
          </select>
        </DialogField>
        <DialogField label="校验IP:(I)">
          <select value={ssh.hostKeyPolicy.checkIp ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, checkIp: event.target.value === "on" } } })}>
            <option value="off">关闭</option>
            <option value="on">开启</option>
          </select>
        </DialogField>
        <DialogField label="信任域:(S)">
          <select value={ssh.hostKeyPolicy.trustScope} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, hostKeyPolicy: { ...ssh.hostKeyPolicy, trustScope: event.target.value as "profile" | "project" | "user" } } })}>
            <option value="profile">profile</option>
            <option value="project">project</option>
            <option value="user">user</option>
          </select>
        </DialogField>
        <DialogField label="扫描:">
          <div className="inline-actions">
            <button type="button" onClick={() => void scanHostKey()}>扫描 Host Key</button>
            <span>{hostKeyScan ? describeHostKeyEvaluation(hostKeyScan) : hostKeyStatus}</span>
          </div>
        </DialogField>
        {hostKeyScan ? (
          <DialogField label="处理:">
            <div className="inline-actions">
              <button type="button" onClick={() => void trustHostKey("trust-once")}>仅本次</button>
              <button type="button" onClick={() => void trustHostKey("append-to-profile")}>加入 Profile</button>
              <button type="button" onClick={() => void trustHostKey("append-to-project")}>加入 Project</button>
              <button type="button" onClick={() => void trustHostKey("replace-for-profile")}>替换 Profile</button>
            </div>
          </DialogField>
        ) : null}
      </>
    );
  }

  if (section === "代理人") {
    return (
      <>
        <DialogField label="Agent:(A)">
          <select value={ssh.agentPolicy.enabled ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, agentPolicy: { ...ssh.agentPolicy, enabled: event.target.value === "on" } } })}>
            <option value="off">禁用</option>
            <option value="on">启用</option>
          </select>
        </DialogField>
        <DialogField label="Forward:(F)">
          <select value={ssh.agentPolicy.forwarding ? "on" : "off"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, agentPolicy: { ...ssh.agentPolicy, forwarding: event.target.value === "on" } } })}>
            <option value="off">禁用</option>
            <option value="on">启用</option>
          </select>
        </DialogField>
        <DialogField label="Offer:(O)">
          <select value={ssh.agentPolicy.offerMode} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, agentPolicy: { ...ssh.agentPolicy, offerMode: event.target.value as "disabled" | "after-profile-keys" | "before-profile-keys" } } })}>
            <option value="disabled">disabled</option>
            <option value="after-profile-keys">after-profile-keys</option>
            <option value="before-profile-keys">before-profile-keys</option>
          </select>
        </DialogField>
      </>
    );
  }

  if (section === "密码") {
    const deleteSavedSecret = (field: "passwordSecretRef" | "passphraseSecretRef") => {
      const secretRef = ssh[field];
      if (!secretRef) return;
      setSecretStatus("");
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, [field]: null } });
      setSecretStatus("保存 Profile 后清理未引用凭据");
    };
    return (
      <>
        <DialogField label="密码引用:">
          <div className="inline-actions">
            <input value={ssh.passwordSecretRef ?? ""} readOnly placeholder="未保存" />
            <button type="button" onClick={() => void deleteSavedSecret("passwordSecretRef")} disabled={!ssh.passwordSecretRef}>删除</button>
          </div>
        </DialogField>
        <DialogField label="口令引用:">
          <div className="inline-actions">
            <input value={ssh.passphraseSecretRef ?? ""} readOnly placeholder="未保存" />
            <button type="button" onClick={() => void deleteSavedSecret("passphraseSecretRef")} disabled={!ssh.passphraseSecretRef}>删除</button>
          </div>
        </DialogField>
        <DialogField label="状态:">
          <input value={secretStatus} readOnly placeholder="连接弹窗勾选保存后会生成引用" />
        </DialogField>
      </>
    );
  }

  if (section === "公钥") {
    const firstIdentity = ssh.identityRefs[0] ?? createIdentityRef();
    const authOrderValue = ssh.identityPolicy.authOrder.join(">");
    const authOrderIsPreset = SSH_AUTH_ORDER_OPTIONS.some((option) => option === authOrderValue);
    const authOrderOptions: readonly string[] = authOrderIsPreset
      ? SSH_AUTH_ORDER_OPTIONS
      : [authOrderValue, ...SSH_AUTH_ORDER_OPTIONS];
    const updateIdentity = (patch: Partial<IdentityRef>) => {
      const identity = { ...firstIdentity, ...patch };
      onDraftChange({ ...draft, kind, connection: { ...ssh, kind, identityRefs: [identity, ...ssh.identityRefs.slice(1)] } });
    };
    const saveVaultPrivateKey = async () => {
      if (secretWriteBusy || !vaultPrivateKey.trim()) return;
      onSecretWriteStart();
      setVaultBusy(true);
      setVaultStatus("");
      try {
        const response = await invokeBackend<{ secretRef: string }>("save_secret", {
          request: { secretRef: null, secret: vaultPrivateKey },
        });
        onSecretCreated(response.secretRef);
        updateIdentity({ source: "profile-vault", secretRef: response.secretRef, path: null });
        setVaultPrivateKey("");
        setVaultStatus("已保存到系统密钥库");
      } catch (error) {
        setVaultStatus(formatError(error));
      } finally {
        setVaultBusy(false);
        onSecretWriteFinish();
      }
    };
    const deleteVaultPrivateKey = () => {
      if (!firstIdentity.secretRef) return;
      setVaultBusy(true);
      setVaultStatus("");
      updateIdentity({ secretRef: null });
      setVaultStatus("保存 Profile 后清理未引用私钥");
      setVaultBusy(false);
    };
    return (
      <>
        <DialogField label="身份:(I)">
          <select value={ssh.identityPolicy.identitiesOnly ? "only" : "agent"} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, identityPolicy: { ...ssh.identityPolicy, identitiesOnly: event.target.value === "only" } } })}>
            <option value="only">IdentitiesOnly</option>
            <option value="agent">Profile + Agent</option>
          </select>
        </DialogField>
        <DialogField label="顺序:(O)">
          <select value={ssh.identityPolicy.authOrder.join(">")} onChange={(event) => onDraftChange({ ...draft, kind, connection: { ...ssh, kind, identityPolicy: { ...ssh.identityPolicy, authOrder: event.target.value.split(">") as AuthMethod[] } } })}>
            {authOrderOptions.map((option, index) => (
              <option key={option} value={option}>
                {option.replaceAll(">", " > ")}{!authOrderIsPreset && index === 0 ? "（当前配置）" : ""}
              </option>
            ))}
          </select>
        </DialogField>
        <DialogToggleField
          label="记住成功方式:(R)"
          checked={ssh.identityPolicy.recordSuccess}
          onChange={(recordSuccess) => onDraftChange({
            ...draft,
            kind,
            connection: {
              ...ssh,
              kind,
              identityPolicy: {
                ...ssh.identityPolicy,
                recordSuccess,
                lastSuccessful: recordSuccess ? ssh.identityPolicy.lastSuccessful : null,
              },
            },
          })}
        />
        <DialogField label="公钥:(K)">
          <select value={firstIdentity.source} onChange={(event) => updateIdentity({ source: event.target.value as IdentityRef["source"] })}>
            <option>profile-vault</option>
            <option>system-file</option>
            <option>agent</option>
            <option>public-key-only</option>
          </select>
        </DialogField>
        <DialogField label="名称:(N)">
          <input value={firstIdentity.label} onChange={(event) => updateIdentity({ label: event.target.value })} />
        </DialogField>
        <DialogField label="私钥文件:(F)">
          <input value={firstIdentity.path ?? ""} onChange={(event) => updateIdentity({ path: event.target.value || null, source: event.target.value ? "system-file" : firstIdentity.source })} placeholder="~/.ssh/id_ed25519" />
        </DialogField>
        <DialogField label="Vault Ref:">
          <input value={firstIdentity.secretRef ?? ""} readOnly placeholder="保存 profile-vault 私钥后生成" />
        </DialogField>
        {firstIdentity.source === "profile-vault" ? (
          <DialogField label="私钥内容:">
            <textarea value={vaultPrivateKey} onChange={(event) => setVaultPrivateKey(event.target.value)} placeholder="粘贴 OpenSSH 私钥，保存后只保留 secretRef" />
          </DialogField>
        ) : null}
        {firstIdentity.source === "profile-vault" ? (
          <DialogField label="密钥库:">
            <div className="inline-actions">
              <button type="button" onClick={() => void saveVaultPrivateKey()} disabled={vaultBusy || secretWriteBusy || !vaultPrivateKey.trim()}>保存到系统密钥库</button>
              <button type="button" onClick={() => void deleteVaultPrivateKey()} disabled={vaultBusy || !firstIdentity.secretRef}>删除</button>
              <span>{vaultStatus}</span>
            </div>
          </DialogField>
        ) : null}
        <DialogField label="指纹:(P)">
          <input value={firstIdentity.fingerprintSha256 ?? ""} onChange={(event) => updateIdentity({ fingerprintSha256: event.target.value || null })} placeholder="SHA256:..." />
        </DialogField>
      </>
    );
  }

  return null;
}

function ProxyAdvancedFields({
  proxy,
  onChange,
  passwordUpdate,
  onPasswordUpdateChange,
}: {
  proxy: ProxyConfig;
  onChange: (proxy: ProxyConfig) => void;
  passwordUpdate: ProxyPasswordUpdate;
  onPasswordUpdateChange: (update: ProxyPasswordUpdate) => void;
}) {
  const update = (patch: Partial<ProxyConfig>) => onChange({ ...proxy, ...patch });
  const password = passwordUpdate?.action === "set" ? passwordUpdate.password : "";
  const passwordPendingClear = passwordUpdate?.action === "clear";
  return (
    <>
      <DialogToggleField label="启用代理:" checked={proxy.enabled} onChange={(enabled) => update({ enabled })} />
      {proxy.enabled ? (
        <>
          <DialogField label="协议:">
            <select value={proxy.kind} onChange={(event) => update({ kind: event.target.value as ProxyConfig["kind"] })}>
              <option value="socks5">SOCKS5</option>
              <option value="http-connect">HTTP CONNECT</option>
            </select>
          </DialogField>
          <DialogField label="代理主机:">
            <input value={proxy.host} onChange={(event) => update({ host: event.target.value })} />
          </DialogField>
          <DialogField label="代理端口:">
            <input type="number" min={1} max={65535} value={proxy.port} onChange={(event) => update({ port: Number(event.target.value) })} />
          </DialogField>
          <DialogField label="代理用户:">
            <input value={proxy.username} autoComplete="username" onChange={(event) => update({ username: event.target.value })} />
          </DialogField>
          <DialogField label="代理密码:">
            <form className="proxy-password-control" onSubmit={(event) => event.preventDefault()}>
              <input type="text" name="username" autoComplete="username" value={proxy.username} readOnly hidden aria-hidden="true" tabIndex={-1} />
              <input
                type="password"
                name="password"
                autoComplete="new-password"
                value={password}
                placeholder={passwordPendingClear ? "保存后移除" : proxy.passwordSecretRef ? "已安全保存" : "未保存"}
                onChange={(event) => onPasswordUpdateChange(event.target.value ? { action: "set", password: event.target.value } : null)}
              />
              <button
                type="button"
                className="icon-button"
                title="移除已保存的代理密码"
                aria-label="移除已保存的代理密码"
                disabled={passwordPendingClear || (!proxy.passwordSecretRef && passwordUpdate?.action !== "set")}
                onClick={() => onPasswordUpdateChange({ action: "clear" })}
              >
                <X size={14} />
              </button>
            </form>
          </DialogField>
        </>
      ) : null}
    </>
  );
}

function TcpLikeAdvancedFields({
  protocol,
  section,
  draft,
  onDraftChange,
  proxyPasswordUpdate,
  onProxyPasswordUpdateChange,
}: {
  protocol: "Telnet" | "Tcp";
  section: string;
  draft: SessionProfile;
  onDraftChange: (draft: SessionProfile) => void;
  proxyPasswordUpdate: ProxyPasswordUpdate;
  onProxyPasswordUpdateChange: (update: ProxyPasswordUpdate) => void;
}) {
  const kind = protocol === "Telnet" ? "telnet" : "tcp";
  const tcp = draft.connection.kind === kind ? draft.connection : createTcpConnection(kind);

  if (section === "连接") {
    const updateTcp = (patch: Partial<typeof tcp>) => onDraftChange({
      ...draft,
      kind,
      connection: { ...tcp, ...patch, kind },
    });
    return (
      <>
        <DialogField label="主机:(H)">
          <input value={tcp.host} onChange={(event) => updateTcp({ host: event.target.value })} />
        </DialogField>
        <DialogField label="端口:(P)">
          <input type="number" min={1} max={65535} value={tcp.port} onChange={(event) => updateTcp({ port: Number(event.target.value) })} />
        </DialogField>
        <DialogToggleField label="自动重连:" checked={tcp.reconnect} onChange={(reconnect) => updateTcp({ reconnect })} />
        <DialogField label="重连延迟(ms):">
          <input
            type="number"
            min={tcpConnectionBounds.reconnectDelayMs.min}
            max={tcpConnectionBounds.reconnectDelayMs.max}
            step={100}
            disabled={!tcp.reconnect}
            value={tcp.reconnectDelayMs}
            onChange={(event) => updateTcp({ reconnectDelayMs: Number(event.target.value) })}
          />
        </DialogField>
        <DialogToggleField label="TCP KeepAlive:" checked={tcp.keepaliveEnabled} onChange={(keepaliveEnabled) => updateTcp({ keepaliveEnabled })} />
        {tcp.keepaliveEnabled ? (
          <>
            <DialogField label="空闲时间(s):">
              <input
                type="number"
                min={tcpConnectionBounds.keepaliveIdleSeconds.min}
                max={tcpConnectionBounds.keepaliveIdleSeconds.max}
                value={tcp.keepaliveIdleSeconds}
                onChange={(event) => updateTcp({ keepaliveIdleSeconds: Number(event.target.value) })}
              />
            </DialogField>
            <DialogField label="探测间隔(s):">
              <input
                type="number"
                min={tcpConnectionBounds.keepaliveIntervalSeconds.min}
                max={tcpConnectionBounds.keepaliveIntervalSeconds.max}
                value={tcp.keepaliveIntervalSeconds}
                onChange={(event) => updateTcp({ keepaliveIntervalSeconds: Number(event.target.value) })}
              />
            </DialogField>
            <DialogField label="失败次数:">
              <input
                type="number"
                min={tcpConnectionBounds.keepaliveRetries.min}
                max={tcpConnectionBounds.keepaliveRetries.max}
                value={tcp.keepaliveRetries}
                onChange={(event) => updateTcp({ keepaliveRetries: Number(event.target.value) })}
              />
            </DialogField>
          </>
        ) : null}
        {protocol === "Telnet" ? (
          <>
            <DialogToggleField label="BINARY:" checked={tcp.telnetBinary} onChange={(telnetBinary) => updateTcp({ telnetBinary })} />
            <DialogToggleField label="NAWS:" checked={tcp.telnetNaws} onChange={(telnetNaws) => updateTcp({ telnetNaws })} />
          </>
        ) : null}
      </>
    );
  }

  return <ProxyAdvancedFields proxy={tcp.proxy} onChange={(proxy) => onDraftChange({ ...draft, kind, connection: { ...tcp, kind, proxy } })} passwordUpdate={proxyPasswordUpdate} onPasswordUpdateChange={onProxyPasswordUpdateChange} />;
}

function SerialAdvancedFields({
  draft,
  serialPorts,
  onDraftChange,
}: {
  draft: SessionProfile;
  serialPorts: string[];
  onDraftChange: (draft: SessionProfile) => void;
}) {
  const serial = draft.connection.kind === "serial" ? draft.connection : createSerialConnection();
  const update = (patch: Partial<ReturnType<typeof createSerialConnection>>) => onDraftChange({ ...draft, kind: "serial", connection: { ...serial, ...patch } });

  return (
    <>
      <DialogField label="串口:(S)">
        <select value={serial.port} onChange={(event) => update({ port: event.target.value })}>
          {serialPortOptions(serial.port, serialPorts).map((option) => (
            <option key={option || "blank"} value={option}>
              {option || "选择串口"}
            </option>
          ))}
        </select>
      </DialogField>
      <DialogField label="波特率:(B)">
        <input type="number" value={serial.baudRate} onChange={(event) => update({ baudRate: Number(event.target.value) })} />
      </DialogField>
      <DialogField label="数据位:(D)">
        <select value={serial.dataBits} onChange={(event) => update({ dataBits: Number(event.target.value) })}>
          <option value={5}>5</option>
          <option value={6}>6</option>
          <option value={7}>7</option>
          <option value={8}>8</option>
        </select>
      </DialogField>
      <DialogField label="停止位:(S)">
        <select value={serial.stopBits} onChange={(event) => update({ stopBits: Number(event.target.value) })}>
          <option value={1}>1</option>
          <option value={2}>2</option>
        </select>
      </DialogField>
      <DialogField label="校验:(P)">
        <select value={serial.parity} onChange={(event) => update({ parity: event.target.value })}>
          <option>none</option>
          <option>odd</option>
          <option>even</option>
        </select>
      </DialogField>
      <DialogField label="流控:(F)">
        <select value={serial.flowControl} onChange={(event) => update({ flowControl: event.target.value })}>
          <option>none</option>
          <option>software</option>
          <option>hardware</option>
        </select>
      </DialogField>
      <DialogField label="DTR:(D)">
        <select value={serial.dtr ? "on" : "off"} onChange={(event) => update({ dtr: event.target.value === "on" })}>
          <option value="off">关闭</option>
          <option value="on">开启</option>
        </select>
      </DialogField>
      <DialogField label="RTS:(R)">
        <select value={serial.rts ? "on" : "off"} onChange={(event) => update({ rts: event.target.value === "on" })}>
          <option value="off">关闭</option>
          <option value="on">开启</option>
        </select>
      </DialogField>
      <DialogToggleField label="自动重连:" checked={serial.reconnect} onChange={(reconnect) => update({ reconnect })} />
      <DialogField label="重连延迟(ms):">
        <input
          type="number"
          min={serialConnectionBounds.reconnectDelayMs.min}
          max={serialConnectionBounds.reconnectDelayMs.max}
          step={100}
          disabled={!serial.reconnect}
          value={serial.reconnectDelayMs}
          onChange={(event) => update({ reconnectDelayMs: Number(event.target.value) })}
        />
      </DialogField>
      <DialogToggleField label="接收空闲超时:" checked={serial.receiveIdleTimeoutEnabled} onChange={(receiveIdleTimeoutEnabled) => update({ receiveIdleTimeoutEnabled })} />
      {serial.receiveIdleTimeoutEnabled ? (
        <DialogField label="空闲上限(s):">
          <input
            type="number"
            min={serialConnectionBounds.receiveIdleTimeoutSeconds.min}
            max={serialConnectionBounds.receiveIdleTimeoutSeconds.max}
            value={serial.receiveIdleTimeoutSeconds}
            onChange={(event) => update({ receiveIdleTimeoutSeconds: Number(event.target.value) })}
          />
        </DialogField>
      ) : null}
    </>
  );
}

function DialogFrame({
  title,
  className,
  onClose,
  closeDisabled = false,
  children,
}: {
  title: string;
  className: string;
  onClose: () => void;
  closeDisabled?: boolean;
  children: ReactNode;
}) {
  return (
    <div className="dialog-backdrop">
      <section className={`wind-dialog ${className}`}>
        <header className="dialog-title">
          <span className="app-icon" />
          <strong>{title}</strong>
          <button onClick={onClose} disabled={closeDisabled}><X size={22} /></button>
        </header>
        {children}
      </section>
    </div>
  );
}

function DialogField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="dialog-field">
      <span>{label}</span>
      {children}
    </label>
  );
}

function DialogToggleField({ label, checked, onChange }: { label: string; checked: boolean; onChange: (value: boolean) => void }) {
  return (
    <label className="dialog-field dialog-toggle-field">
      <span>{label}</span>
      <button type="button" className={checked ? "switch-toggle on" : "switch-toggle"} onClick={() => onChange(!checked)} aria-pressed={checked}>
        <span />
      </button>
    </label>
  );
}

function formatError(error: unknown) {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}
