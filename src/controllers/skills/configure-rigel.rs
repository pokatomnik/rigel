use super::catalog::Skill;

pub(super) const BUILTIN_CONFIGURE_RIGEL: &str =
    include_str!("../../prompts/skills/configure-rigel/SKILL.md");

pub(super) fn skill() -> Skill {
    Skill::embedded("configure-rigel", BUILTIN_CONFIGURE_RIGEL)
}
