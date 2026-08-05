import { useEffect, useRef } from "react";
import { ArrowDown, ArrowUp, Plus, Trash2 } from "lucide-react";
import {
  appendShellArgument,
  MAX_SHELL_ARGUMENTS,
  moveShellArgument,
  removeShellArgument,
  updateShellArgument,
} from "./shell-argument-state";

export default function ShellArgumentsEditor({
  args,
  onChange,
}: {
  args: string[];
  onChange: (args: string[]) => void;
}) {
  const inputs = useRef<Array<HTMLInputElement | null>>([]);
  const pendingFocus = useRef<number | null>(null);

  useEffect(() => {
    const index = pendingFocus.current;
    if (index === null || index >= args.length) return;
    pendingFocus.current = null;
    inputs.current[index]?.focus();
  }, [args.length, args]);

  function addArgument() {
    if (args.length >= MAX_SHELL_ARGUMENTS) return;
    pendingFocus.current = args.length;
    onChange(appendShellArgument(args));
  }

  function removeArgument(index: number) {
    const next = removeShellArgument(args, index);
    pendingFocus.current = next.length ? Math.min(index, next.length - 1) : null;
    onChange(next);
  }

  function moveArgument(index: number, offset: -1 | 1) {
    pendingFocus.current = index + offset;
    onChange(moveShellArgument(args, index, offset));
  }

  return (
    <div className="shell-arguments-editor" role="group" aria-label="Shell 参数列表">
      {args.length ? (
        <div className="shell-argument-list">
          {args.map((argument, index) => (
            <div className="shell-argument-row" key={index}>
              <span className="shell-argument-index" aria-hidden="true">{index + 1}</span>
              <input
                ref={(node) => { inputs.current[index] = node; }}
                aria-label={`Shell 参数 ${index + 1}`}
                autoComplete="off"
                value={argument}
                onChange={(event) => onChange(updateShellArgument(args, index, event.target.value))}
              />
              <button
                type="button"
                className="icon-button"
                title={`上移参数 ${index + 1}`}
                aria-label={`上移 Shell 参数 ${index + 1}`}
                disabled={index === 0}
                onClick={() => moveArgument(index, -1)}
              >
                <ArrowUp size={14} />
              </button>
              <button
                type="button"
                className="icon-button"
                title={`下移参数 ${index + 1}`}
                aria-label={`下移 Shell 参数 ${index + 1}`}
                disabled={index === args.length - 1}
                onClick={() => moveArgument(index, 1)}
              >
                <ArrowDown size={14} />
              </button>
              <button
                type="button"
                className="icon-button"
                title={`删除参数 ${index + 1}`}
                aria-label={`删除 Shell 参数 ${index + 1}`}
                onClick={() => removeArgument(index)}
              >
                <Trash2 size={14} />
              </button>
            </div>
          ))}
        </div>
      ) : null}
      <div className="shell-argument-toolbar">
        <button type="button" onClick={addArgument} disabled={args.length >= MAX_SHELL_ARGUMENTS}>
          <Plus size={14} />
          添加参数
        </button>
        <span>{args.length}/{MAX_SHELL_ARGUMENTS}</span>
      </div>
    </div>
  );
}
