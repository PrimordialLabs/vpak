# vpak

Intent-based software packaging.

A traditional installer ships a fixed artifact and a fixed procedure. A vpak
ships the **intent** behind a system, a **reference** realization of it, and
the **constraints** it must honor. An agent then works out how to realize that
intent at a **target** the installer chooses. Like origami: one sheet of
instructions, folded one recorded step at a time into structure.

A vpak is half the packer's and half the installer's:

| Packer ships                         | Installer brings                              |
|--------------------------------------|-----------------------------------------------|
| intent: what the system is and does  | target: where and how to install it           |
| reference: IaC, CI/CD, source        | constraints: what must hold at their site     |
| constraints: what the packer requires| a working directory where discovery happens   |

The target is the installer's primary input. With no target, the install aims
for the closest analogy to the reference. With one, the shape can change
entirely: a GCP Cloud Run reference becomes ECS on AWS, or a container on a
laptop. Everything downstream is derived from the target and the reference
together.

## Install

Straight from GitHub, no clone needed (requires a Rust toolchain):

```
cargo install --git https://github.com/primordiallabs/vpak vpak
```

Or from a checkout of this repo:

```
cargo install --path crates/vpak
```

Either way the `vpak` binary lands in `~/.cargo/bin`. Builds on macOS, Linux
and Windows.

Default runner is headless Claude Code (`claude -p`). Any process that can be
told what to do can be a runner; see "Runners".

## Quickstart

Pack the example (skip the agentic inspection pass):

```
vpak add examples/hello-service --no-inspect
vpak inspect hello-service.vpak
```

Print the bootstrap an agent would receive for a given target:

```
vpak install hello-service.vpak --target examples/targets/aws-ecs.md --print-bootstrap
```

Run an install with Claude Code as the runner, interactively:

```
vpak install hello-service.vpak --target "Install to my AWS, us-east-1, ECS Fargate. Same creds as my other projects." --dest ./hello-aws
```

Run the whole loop without any agent, using the built-in test fixture as the
runner (it records facts, constraints, journal entries and advances phases):

```
VPAK_SHELL_RUNNER_CMD="vpak __fixture agent" vpak install hello-service.vpak --runner shell --mode auto
vpak --workdir .vpak/<id> status
```

## What an install leaves behind

```
.vpak/<install-id>/
├── install.toml          # id, archive digest, phase, status, runner, dest
├── vpak/                 # the unpacked archive (read-only)
├── target/               # target.md prose + target.toml structured
├── constraints/          # resolved.toml with provenance, conflicts.toml
├── survey/facts.toml     # read-only discoveries, each sourced to a command
├── plan/                 # mapping.md, steps/NNNN-<slug>.toml
├── bootstrap/
│   ├── journal/          # append-only NNNN-<phase>-<slug>.md
│   ├── current.md        # compacted view, regenerated from the journal
│   └── compiled.md       # what the runner receives: seed + current + guide
├── questions/            # questions to the human, answered or open
├── fleet/collective.toml # present when a vflt collective was engaged
└── audit.jsonl           # one line per primitive call and runner event
```

The installation itself (generated IaC, code, deployed resources) is built in
`--dest`, not here. The working directory is where the agent figures out what
is necessary.

## Phases

`locate → survey → constrain → plan → decide → execute → verify → done`.
Loop-backs are allowed and journaled. Survey is always read-only; mutating
actions are plan steps that run only in execute, under the runner policy.
Running `vpak install` again, or `vpak resume <workdir>`, continues from the
recorded phase.

## Primitives

The agent drives the install through `vpak` subcommands: `status`,
`phase set`, `journal append|current`, `target show|set|note`,
`survey record|list`, `constraint add|list|conflicts|resolve`,
`plan map|add|list|mark`, `ask`, `question list|answer`, `policy`,
`fleet bootstrap`. Every call appends to `audit.jsonl`. The trail is the same
whichever agent is driving.

Constraint precedence: installer beats packer. Discovered facts never override
an explicit constraint; a disagreement is recorded as a conflict and raised.

## Modes and questions

Interactive mode (default on a terminal) prompts the installer when the
bootstrap needs a decision; answers become constraints or target fields.
Auto mode records questions instead and stops with exit code 3 and status
`needs_human`. Answer with `vpak question answer <id> --text ...` and
`vpak resume <workdir>`.

## Runners

- `claude` (default): `claude -p --permission-mode auto` with the policy's
  tool allowlist and its classifier rules passed as Claude Code's `autoMode`
  settings. Headless runs cannot prompt; denied actions become questions.
- `shell`: any command template, split into argv without a shell, prompt on
  stdin. Set `VPAK_SHELL_RUNNER_CMD` or configure it.
- Custom: `[runners.<name>]` in the config file (`~/.config/vpak/config.toml`
  on Linux, the platform config dir elsewhere, or `VPAK_CONFIG`).

`vpak runners` lists what this machine has.

## Skill mode

`skills/vpak/SKILL.md` is a Claude Code skill (and `plugin.json` makes the
repo a plugin). In skill mode the interactive session is the runner:
`vpak install <file> --print-bootstrap` emits the compiled bootstrap and the
session drives the same primitives. The audit trail is identical.

## Fleet handoff

Large installs hand off to [vflt](https://github.com/primordiallabs/vflt): `vpak fleet bootstrap` creates a
collective at `<dest>/.vflt`, installs a supervisor profile whose standing
instructions are the compiled bootstrap, and seeds one item per plan step.
vflt is reached as a separate binary (`vflt` on PATH or `VFLT_BIN`).

## Packing

```
vpak add <dir> [--directives <file|text>] [--ignore <glob|file>]... [--out <file>] [--no-inspect]
```

With a `vpak.toml` at its root, `<dir>` is a structured vpak source tree
(`intent/`, `reference/`, `constraints/`, `targets/`, `bootstrap/`). Without
one, the whole tree becomes `reference/` and the manifest, intent and policy
are synthesized. `.gitignore` and `.vpakignore` are honored. A secret scan
fails the pack unless findings are waived with `--allow-secret <path>`. The
inspection pass runs the configured runner to write `reference/REFERENCE.md`
and list gaps as questions.

## Cross-platform

macOS, Linux and Windows. No shell is used to launch runners, no `/dev/tty`
(prompts go through the console abstraction), no symlinks in the layout, and
tests use fixtures built into the binary. CI runs on all three.

## Docs

- [docs/DESIGN.md](docs/DESIGN.md) — the design: two halves, archive format,
  install workdir, phases, primitives, runner, skill mode, fleet handoff.

## License

Apache-2.0. See [LICENSE](LICENSE).
