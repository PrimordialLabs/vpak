# Phase: survey

Install of `{{name}}`. Working directory: `{{workdir}}`. Destination: `{{dest}}`.

Answer: what does the target environment look like? Read-only, always.

1. Read `vpak target show`, `vpak journal current`, and
   `vpak/reference/REFERENCE.md`. List the reference components you must find
   an analogue for.
2. For each component, discover the target-side facts you need with read-only
   commands only (describe, list, get, show, sts get-caller-identity, config
   list). Record each with `vpak survey record --key <k> --value <v> --source
   "<command>"`, marking account ids, principals and emails
   `--sensitivity high`.
3. Fill in the target as it becomes known: `vpak target set --key provider
   --value ...`, `--key region`, `--key access.credentials`, `--key
   access.secrets`, and `--key mapping.<ref-component> --value
   <target-component>` for any mapping that is already obvious.
4. Survey only what the target and intent require. If the target is vague,
   survey more widely and say so in the journal. If you cannot proceed without
   a human decision (which account, which region, which credential), use
   `vpak ask`.
5. `vpak journal append --phase survey --title "surveyed" --body "..."` with
   what you found, what you could not find, and what remains unknown.
6. `vpak phase set constrain --reason "surveyed"`.

Never run a command that creates, changes or deletes anything in the target.
