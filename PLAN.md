## Background
The keys and UI/UX have gotten a bit clunky. The list of commands at the bottom of the screen is bloated and only fully shows in full-screen mode on larger screens. The UI is ok but I think we may be able to clean it up a little more.

## Goal
Create a clean, usable UI/UX that shows the relevant shortcut keys even when on small screen.

## Role
You are a task manager agent. You do not implement tasks yourself -- you delegate each step to a subagent, verify its output against the acceptance criteria below, and decide whether to proceed, retry, or escalate.

## Plan
The following steps must be completed in order:
1. Analyze current key shortcuts and make sure they make sense across all section, sessions, jobs, and tmux
2. Analyze the UI/UX and look for ways to improve it.
3. Create a plan from the findings
4. Any other steps you think are required

> If a step's output does not meet the acceptance criteria, retry once with corrective 
> guidance. If it fails again, stop and report the failure before proceeding.

## Acceptance Criteria
<!-- How do you know each step succeeded? List checkable conditions. -->
- [x] A clear plan on what needs to be refactored and how it should be improved — `UI_REFACTOR_PLAN.md`
- [x] A clean UI/UX with refined key shortcuts across the full app that fit the task — implemented; shortcuts fit at 60 columns, `? help` and `q quit` never drop


## Constraints
- **Do not make assumptions.** If something is ambiguous or missing, ask before proceeding.
- **Batch clarifying questions.** If you need to ask, collect all questions and ask at once 
  before starting any steps.
- **Do not proceed past a failed step** without explicit instruction.
- <!-- Any domain-specific constraints: don't modify production, don't delete data, etc. -->
