export const MAX_SHELL_ARGUMENTS = 128;
export const MAX_SHELL_ARGUMENT_CHARACTERS = 4_096;

export function appendShellArgument(args: readonly string[]): string[] {
  if (args.length >= MAX_SHELL_ARGUMENTS) return [...args];
  return [...args, ""];
}

export function updateShellArgument(args: readonly string[], index: number, value: string): string[] {
  if (!Number.isInteger(index) || index < 0 || index >= args.length) return [...args];
  const bounded = Array.from(value).slice(0, MAX_SHELL_ARGUMENT_CHARACTERS).join("");
  return args.map((argument, argumentIndex) => argumentIndex === index ? bounded : argument);
}

export function removeShellArgument(args: readonly string[], index: number): string[] {
  if (!Number.isInteger(index) || index < 0 || index >= args.length) return [...args];
  return args.filter((_, argumentIndex) => argumentIndex !== index);
}

export function moveShellArgument(args: readonly string[], index: number, offset: -1 | 1): string[] {
  const target = index + offset;
  if (!Number.isInteger(index) || index < 0 || index >= args.length || target < 0 || target >= args.length) {
    return [...args];
  }
  const next = [...args];
  [next[index], next[target]] = [next[target], next[index]];
  return next;
}
