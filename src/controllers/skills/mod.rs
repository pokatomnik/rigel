pub mod catalog;
#[path = "configure-rigel.rs"]
pub mod configure_rigel;

use catalog::Skill;

fn builtin_skills() -> Vec<Skill> {
    vec![configure_rigel::skill()]
}
