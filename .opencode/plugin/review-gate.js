// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

// Injects the project's "review before done" rule into every request, so
// the rule survives a context that has drifted away from AGENTS.md.  The
// contract itself lives in AGENTS.md; this is the mechanical reminder.

const RULE =
  "GNU Mach completion rule: before reporting a coding task complete, " +
  "run the `rust-reviewer` subagent over the pending diff (or the " +
  "`/review` command) and resolve every BLOCKER it reports.  Do not " +
  "call the task done until the review is READY or READY WITH NITS, or " +
  "the user waives the review.";

export const ReviewGate = async () => ({
  "experimental.chat.system.transform": async (_input, output) => {
    output.system.push(RULE);
  },
});
