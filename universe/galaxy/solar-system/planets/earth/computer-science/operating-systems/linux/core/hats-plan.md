# Hats plan

## Source

- https://github.com/planesailingio/hats
- commit 8a005784f1741de33dffc367259b6631b785c0e5
- MIT, Rust, 62 files, workspace member `cli`
- re-clone: `git clone --depth 1 https://github.com/planesailingio/hats.git /tmp/hats`

## What it is

Per-terminal identity switching (git, ssh, gpg, aws, azure, kube, k9s,
terraform, vscode, github, coder) through environment variables. The
shell hook is a zsh template: `cli/shell/hats.zsh.j2`.

## Goal

First class part of the console, rebranded (he dislikes the name).
Nushell only. Uses his own git, ssh and style systems. Colors come
from the style roles (text, dim, error, bg).

## Steps (guess, not measured: 10 to 15 rounds)

1. Rebrand and strip: rename binary, crate, strings, docs. Drop the
   hats he does not need. 2 to 3 rounds.
2. Nushell hook instead of zsh. A child process cannot change the
   parent shell, so the hook emits Nushell code the shell loads.
   3 to 4 rounds.
3. Use console git-tui, ssh-manager and git-secrets: read how they
   are driven, then replace `hat/git.rs` and `hat/ssh.rs`. 4 to 6
   rounds, the big unknown.
4. Wire into console: manifest build step, fetch list, a key in the
   shell visual layer (`pane-keys.rkt`), like brochure. 1 to 2
   rounds.
5. Keep the upstream version in an `UPSTREAM` file and in
   `upstream-version.txt`.

## Open question

Only worth it if he works as several identities (client or separate
GitHub accounts). With one account it adds little.
