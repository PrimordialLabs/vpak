//! Command-line surface. See docs/DESIGN.md sections 3 to 5.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "vpak",
    version,
    about = "Intent-based software packaging: pack a reference realization, install it to any target."
)]
pub struct Cli {
    /// Install working directory (default: VPAK_WORKDIR, else the nearest ancestor holding install.toml).
    #[arg(long, global = true, env = "VPAK_WORKDIR")]
    pub workdir: Option<PathBuf>,

    /// Machine-readable output.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Pack a directory into a .vpak archive.
    Add(AddArgs),
    /// Show an archive's manifest and contents.
    Inspect { file: PathBuf },
    /// Install a .vpak to a target, or resume an install in --workdir.
    Install(InstallArgs),
    /// Resume an install from its working directory.
    Resume {
        workdir_path: PathBuf,
        /// Stop after this many runner turns.
        #[arg(long, default_value_t = 20)]
        max_phases: u32,
    },
    /// Phase, counts, open questions.
    Status,
    /// Phase transitions.
    Phase {
        #[command(subcommand)]
        cmd: PhaseCmd,
    },
    /// The append-only journal and its compacted view.
    Journal {
        #[command(subcommand)]
        cmd: JournalCmd,
    },
    /// The installer's target: where and how to install.
    Target {
        #[command(subcommand)]
        cmd: TargetCmd,
    },
    /// Discovered facts (read-only findings).
    Survey {
        #[command(subcommand)]
        cmd: SurveyCmd,
    },
    /// Constraints with provenance and precedence.
    Constraint {
        #[command(subcommand)]
        cmd: ConstraintCmd,
    },
    /// Component mapping and ordered steps.
    Plan {
        #[command(subcommand)]
        cmd: PlanCmd,
    },
    /// Ask the installer a question (interactive) or record it (auto, exit 3).
    Ask(AskArgs),
    /// Questions recorded for a human.
    Question {
        #[command(subcommand)]
        cmd: QuestionCmd,
    },
    /// The runner policy for this install.
    Policy {
        #[command(subcommand)]
        cmd: PolicyCmd,
    },
    /// Hand off to a vflt collective.
    Fleet {
        #[command(subcommand)]
        cmd: FleetCmd,
    },
    /// List known runners and whether they are available.
    Runners,
    /// Test fixtures (hidden): `__fixture vflt ...` and `__fixture agent`.
    #[command(name = "__fixture", hide = true)]
    Fixture {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// Source directory. With a vpak.toml it is a structured vpak source tree; otherwise all of it becomes the reference.
    pub dir: PathBuf,
    /// Packer directives: a file, literal prose, or '-' for stdin.
    #[arg(long)]
    pub directives: Option<String>,
    /// Glob to ignore, or path to an ignore file. Repeatable. .gitignore and .vpakignore are always honored.
    #[arg(long = "ignore")]
    pub ignore: Vec<String>,
    /// Output archive (default: <name>.vpak in the current directory).
    #[arg(long, short)]
    pub out: Option<PathBuf>,
    /// Skip the agentic inspection pass.
    #[arg(long)]
    pub no_inspect: bool,
    /// Waive secret-scan findings under this source-relative path. Repeatable.
    #[arg(long = "allow-secret")]
    pub allow_secret: Vec<PathBuf>,
    /// Runner for the inspection pass (default: config default_runner, else claude).
    #[arg(long)]
    pub runner: Option<String>,
    /// Packer identity recorded in the manifest.
    #[arg(long)]
    pub packer: Option<String>,
}

#[derive(Args, Debug)]
pub struct InstallArgs {
    pub file: PathBuf,
    /// Installer's target: a file, literal prose, or '-' for stdin.
    #[arg(long)]
    pub target: Option<String>,
    /// Installer's constraints: a file, literal prose, or '-' for stdin.
    #[arg(long)]
    pub constraints: Option<String>,
    /// Where the installation itself is built (default: ./<name>/).
    #[arg(long)]
    pub dest: Option<PathBuf>,
    /// Runner name (default: config default_runner, else claude).
    #[arg(long)]
    pub runner: Option<String>,
    /// interactive (default on a terminal) or auto.
    #[arg(long)]
    pub mode: Option<String>,
    /// Compile and print the bootstrap, then exit (skill mode).
    #[arg(long)]
    pub print_bootstrap: bool,
    /// Stop after this many runner turns.
    #[arg(long, default_value_t = 20)]
    pub max_phases: u32,
}

#[derive(Subcommand, Debug)]
pub enum PhaseCmd {
    /// Move to a phase (loop-backs allowed). `done` completes the install.
    Set {
        phase: String,
        #[arg(long)]
        reason: Option<String>,
        /// Allow `done` while plan steps are still pending
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum JournalCmd {
    Append {
        #[arg(long)]
        phase: Option<String>,
        #[arg(long)]
        title: String,
        /// Body text (literal). Mutually exclusive with --body-file.
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Body from a file, or '-' for stdin.
        #[arg(long)]
        body_file: Option<String>,
    },
    /// Print the compacted current view.
    Current,
    /// List entries.
    List,
}

#[derive(Subcommand, Debug)]
pub enum TargetCmd {
    Show,
    Set {
        #[arg(long)]
        key: String,
        #[arg(long)]
        value: String,
    },
    Note {
        text: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum SurveyCmd {
    Record {
        #[arg(long)]
        key: String,
        #[arg(long)]
        value: String,
        /// The command or observation that produced the value.
        #[arg(long)]
        source: String,
        #[arg(long, default_value = "low")]
        sensitivity: String,
    },
    List,
}

#[derive(Subcommand, Debug)]
pub enum ConstraintCmd {
    Add {
        /// installer or discovered (packer constraints come from the archive).
        #[arg(long)]
        source: String,
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        value: Option<String>,
        /// Force a conflict with this constraint id.
        #[arg(long)]
        conflicts_with: Option<String>,
    },
    List,
    Conflicts,
    Resolve {
        id: String,
        #[arg(long)]
        keep: String,
        #[arg(long)]
        reason: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum PlanCmd {
    Map {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        why: String,
    },
    Add {
        #[arg(long)]
        step: String,
        #[arg(long)]
        mutating: bool,
        #[arg(long)]
        cmd: Option<String>,
        /// survey, code, review, test or deploy (default: deploy if mutating, else code).
        #[arg(long)]
        kind: Option<String>,
    },
    List,
    Mark {
        n: u32,
        #[arg(long)]
        status: String,
        #[arg(long, default_value = "")]
        result: String,
    },
}

#[derive(Args, Debug)]
pub struct AskArgs {
    #[arg(long)]
    pub question: String,
    /// Comma-separated choices.
    #[arg(long, value_delimiter = ',')]
    pub options: Vec<String>,
    /// How to apply the answer: constraint, target, or note.
    #[arg(long = "as", default_value = "note")]
    pub apply_as: String,
    /// Key for a constraint or target field.
    #[arg(long)]
    pub key: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum QuestionCmd {
    List,
    Answer {
        id: String,
        #[arg(long)]
        text: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum PolicyCmd {
    Show,
    Allow { tool: String },
    Deny { tool: String },
    Mode { mode: String },
}

#[derive(Subcommand, Debug)]
pub enum FleetCmd {
    Bootstrap {
        /// Collective directory (default: <dest>/.vflt).
        #[arg(long)]
        collective: Option<PathBuf>,
    },
}
