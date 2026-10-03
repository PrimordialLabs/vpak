# vpak primitives

Run every primitive from the working directory, or pass `--workdir <dir>`.
`VPAK_WORKDIR` is set for you. Every call appends to `audit.jsonl`.

| Phase     | Record with                                                                                  | Finish with                      |
|-----------|----------------------------------------------------------------------------------------------|----------------------------------|
| locate    | `vpak survey record --key host.os --value ... --source "uname -s"`, tool availability facts    | `vpak phase set survey`          |
| survey    | `vpak survey record ...` (read-only commands only), `vpak target set --key ... --value ...`    | `vpak phase set constrain`       |
| constrain | `vpak constraint add --source discovered --key ... --value ... --text ...`, review conflicts   | `vpak phase set plan`            |
| plan      | `vpak plan map --from <ref> --to <target> --why ...`, `vpak plan add --step ... [--mutating]`  | `vpak phase set decide`          |
| decide    | `vpak fleet bootstrap` if the plan is large, otherwise note the inline decision in the journal | `vpak phase set execute`         |
| execute   | run each step, then `vpak plan mark <n> --status done|failed|skipped --result ...`             | `vpak phase set verify`          |
| verify    | `vpak journal append --phase verify --title "verification" --body ...`                        | `vpak phase set done` or back to `plan` |

Always:

- `vpak status` to see where you are.
- `vpak journal append --phase <phase> --title <title> --body <text>` at least once per phase, with what you learned and why you decided what you decided.
- `vpak journal current` to read the compacted state after a restart.
- `vpak target show` and `vpak constraint list` before planning.
- `vpak ask --question "..." [--options a,b,c] [--as constraint|target|note] [--key k]` when a human must decide. In interactive mode this prompts the installer. In auto mode it records the question and exits with code 3; stop the phase and leave the phase unchanged.
- `vpak phase set <phase> --reason "..."` to move. Loop-backs are allowed and journaled.

Rules:

- Survey commands must be read-only: describe, list, get, show, cat, ls. No create, update, delete, apply, or write to the target environment before the execute phase.
- Record sensitivity on facts that identify accounts or principals: `--sensitivity high`. They are redacted in the compacted view.
- Keep the destination directory for the installation itself. Keep the working directory for reasoning.
