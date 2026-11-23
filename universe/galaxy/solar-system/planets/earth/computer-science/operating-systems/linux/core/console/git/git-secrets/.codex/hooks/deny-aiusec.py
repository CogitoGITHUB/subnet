#!/usr/bin/env python3
import json
import os
import re
import sys

COMMAND_KEYS = {"cmd", "command", "Command", "CommandLine", "commandLine"}
CTG_COMMAND = re.compile(
    r"(?:^|[;&|()\n\r])\s*(?:builtin\s+|command\s+|exec\s+|sudo\s+)?(?:[./\w-]*/)?aiusecx?\b"
)
COTT_SUFFIX = re.compile(r"\.cott\.[^./\\]+$")
TOKEN_RE = re.compile(r"""[^\s"'`|;&<>()]+""")
PATH_KEY_HINT = re.compile(r"path|file|dir|target|absolute", re.IGNORECASE)


def repo_root(hint=None):
    d = os.path.abspath(hint) if hint and os.path.isdir(hint) else os.getcwd()
    while True:
        if os.path.isdir(os.path.join(d, ".git")) or os.path.isdir(os.path.join(d, ".git-secrets")):
            return d
        parent = os.path.dirname(d)
        if parent == d:
            return os.path.abspath(hint or os.getcwd())
        d = parent


def is_sensitive(path, root):
    if not isinstance(path, str):
        return False
    path = path.strip().strip("'\"")
    if not path or len(path) > 4096 or "\n" in path:
        return False
    normalized = path.replace("\\", "/")
    if ".git-secrets" in [p for p in normalized.split("/") if p]:
        return True
    if COTT_SUFFIX.search(normalized):
        return True
    abs_path = path if os.path.isabs(path) else os.path.join(root, path)
    try:
        return os.path.exists(abs_path + ".cott.age")
    except OSError:
        return False


def find_command(value):
    if isinstance(value, dict):
        for key, nested in value.items():
            if key in COMMAND_KEYS and isinstance(nested, str):
                return nested
        for nested in value.values():
            command = find_command(nested)
            if command:
                return command
    elif isinstance(value, list):
        for nested in value:
            command = find_command(nested)
            if command:
                return command
    return ""


def find_sensitive_in_command(command, root):
    if not command:
        return None
    for token in TOKEN_RE.findall(command):
        if is_sensitive(token, root):
            return token
    return None


def find_sensitive(value, root, path_context=False):
    """Only treat strings as path candidates when reached through a
    path/file/dir-hinted key (e.g. apply_patch's file paths), so patch
    content that merely mentions .git-secrets or *.cott.* in prose is never
    mistaken for a path to protect."""
    if isinstance(value, str):
        return value if path_context and is_sensitive(value, root) else None
    if isinstance(value, dict):
        for key, nested in value.items():
            if key in COMMAND_KEYS:
                continue
            hit = find_sensitive(nested, root, bool(PATH_KEY_HINT.search(str(key))))
            if hit:
                return hit
    elif isinstance(value, list):
        for nested in value:
            hit = find_sensitive(nested, root, path_context)
            if hit:
                return hit
    return None


def deny(reason):
    print(
        json.dumps(
            {
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": reason,
                }
            }
        )
    )


REASON = (
    "Blocked by git-secrets workspace policy: '{}' is a protected secret "
    "(inside .git-secrets/, matches *.cott.*, or is a decrypted file with a "
    ".cott.age counterpart). AI agents must not view or edit it."
)


def main():
    try:
        data = json.loads(sys.stdin.read() or "{}")
    except json.JSONDecodeError:
        return 0

    command = find_command(data)
    if command and CTG_COMMAND.search(command):
        deny(
            "AI agents are forbidden from executing aiusec or aiusecx commands in this workspace."
        )
        return 0

    root = repo_root(data.get("cwd"))

    if command:
        hit = find_sensitive_in_command(command, root)
        if hit:
            deny(REASON.format(hit))
            return 0

    hit = find_sensitive(data, root)
    if hit:
        deny(REASON.format(hit))

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
