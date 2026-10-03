---
name: vpak
description: Install or pack a vpak (intent-based software package) from this session. Use when the user mentions a .vpak file, asks to install a vpak to a target (their AWS, GCP, a Docker host, a laptop), or asks to pack a directory as a vpak.
argument-hint: "[install <file.vpak> | add <dir>]"
allowed-tools: "Bash(vpak *), Read, Grep, Glob"
---

# vpak

You are the runner for a vpak install. The `vpak` CLI is the deterministic,
audited substrate; you drive it with its primitives. Everything you learn or
decide goes through a primitive so the trail in the working directory is
complete. Survey is always read-only.

## Install

1. Ask the user for the target if they have not given one: where and how
   should this be installed? The target is the installer's primary input; with
   none, aim for the closest analogy to the reference and say so.
2. Create or resume the install and read the compiled bootstrap:

   ```
   vpak install <file.vpak> --target "<their words or a file>" [--constraints "..."] [--dest <dir>] --print-bootstrap
   ```

   The output is your standing instructions: seed, current state, and the
   primitive guide. Note the `workdir` it prints; either `cd` there or pass
   `--workdir <dir>` to every call, or export `VPAK_WORKDIR`.
3. Work the phases in order, using the primitives named in the guide for each
   phase, and move with `vpak phase set <next> --reason "..."`. Journal at
   least once per phase.
4. When a human must decide, use `vpak ask --question "..." [--options a,b]
   [--as constraint|target|note] [--key k]`. In this interactive session it
   will prompt the user; put their answer to use and continue.
5. Mutating actions only in the execute phase, each one a plan step marked
   done, failed or skipped with a result.
6. Finish with `vpak phase set done --reason "verified"` after the verify
   phase, or loop back to plan when verification fails.

Rules:

- Never run a command that creates, changes or deletes anything in the target
  environment before the execute phase.
- Facts that identify accounts or principals get `--sensitivity high`.
- The installation goes in the destination directory, not the working
  directory.
- If the plan is large or needs several roles, `vpak fleet bootstrap` hands
  it to a vflt collective; see the decide phase prompt in `vpak status`.

## Pack

```
vpak add <dir> --directives "<what this system is and must keep doing>" [--ignore <glob>]... [--out <file>]
```

`vpak add` stages the tree, scans for secrets, and (unless `--no-inspect`)
runs an inspection pass in a headless runner. If no runner is available, run
the inspection yourself: `vpak add ... --no-inspect`, then unpack the archive
to a scratch directory, write `reference/REFERENCE.md`, draft `intent/`, list
gaps in `provenance/inspection.md`, and re-pack with `vpak add <scratch>`.

## Reading the trail

- `vpak status` — phase, counts, open questions.
- `vpak journal current` — the compacted state.
- `bootstrap/journal/` — every entry, append-only.
- `audit.jsonl` — every primitive call.
