# Phase: plan

Install of `{{name}}`. Working directory: `{{workdir}}`. Destination: `{{dest}}`.

Answer: how does the reference map onto the target, and what are the steps?

1. For every reference component, record the mapping:
   `vpak plan map --from <ref-component> --to <target-component> --why "..."`.
   A component that is dropped maps to `none` with a reason. A new component
   the target requires maps from `none`.
2. Write the steps in order with `vpak plan add --step "..." [--mutating]
   [--cmd "..."] [--kind code|deploy|test|review|survey]`. Anything that
   creates, changes or deletes something in the target or writes to the
   destination is `--mutating`. Keep steps small enough to mark done or failed
   individually.
3. Steps that generate files put them under the destination `{{dest}}`, never
   the working directory.
4. Steps must honor every constraint in `vpak constraint list`. If a step
   cannot, raise it with `vpak ask` instead of weakening the constraint.
5. `vpak journal append --phase plan --title "planned" --body "..."` with the
   shape of the realization and the main trade-offs.
6. `vpak phase set decide --reason "planned"`.
