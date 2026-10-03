# Phase: constrain

Install of `{{name}}`. Working directory: `{{workdir}}`. Destination: `{{dest}}`.

Answer: what must hold, and where do the sources disagree?

1. `vpak constraint list` shows installer and packer constraints already
   recorded. `vpak survey list` shows discovered facts.
2. Turn survey facts that limit the plan into constraints:
   `vpak constraint add --source discovered --key <k> --value <v> --text "..."`.
   Examples: the only region available, an org policy, a quota, a mandated
   secrets mechanism.
3. `vpak constraint conflicts`. For each open conflict, either resolve it with
   `vpak constraint resolve <id> --keep <constraint-id> --reason "..."` when
   the precedence rule decides it (installer beats packer beats discovered), or
   raise it with `vpak ask` when a human must choose.
4. If the installer's target is still inferred, write the inferred target now:
   `vpak target set` for provider, region, access, and mapping, with
   `--key origin --value inferred`, and explain the inference in the journal.
5. `vpak journal append --phase constrain --title "constrained" --body "..."`.
6. `vpak phase set plan --reason "constrained"`.
