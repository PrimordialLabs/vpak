//! Compile the system prompt a runner receives: the archive's seed, the
//! compacted current state, and the primitive guide.

use std::collections::HashMap;

use anyhow::Result;

use crate::install::{journal, Phase, Workdir};
use crate::templates;
use crate::util;

/// Compile and write `bootstrap/compiled.md`; return its text.
pub fn compile(wd: &Workdir) -> Result<String> {
    let manifest = wd.manifest()?;
    let seed_path = wd.vpak_dir().join(&manifest.bootstrap.seed);
    let seed = if seed_path.is_file() {
        util::read_to_string(&seed_path)?
    } else {
        crate::archive::compile_seed(&wd.vpak_dir(), &manifest)?
    };
    let current = journal::regenerate_current(wd)?;
    let mut out = String::new();
    out.push_str(seed.trim_end());
    out.push_str("\n\n---\n\n");
    out.push_str(current.trim_end());
    out.push_str("\n\n---\n\n");
    out.push_str(templates::PRIMITIVE_GUIDE.trim_end());
    out.push('\n');
    util::write_atomic(&wd.compiled_file(), &out)?;
    Ok(out)
}

/// The phase instruction handed to the runner as the user prompt.
pub fn phase_prompt(wd: &Workdir, phase: Phase) -> Result<String> {
    let st = wd.state()?;
    let tpl = templates::phase_prompt(phase.as_str())
        .ok_or_else(|| anyhow::anyhow!("phase {phase} has no prompt"))?;
    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("name", format!("{} {}", st.vpak_name, st.vpak_version));
    vars.insert("workdir", wd.root().display().to_string());
    vars.insert("dest", st.dest.display().to_string());
    vars.insert("phase", phase.as_str().to_string());
    vars.insert("mode", st.mode.as_str().to_string());
    Ok(util::render(tpl, &vars))
}
