use super::catalog::Skill;

pub(super) const BUILTIN_CONFIGURE_RIGEL: &str = include_str!("configure-rigel/SKILL.md");

pub(super) fn skill() -> Skill {
    Skill::embedded("configure-rigel", BUILTIN_CONFIGURE_RIGEL)
}
