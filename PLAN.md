## Background
With a Claude Pro/Max account you have only so much usage every 5 hours. Because of the limit if you are working on something that uses your full usage before it completes you have to either pay for extra usage credit or wait until the usage resets. If you wait until the usage resets you have to be around in reinitiate the continuation of the task(s).

## Goal
Update this ccsm tool so that it can dispatch/manage/schedule Claude Code session and watches account usages, pauses active session when usage is at 95% and continues the session(s) when the account usage resets.

## References
@./claude-usage is a tool I've created that gives the current account usage and reset time. Command `claude-usage --format json`

## Role
You are a task manager agent. You do not implement tasks yourself -- you delegate each step to a subagent, verify its output against the acceptance criteria below, and decide whether to proceed, retry, or escalate.

## Plan
The following steps must be completed in order:
1. Figure out the best way to manage one or more sessions
2. Figure out how to start and restart a Claude Code session with a prompt
3. Build a management interface into ccsm
4. Any other steps you think are required

> If a step's output does not meet the acceptance criteria, retry once with corrective
> guidance. If it fails again, stop and report the failure before proceeding.

## Acceptance Criteria
<!-- How do you know each step succeeded? List checkable conditions. -->
- [ ] ccsm should be able to start a new Claude Code session with a defined prompt
- [ ] It should be able to continue an existing session when usage resets
- [ ] It should be able to pause/stop an existing session when usage gets to a defined threshold, such as 95%
- [ ] Support the user providing/picking an existing Claude code session to restart, monitor, and manage

## Output
A updated version of the ccsm app

## Constraints
- **Do not make assumptions.** If something is ambiguous or missing, ask before proceeding.
- **Batch clarifying questions.** If you need to ask, collect all questions and ask at once
  before starting any steps.
- **Do not proceed past a failed step** without explicit instruction.
- <!-- Any domain-specific constraints: don't modify production, don't delete data, etc. -->
