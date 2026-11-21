import type { MessageKey } from "../i18n";

const PROBLEMS = {
  "addon-needs-update": "addonCommand.problem.needsUpdate",
  "addon-cancelled": "addonCommand.problem.cancelled",
  "addon-timeout": "addonCommand.problem.timeout",
  "addon-unavailable": "addonCommand.problem.unavailable",
  "addon-invalid-response": "addonCommand.problem.invalidResponse",
  "addon-invalid-selection": "addonCommand.problem.invalidSelection",
  "addon-unsupported": "addonCommand.problem.unsupported",
  "addon-command-failed": "addonCommand.problem.failed",
  "addon-compile-failed": "addonCommand.problem.compileFailed",
  "addon-does-not-fit": "addonCommand.problem.doesNotFit",
  "addon-too-many-proposals": "addonCommand.problem.tooMany",
  proposal_changed: "addonCommand.problem.stale",
} as const satisfies Record<string, MessageKey>;

/** Map a closed Rust code inside an error's details to a localized status; never show raw text. */
export function addonProblem(details: string): MessageKey {
  if (/stale|revision_conflict|changed since/i.test(details)) return "addonCommand.problem.stale";
  const code = Object.keys(PROBLEMS).find((key) => details.includes(key)) as keyof typeof PROBLEMS | undefined;
  return code ? PROBLEMS[code] : "addonCommand.problem.failed";
}
