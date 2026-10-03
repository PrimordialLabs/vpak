//! Prompt templates embedded in the binary so a `vpak` install needs no
//! external files. Sources live in `templates/` at the repo root.

pub const BOOTSTRAP_SEED: &str = include_str!("../../../templates/bootstrap-seed.md");
pub const PRIMITIVE_GUIDE: &str = include_str!("../../../templates/primitive-guide.md");
pub const PACK_INSPECT: &str = include_str!("../../../templates/pack-inspect.md");
pub const POLICY_DEFAULT: &str = include_str!("../../../templates/policy-default.toml");

pub const PHASE_LOCATE: &str = include_str!("../../../templates/phases/locate.md");
pub const PHASE_SURVEY: &str = include_str!("../../../templates/phases/survey.md");
pub const PHASE_CONSTRAIN: &str = include_str!("../../../templates/phases/constrain.md");
pub const PHASE_PLAN: &str = include_str!("../../../templates/phases/plan.md");
pub const PHASE_DECIDE: &str = include_str!("../../../templates/phases/decide.md");
pub const PHASE_EXECUTE: &str = include_str!("../../../templates/phases/execute.md");
pub const PHASE_VERIFY: &str = include_str!("../../../templates/phases/verify.md");

/// The phase prompt for a phase name, or `None` for the terminal pseudo-phase.
pub fn phase_prompt(phase: &str) -> Option<&'static str> {
    Some(match phase {
        "locate" => PHASE_LOCATE,
        "survey" => PHASE_SURVEY,
        "constrain" => PHASE_CONSTRAIN,
        "plan" => PHASE_PLAN,
        "decide" => PHASE_DECIDE,
        "execute" => PHASE_EXECUTE,
        "verify" => PHASE_VERIFY,
        _ => return None,
    })
}
