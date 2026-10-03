# Phase: locate

Install of `{{name}}`. Working directory: `{{workdir}}`. Destination: `{{dest}}`.

Answer: where am I running, who am I, and what tools exist here?

1. Record host facts with `vpak survey record`: OS, architecture, shell, user,
   working directory, destination directory, and whether the destination exists
   and is empty. Source each fact with the command you ran.
2. Record tool availability as facts: `docker`, `terraform`, `aws`, `gcloud`,
   `kubectl`, `git`, `go`, `node`, `python3`, and anything the reference uses.
   Key them `tool.<name>` with value `present <version>` or `absent`.
3. Read `vpak target show`. If the installer gave a target, note in the journal
   what it implies for the survey. If not, state that the target will be
   inferred as the closest analogy to the reference and record
   `vpak target set --key origin --value inferred`.
4. `vpak journal append --phase locate --title "located" --body "..."` with a
   short summary of where you are and what the next phase needs to find out.
5. `vpak phase set survey --reason "located"`.

Do not touch the target environment yet.
