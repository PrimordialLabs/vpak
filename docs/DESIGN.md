# vpak design

vpak is intent-based software packaging. A traditional installer ships a
fixed artifact and a fixed procedure. A vpak ships the *intent* behind a
system, a *reference* realization of it, and the *constraints* it must honor,
and then an agent works out how to realize that intent at a *target* chosen
by the installer.

The analogy is origami: one sheet of instructions, folded step by step into
structure. Every fold is recorded.

## 1. Two halves

A vpak is half the packer's and half the installer's.

| Half      | Component    | Who writes it | Lives where                                  |
|-----------|--------------|---------------|----------------------------------------------|
| Packer    | intent       | packer        | archive `intent/`                            |
| Packer    | reference    | packer        | archive `reference/`                         |
| Packer    | constraints  | packer        | archive `constraints/`                       |
| Packer    | provenance   | `vpak add`    | archive `provenance/`                        |
| Installer | target       | installer     | install workdir `target/`                    |
| Installer | constraints  | installer     | install workdir `constraints/`               |
| Installer | working dir  | bootstrap     | install workdir (survey, journal, plan, audit)|

The **installation itself** (generated IaC, code, deployed resources) lives
in a separate destination, not in the working dir. The working dir is where
the agent figures out what is necessary. The destination is where it builds.

### Intent

An ordered series of prompts describing what the system is and does, in the
packer's words. Example: "this vpak builds a payments service that handles
user authentication and user payments against a separate inventory system."
Intent is the part that must survive any change of target.

### Reference

A snapshot of a working realization: IaC, CI/CD, source, docs. The reference
is the closest thing to a traditional package, but it is *evidence*, not a
procedure. The installer's target decides how much of it survives.

### Constraints

Hard limits. Two sources, with precedence:

1. **installer** constraints (flag, file, stdin, or interactive answers)
2. **packer** constraints (shipped in the archive)
3. **discovered** facts (from survey)

Installer beats packer. Discovered facts never override an explicit
constraint; when they disagree, the bootstrap records a *conflict* and raises
it to the user rather than deciding silently.

### Target

The target is the installer's primary input and the most important one. It
answers *where and how do I want this installed*.

- **Default target** is the closest analogy to the reference. A reference on
  GCP Cloud Run + Cloud SQL, installed with no target given, aims for the
  same shape in whatever account the survey finds.
- **An explicit target can mutate the shape entirely.** "Install to my AWS,
  use the same creds I use for other AWS projects, request secrets through
  service accounts via corporate access control, run on ECS not Lambda."
- The target is resolved once, early, and recorded. Everything downstream
  (survey scope, plan, mapping) is derived from the target and the reference
  together.

Target has a prose form (always) and a structured form (filled as it becomes
known):

```toml
# target/target.toml  (structured, grows during bootstrap)
[target]
summary   = "Corporate AWS, us-east-1, ECS Fargate, RDS Postgres"
origin    = "explicit"          # explicit | inferred
provider  = "aws"
region    = "us-east-1"

[target.access]
credentials = "existing-profile"  # how to authenticate
secrets     = "service-account-via-corporate-access-control"

[target.mapping]            # reference component -> target component
"cloud-run"  = "ecs-fargate"
"cloud-sql"  = "rds-postgres"
"cloud-build" = "github-actions"
```

```markdown
# target/target.md  (prose, the installer's own words plus interactive answers)
Install this to my AWS. Same creds I use for other AWS projects. Request
secrets via service accounts through corporate access control. ...
```

### Working directory

Where discovery and reasoning live. Append-only journal, survey facts, plan
steps, audit log, and the grown bootstrap. See section 4.

## 2. Archive format

A `.vpak` is a gzip-compressed tar with a fixed top-level layout.

```
<name>.vpak
├── vpak.toml                 # manifest (required)
├── intent/                   # ordered prompts: 00-overview.md, 10-auth.md, ...
├── reference/                # snapshot of the reference realization
│   └── REFERENCE.md          # written at pack time by the inspection pass
├── constraints/              # packer constraints: *.md prose, constraints.toml typed
├── targets/                  # optional: target templates the packer suggests,
│                             #   and realized targets from prior installs (re-pack)
├── bootstrap/
│   ├── seed.md               # seed bootstrap prompt compiled at pack time
│   └── policy.toml           # suggested runner policy (tool allowlist, classifier hints)
└── provenance/
    ├── inspection.md         # what the inspection pass looked at and concluded
    ├── manifest.sha256       # digest of every file in the archive
    └── pack.log              # audit of the pack run
```

