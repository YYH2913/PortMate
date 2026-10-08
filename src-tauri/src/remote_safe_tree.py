"""POSIX descriptor-relative tree operations, invoked over the current SSH lease.

Never fall back to path-based SFTP recursion when these primitives are absent.
"""
import json
import os
import stat
import sys

MAX_ENTRIES = 20000
MAX_FILES = 1000
MAX_DEPTH = 128


def identity(metadata):
    return metadata.st_dev, metadata.st_ino, stat.S_IFMT(metadata.st_mode)


def open_parent(path):
    path = os.path.abspath(path)
    parts = path.split("/")
    if any(p in (".", "..") for p in parts) or path == "/":
        raise ValueError("unsafe root path")
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    ancestors = []
    handles = [fd]
    try:
        for part in parts[1:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            ancestors.append((fd, part, identity(os.fstat(child)), child))
            handles.append(child)
            fd = child
        return fd, parts[-1], path, ancestors, handles
    except BaseException:
        for handle in reversed(handles):
            os.close(handle)
        raise


def run(action, paths, boundary=None):
    if os.name != "posix" or os.open not in os.supports_dir_fd or os.listdir not in os.supports_fd:
        raise RuntimeError("remote OS cannot provide descriptor-relative safe tree operations")
    plan = {"directories": [], "files": [], "skipped": []}
    count = 0
    bindings = []

    def visit(parent, name, source, relative, depth):
        nonlocal count
        count += 1
        if count > MAX_ENTRIES or depth > MAX_DEPTH:
            raise ValueError("remote tree entry/depth limit exceeded")
        before = os.stat(name, dir_fd=parent, follow_symlinks=False)
        if stat.S_ISDIR(before.st_mode):
            fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
            bindings.append((parent, name, identity(before), fd))
            if identity(os.fstat(fd)) != identity(before):
                raise RuntimeError("remote directory changed while opening")
            if action == "plan":
                plan["directories"].append(relative)
            # Bound directory listing memory as well as the recursive count.
            with os.scandir(fd) as entries:
                children = []
                for entry in entries:
                    children.append(entry.name)
                    if len(children) + count > MAX_ENTRIES:
                        raise ValueError("remote tree entry limit exceeded")
            children.sort()
            if boundary is not None:
                boundary(source)
            for child in children:
                visit(fd, child, source + "/" + child, relative + "/" + child, depth + 1)
            if identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) != identity(before):
                raise RuntimeError("remote directory changed during tree operation")
            if action == "delete":
                # rmdir never follows a substituted final symlink. Child
                # unlinks above were relative to the held original directory.
                os.rmdir(name, dir_fd=parent)
        elif action == "delete":
            os.unlink(name, dir_fd=parent)
        elif stat.S_ISREG(before.st_mode):
            if len(plan["files"]) >= MAX_FILES:
                raise ValueError("remote batch file limit exceeded")
            plan["files"].append({"source": source, "relative": relative, "size": before.st_size})
        else:
            plan["skipped"].append(source + " (symbolic link or special file)")

    parents = []
    try:
        for path in paths:
            parent, name, absolute, ancestors, handles = open_parent(path)
            bindings.extend(ancestors)
            parents.extend(handles)
            visit(parent, name, absolute, name, 0)
        if action == "plan":
            for parent, name, expected, fd in bindings:
                if identity(os.stat(name, dir_fd=parent, follow_symlinks=False)) != expected:
                    raise RuntimeError("remote directory changed before batch publication")
        return plan
    finally:
        for fd in set(parents + [item[3] for item in bindings]):
            os.close(fd)


if __name__ == "__main__":
    try:
        action, paths = json.loads(sys.argv[1])
        if action not in ("delete", "plan"):
            raise ValueError("invalid tree operation")
        print(json.dumps(run(action, paths), ensure_ascii=True))
    except Exception as error:
        print("safe remote tree operation failed: " + str(error), file=sys.stderr)
        sys.exit(1)
