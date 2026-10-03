mod add;
mod ask;
mod fixture;
mod install;
mod prims;

use anyhow::Result;

use crate::cli::{Cli, Cmd};

pub fn dispatch(cli: Cli) -> Result<i32> {
    let json = cli.json;
    let workdir = cli.workdir.clone();
    match cli.cmd {
        Cmd::Add(a) => add::run(a, json),
        Cmd::Inspect { file } => add::inspect(&file, json),
        Cmd::Install(a) => install::run(a, workdir.as_deref(), json),
        Cmd::Resume {
            workdir_path,
            max_phases,
        } => install::resume(&workdir_path, max_phases, json),
        Cmd::Ask(a) => ask::run(a, workdir.as_deref(), json),
        Cmd::Runners => prims::runners(json),
        Cmd::Fixture { args } => fixture::run(&args),
        other => prims::run(other, workdir.as_deref(), json),
    }
}