Manifest:

```toml
format   = 1
name     = "payments-service"
version  = "0.3.0"
created  = "2026-10-02T21:00:00Z"
packer   = "vmantese@gmail.com"
summary  = "Payments service with auth and inventory integration"

[reference]
root     = "reference/"
kind     = "gcp"                 # free-form hint of the reference environment
entry    = ["terraform/", ".github/workflows/", "Dockerfile"]

[intent]
order    = ["00-overview.md", "10-auth.md", "20-payments.md"]

[bootstrap]
seed     = "bootstrap/seed.md"
policy   = "bootstrap/policy.toml"

[[targets]]                       # optional, repeatable
name     = "origin"
summary  = "GCP Cloud Run + Cloud SQL, as shipped"
path     = "targets/origin.toml"
```

Re-pack after a successful install appends a realized target to `targets/`
and a provenance note. A vpak therefore accumulates known realizations over
its life. "Closest analogy to the reference" generalizes to "closest analogy
to any known realization."

## 3. Packing (`vpak add`)

`vpak add <dir> [--directives <file>] [--ignore <file|glob>...] [--out <file>] [--no-inspect]`

Packing is the install in reverse: look at a context and make sure enough of
it is captured for a stranger to realize the intent elsewhere.

Deterministic steps (always):

1. Walk `<dir>` honoring `.gitignore`, `.vpakignore`, and `--ignore`.
2. Secret scan. Known key shapes (AWS keys, GCP service account JSON, private
   key blocks, generic high-entropy tokens next to `secret|token|password`)
   fail the pack unless explicitly waived with `--allow-secret <path>`.
3. Build `reference/` snapshot, `vpak.toml`, `provenance/manifest.sha256`.

Agentic step (default on; `--no-inspect` skips it):

4. The inspection pass runs the configured runner with the pack prompt. It
   reads the directives and the tree, writes `reference/REFERENCE.md`,
   drafts or extends `intent/` from the directives, proposes
   `constraints/` and `bootstrap/policy.toml`, and **raises gaps to the
   user** (missing CI definition, undocumented external dependency, secrets
   referenced but not described). Gaps are questions, not guesses.

If no runner is available, `vpak add` scans for installed runners, reports
what it found, and offers `--no-inspect`.

## 4. Installing (`vpak install`)

```
vpak install <file.vpak>
    [--target <file|prompt|->]        # installer's target; '-' reads stdin
    [--constraints <file|prompt|->]   # installer's constraints
    [--workdir <dir>]                 # default: ./.vpak/<install-id>/
    [--dest <dir>]                    # default: ./<name>/
    [--runner <name>]                 # default: claude
    [--mode interactive|auto]         # default: interactive when on a TTY
    [--print-bootstrap]               # emit the bootstrap and exit (skill mode)
```

Running `vpak install` again against an existing workdir resumes from the
recorded current state. Installs are identified by an install id stored in
`install.toml`.

### Workdir layout

```
.vpak/<install-id>/
├── install.toml          # id, vpak digest, phase, status, runner, dest, timestamps
├── vpak/                 # the unpacked archive (read-only during install)
├── target/
│   ├── target.md         # prose
│   └── target.toml       # structured, grows
├── constraints/
│   ├── installer.md      # from flag/file/stdin/interactive
│   ├── resolved.toml     # merged view with provenance per constraint
│   └── conflicts.toml    # unresolved disagreements, each needs a human decision
├── survey/
│   └── facts.toml        # discovered facts: key, value, source command, sensitivity
├── plan/
│   ├── mapping.md        # reference component -> target component, with rationale
│   └── steps/            # NNNN-<slug>.toml: step, mutating?, command, status, result
├── bootstrap/
│   ├── journal/          # append-only: NNNN-<phase>-<slug>.md
│   └── current.md        # compacted view regenerated from the journal
├── fleet/                # present if the install engaged vflt
│   └── collective.toml   # path to the collective root, runner, supervisor profile name
├── audit.jsonl           # one line per action: ts, actor, kind, args, outcome
└── questions/            # open questions to the human, one file each
```

### Phases

The bootstrap is a state machine. Each phase writes at least one journal
entry and one audit line, so the trail is obvious and a restart has a fixed
place to resume from.

