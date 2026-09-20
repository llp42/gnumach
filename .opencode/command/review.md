---
description: Review pending changes before commit with rust-reviewer.
agent: rust-reviewer
---

Review the code before commit.  This is diagnostic only: do not modify
anything.

Scope: $ARGUMENTS

If the scope above is empty, review the pending commit — staged,
unstaged and untracked changes (`git status --short`, `git diff HEAD`).

Follow your full reviewer protocol and produce the structured report.
