# Phase: decide

Install of `{{name}}`. Working directory: `{{workdir}}`. Destination: `{{dest}}`.

Answer: is this small enough to do inline, or should a fleet do it?

1. `vpak plan list`. Count steps and independent components.
2. Fleet it when the plan has many independent components, when steps need
   different roles (survey, code, review, test, deploy) that benefit from
   separate agents, or when the installer asked for a fleet. Otherwise do it
   inline.
3. To fleet: `vpak fleet bootstrap`. This creates a vflt collective at the
   destination, installs a supervisor profile carrying this bootstrap, and
   seeds one item per plan step. If it reports that `vflt` is not installed,
   fall back to inline unless the installer required a fleet; then `vpak ask`.
4. `vpak journal append --phase decide --title "decided" --body "..."` with the
   decision and why.
5. `vpak phase set execute --reason "decided"`.
