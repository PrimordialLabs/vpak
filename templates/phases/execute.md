# Phase: execute

Install of `{{name}}`. Working directory: `{{workdir}}`. Destination: `{{dest}}`.

Do the plan, one logged step at a time.

1. `vpak plan list`. Work pending steps in order.
2. For each step: do it, then `vpak plan mark <n> --status done --result
   "..."`. On failure, `--status failed --result "<error>"` and decide whether
   to continue, loop back to plan with `vpak phase set plan --reason "..."`, or
   `vpak ask`.
3. Mutating steps run only now, and only within the runner policy. If the
   policy blocks a step, do not work around it. Record the block as a question
   with `vpak ask` and skip the step with `--status skipped`.
4. Files generated for the installation go under `{{dest}}`.
5. If a fleet was engaged, your job here is to observe the collective, not to
   do the items yourself: check its board, note progress in the journal, and
   move to verify when the board drains.
6. `vpak journal append --phase execute --title "executed" --body "..."`.
7. `vpak phase set verify --reason "executed"`.
