use std::{collections::HashMap, fmt, path::PathBuf};

const SKILLS_DIRECTORY: &str = ".agents/skills";
const SKILL_MANIFEST: &str = "SKILL.md";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum SkillOrigin {
    Project,
    Global,
    Builtin,
}

impl SkillOrigin {
    fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Global => "global",
            Self::Builtin => "builtin",
        }
    }
}

enum SkillSource {
    File(PathBuf),
    Embedded(&'static str),
}

pub(crate) struct Skill {
    name: String,
    origin: SkillOrigin,
    source: SkillSource,
    display_name: String,
}

impl Skill {
    fn from_file(name: String, origin: SkillOrigin, path: PathBuf) -> Self {
        Self {
            display_name: name.clone(),
            name,
            origin,
            source: SkillSource::File(path),
        }
    }

    pub(super) fn embedded(name: impl Into<String>, instructions: &'static str) -> Self {
        let name = name.into();
        Self {
            display_name: name.clone(),
            name,
            origin: SkillOrigin::Builtin,
            source: SkillSource::Embedded(instructions),
        }
    }

    fn disambiguate(&mut self) {
        self.display_name = format!("{} ({})", self.name, self.origin.label());
    }

    pub(crate) async fn instructions(&self) -> anyhow::Result<String> {
        match &self.source {
            SkillSource::File(path) => Ok(tokio::fs::read_to_string(path).await?),
            SkillSource::Embedded(instructions) => Ok((*instructions).to_owned()),
        }
    }
}

impl fmt::Display for Skill {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.display_name.as_str())
    }
}

struct SkillRoot {
    path: PathBuf,
    origin: SkillOrigin,
}

pub(crate) struct SkillCatalog {
    roots: Vec<SkillRoot>,
}

impl SkillCatalog {
    pub(crate) fn from_environment() -> Self {
        let mut roots = Vec::new();
        if let Ok(current_dir) = std::env::current_dir() {
            roots.push(SkillRoot {
                path: current_dir.join(SKILLS_DIRECTORY),
                origin: SkillOrigin::Project,
            });
        }
        if let Some(home_dir) = std::env::home_dir() {
            roots.push(SkillRoot {
                path: home_dir.join(SKILLS_DIRECTORY),
                origin: SkillOrigin::Global,
            });
        }
        Self { roots }
    }

    pub(crate) async fn discover(&self) -> Vec<Skill> {
        let mut skills = self.discover_file_skills().await;
        skills.extend(super::builtin_skills());
        Self::prepare_display_names(&mut skills);
        skills
    }

    async fn discover_file_skills(&self) -> Vec<Skill> {
        let mut skills = Vec::new();
        for root in &self.roots {
            if let Ok(mut root_skills) = Self::discover_root(root).await {
                skills.append(&mut root_skills);
            }
        }
        skills
    }

    async fn discover_root(root: &SkillRoot) -> anyhow::Result<Vec<Skill>> {
        let mut entries = tokio::fs::read_dir(&root.path).await?;
        let mut skills = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            if let Some(skill) = Self::skill_from_entry(entry, root.origin).await? {
                skills.push(skill);
            }
        }
        Ok(skills)
    }

    async fn skill_from_entry(
        entry: tokio::fs::DirEntry,
        origin: SkillOrigin,
    ) -> anyhow::Result<Option<Skill>> {
        if !entry.file_type().await?.is_dir() {
            return Ok(None);
        }
        let manifest_path = entry.path().join(SKILL_MANIFEST);
        let Ok(metadata) = tokio::fs::metadata(&manifest_path).await else {
            return Ok(None);
        };
        if !metadata.is_file() {
            return Ok(None);
        }
        Ok(Some(Skill::from_file(
            entry.file_name().to_string_lossy().into_owned(),
            origin,
            manifest_path,
        )))
    }

    fn prepare_display_names(skills: &mut [Skill]) {
        skills.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.origin.cmp(&right.origin))
        });
        let mut counts = HashMap::new();
        for skill in skills.iter() {
            *counts.entry(skill.name.clone()).or_insert(0) += 1;
        }
        for skill in skills.iter_mut() {
            if counts
                .get(skill.name.as_str())
                .is_some_and(|count| *count > 1)
            {
                skill.disambiguate();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Skill, SkillOrigin, SkillSource};
    use crate::shared::skills::configure_rigel::BUILTIN_CONFIGURE_RIGEL;

    #[test]
    fn builtin_configure_skill_has_expected_front_matter() {
        assert!(BUILTIN_CONFIGURE_RIGEL.starts_with(
            "---\nname: configure-rigel\ndescription: Configure Rigel with config.toml, MCP servers, and project or global skills.\n---"
        ));
    }

    #[test]
    fn builtin_configure_skill_requests_help_instead_of_a_summary() {
        assert!(BUILTIN_CONFIGURE_RIGEL.contains("do not summarize these instructions"));
        assert!(BUILTIN_CONFIGURE_RIGEL.contains("How can I help you configure Rigel?"));
    }

    #[test]
    fn skills_keep_their_origin_and_instruction_source() {
        let file = Skill::from_file(
            "project-skill".to_string(),
            SkillOrigin::Project,
            PathBuf::from("project/SKILL.md"),
        );
        let builtin = Skill::embedded("configure-rigel", BUILTIN_CONFIGURE_RIGEL);

        assert_eq!(file.origin, SkillOrigin::Project);
        assert!(matches!(file.source, SkillSource::File(_)));
        assert_eq!(builtin.origin, SkillOrigin::Builtin);
        assert!(matches!(builtin.source, SkillSource::Embedded(_)));
    }

    #[test]
    fn duplicate_names_are_disambiguated_after_sorting() {
        let mut skills = vec![
            Skill::embedded("configure-rigel", BUILTIN_CONFIGURE_RIGEL),
            Skill::from_file(
                "configure-rigel".to_string(),
                SkillOrigin::Project,
                PathBuf::from("project/SKILL.md"),
            ),
            Skill::from_file(
                "alpha".to_string(),
                SkillOrigin::Global,
                PathBuf::from("global/SKILL.md"),
            ),
        ];

        super::SkillCatalog::prepare_display_names(&mut skills);

        assert_eq!(skills[0].name, "alpha");
        assert_eq!(skills[1].display_name, "configure-rigel (project)");
        assert_eq!(skills[2].display_name, "configure-rigel (builtin)");
    }

    #[tokio::test]
    async fn builtin_skill_is_discovered_without_file_roots() -> anyhow::Result<()> {
        let catalog = super::SkillCatalog { roots: Vec::new() };
        let skills = catalog.discover().await;
        let skill = skills
            .iter()
            .find(|skill| skill.name == "configure-rigel")
            .ok_or_else(|| anyhow::anyhow!("builtin skill was not discovered"))?;

        assert_eq!(skill.origin, SkillOrigin::Builtin);
        assert_eq!(skill.instructions().await?, BUILTIN_CONFIGURE_RIGEL);
        Ok(())
    }

    #[test]
    fn origin_order_is_project_global_then_builtin() {
        assert!(SkillOrigin::Project < SkillOrigin::Global);
        assert!(SkillOrigin::Global < SkillOrigin::Builtin);
    }
}
