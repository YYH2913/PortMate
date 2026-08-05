import { describe, expect, it } from "vitest";
import {
  appendShellArgument,
  MAX_SHELL_ARGUMENT_CHARACTERS,
  MAX_SHELL_ARGUMENTS,
  moveShellArgument,
  removeShellArgument,
  updateShellArgument,
} from "./shell-argument-state";

describe("shell argument editing", () => {
  it("preserves exact argument boundaries and significant whitespace", () => {
    let args = appendShellArgument([]);
    args = updateShellArgument(args, 0, "-c");
    args = appendShellArgument(args);
    args = updateShellArgument(args, 1, " printf '%s\\n' 'hello world' ");
    args = appendShellArgument(args);

    expect(args).toEqual(["-c", " printf '%s\\n' 'hello world' ", ""]);
  });

  it("moves and removes one argument without reparsing its contents", () => {
    const original = ["--flag", "two words", ""];
    const moved = moveShellArgument(original, 1, -1);

    expect(moved).toEqual(["two words", "--flag", ""]);
    expect(removeShellArgument(moved, 1)).toEqual(["two words", ""]);
    expect(original).toEqual(["--flag", "two words", ""]);
  });

  it("enforces bounded Unicode input and argument count", () => {
    const full = Array.from({ length: MAX_SHELL_ARGUMENTS }, (_, index) => String(index));
    const bounded = updateShellArgument([""], 0, "😀".repeat(MAX_SHELL_ARGUMENT_CHARACTERS + 1));

    expect(appendShellArgument(full)).toEqual(full);
    expect(Array.from(bounded[0])).toHaveLength(MAX_SHELL_ARGUMENT_CHARACTERS);
  });

  it("leaves invalid edits unchanged", () => {
    expect(updateShellArgument(["one"], -1, "other")).toEqual(["one"]);
    expect(removeShellArgument(["one"], 1)).toEqual(["one"]);
    expect(moveShellArgument(["one"], 0, -1)).toEqual(["one"]);
  });
});
