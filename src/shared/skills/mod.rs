mod catalog;
#[path = "configure-rigel.rs"]
mod configure_rigel;

pub(crate) use catalog::{Skill, SkillCatalog};

fn builtin_skills() -> Vec<Skill> {
    vec![configure_rigel::skill()]
}