| Phase     | Question it answers                              | Writes                              |
|-----------|--------------------------------------------------|-------------------------------------|
| locate    | Where am I running? Who am I? What tools exist?  | journal, survey facts (host, tools) |
| survey    | What does the target environment look like?      | survey facts (read-only only)       |
| constrain | What must hold? What conflicts?                  | constraints/resolved, conflicts     |
| plan      | How does the reference map onto the target?      | plan/mapping, plan/steps            |
| decide    | Is this small enough to do inline, or fleet it?  | journal; fleet/ if vflt engaged     |
| execute   | Do the plan steps, each one logged.              | steps results, dest contents        |
| verify    | Does the realization satisfy the intent?         | journal, verification report        |

Phases can loop back (verify finds a gap, returns to plan). The journal
records every transition.

### Survey posture

Survey is **always read-only**. It does not survey beyond what the target and
intent require, but it surveys aggressively when it has been given too little
to go on. Mutating actions are never performed during survey. They are
written as plan steps marked `mutating = true` and executed only in the
execute phase under the runner policy.

### Interactive back-and-forth and the policy

In interactive mode the bootstrap asks the installer what they are
comfortable with before anything mutating happens. Answers become installer
constraints and target fields, and also drive the **runner policy**: the
allowlist, denylist and classifier guidance the runner is launched with. When
the runner's classifier blocks an action it bubbles to the user, as fleet
operations expect. In auto mode, blocked actions are recorded in
`questions/` and the phase exits with `needs_human`.

### Growing bootstrap

The bootstrap prompt is compiled from the archive's `seed.md` plus the
journal. It is **append-only**: every phase appends entries, and `current.md`
is a compacted view regenerated from them. A restart reads `install.toml`
for the phase and `current.md` for the state. The journal is the audit trail;
`current.md` is what the runner actually receives.

## 5. Primitives

The vpak CLI is the deterministic substrate. The runner (headless Claude, an
interactive Claude Code session via the skill, or any other agent) drives the
install by calling these primitives. Every call appends to `audit.jsonl`.
That is what makes the process auditable regardless of which agent is driving.

```
vpak status                                   # phase, counts, open questions
vpak phase set <phase> [--reason <text>]      # transition, journaled
vpak journal append --phase <p> --title <t> [--body-file <f> | -]
vpak journal current                          # print current.md
vpak target show | set --key <k> --value <v> | note <text>
vpak survey record --key <k> --value <v> --source <cmd> [--sensitivity low|med|high]
vpak survey list
vpak constraint add --source installer|discovered --text <t> [--key <k>]
vpak constraint list | conflicts
vpak plan map --from <ref-component> --to <target-component> --why <text>
vpak plan add --step <text> [--mutating] [--cmd <cmd>]
vpak plan list
vpak plan mark <n> --status done|failed|skipped --result <text>
vpak ask --question <text> [--options a,b,c]   # interactive: prompts on /dev/tty
                                               # auto: records a question, exits 3
vpak fleet bootstrap [--collective <dir>]      # decide phase: hand off to vflt
```

## 6. Runner

A **runner** is the thing that executes an agent turn. The default runner is
headless Claude Code, but a runner is configurable and headless Claude is
just the default.

```rust
pub trait Runner {
    fn name(&self) -> &str;
    fn available(&self) -> Result<Availability>;      // installed? authenticated?
    fn run(&self, req: RunRequest) -> Result<RunOutcome>;
}

pub struct RunRequest {
    pub system_prompt: String,     // compiled bootstrap (current.md) + primitive guide
    pub prompt: String,            // the phase instruction
    pub cwd: PathBuf,              // the workdir
    pub extra_dirs: Vec<PathBuf>,  // dest and any surveyed paths
    pub policy: Policy,            // permission mode, allow/deny tools, classifier rules
    pub budget: Budget,            // max turns, wall clock
    pub session: Option<String>,   // resume id if the runner supports it
}
```

`claude` runner: `claude -p --permission-mode <mode>
--append-system-prompt-file <compiled> --allowedTools ... --add-dir <workdir>
--add-dir <dest> --settings '<json>' --permission-prompts none --max-turns <n>
--output-format stream-json`. The policy's classifier rules are passed as the
`autoMode` block (`environment`, `allow`, `soft_deny`, `hard_deny`) inside the
inline `--settings` JSON. Stream events are mirrored into `audit.jsonl`; the
final `result` event supplies `session_id`, cost, and `permission_denials`,
which become `questions/` entries.

```toml
# bootstrap/policy.toml (shipped by the packer, refined at install)
permission_mode  = "auto"
allowed_tools    = ["Read", "Grep", "Glob", "Bash(vpak *)", "Bash(aws * describe*)", "Bash(aws sts *)"]
disallowed_tools = ["Bash(rm -rf *)"]

[classifier]
environment = ["Survey phase: the target account is read-only until the execute phase."]
allow       = ["Read-only describe, list, and get calls against the target provider."]
soft_deny   = ["Any create, update, delete, or apply call before the execute phase."]
hard_deny   = ["Printing credential material.", "Modifying IAM outside the plan."]
```

Other runners register under `[runners.<name>]` in `~/.config/vpak/config.toml`
with a command template. A `shell` runner exists for tests.

Every runner launch gets `VPAK_WORKDIR`, `VPAK_BIN` (the exact binary that is
driving the install) and a `PATH` with that binary's directory prepended, so
the agent's `vpak ...` calls hit the same build whether or not vpak is
installed system-wide.

## 7. Skill mode

The repo ships `skills/vpak/SKILL.md`, a Claude Code skill, and a
`plugin.json` so it can also be installed as a plugin. In skill mode the
user's interactive session is the runner. `vpak install --print-bootstrap`
emits the compiled bootstrap, and the skill teaches the session to drive the
same primitives. The audit trail is identical in both modes because the
primitives are the same.

## 8. Fleet handoff (decide phase)

Small installs loop inline: the bootstrap drives phases itself. Larger ones
engage vflt. The decide phase considers plan size, number of independent
components, and whether the installer asked for a fleet.

Default handoff:

1. `vflt collective init <dest>/.vflt` (or reuse one named in config).
2. Create a **supervisor** profile whose standing instructions are the
   current compiled bootstrap. The supervisor runs on a cycle.
3. Seed items from `plan/steps/`, one item per step or component, with the
   stage set from the step kind.
4. Record the collective path in `fleet/` and loop: the bootstrap now
   observes the collective and re-enters verify when the board drains.

vpak references vflt by shelling out to the `vflt` binary. The repos stay
independent. If `vflt` is not installed the decide phase reports that and
falls back to inline unless the installer required a fleet.

## 9. Examples

`examples/hello-service/` is a packable source tree: a small HTTP service,
Dockerfile, GitHub Actions CI, Terraform for GCP Cloud Run, intent prompts
and packer constraints. `examples/targets/` holds installer target prompts
(`local-docker.md`, `aws-ecs.md`, `gcp-cloud-run.md`). Local Docker is one
target among several, useful because it exercises the whole loop without
cloud credentials. It is not the assumed target.

## 10. Cross-platform

Both tools target macOS, Linux and Windows. Rules that follow from that:

- **No `/dev/tty`.** Interactive prompts go through a cross-platform console
  abstraction (the `console` / `dialoguer` crates, which open `CONIN$` on
  Windows). Fall back to stdin when no console is available.
- **No symlinks in data layouts.** Where the layout pointed at another
  directory, write a small toml file with a `path` key instead.
- **No `sh -c`.** Runner command templates are split into argv (shlex-style)
  and spawned directly. Anyone who wants a shell names it in the template.
- **Identity** comes from `USER`, then `USERNAME`, then the `whoami` crate.
- **Paths** are `PathBuf` end to end. Archive entries always use `/`.
  Windows verbatim prefixes are stripped before writing paths into files.
- **Config and data dirs** come from the `dirs` crate, never a hand-built
  `~/.config`.
- **Executable lookup** goes through `which`, which honours `PATHEXT`.
- **Atomic claims** use `create_new`, which is atomic on all three platforms.
- **Budgets** kill the child process on wall-clock expiry; no signals.
- **Line endings**: write `\n`, read tolerant of `\r\n`.
- **Test fixtures** are small Rust binaries, not shell scripts. Any fixture
  that must be a script is `#[cfg(unix)]` gated and has a Windows twin or a
  documented skip.
- **CI** runs `cargo build`, `cargo test` and `cargo clippy` on an
  ubuntu / macos / windows matrix (`.github/workflows/ci.yml`).

## 11. Non-goals for the first cut

- No network protocol of its own. The runner and vflt are both local
  processes.
- No secret storage. The target describes *how* secrets are obtained; the
  realization obtains them at the destination.
- No automatic re-pack. `vpak add --from-install <workdir>` is explicit.
