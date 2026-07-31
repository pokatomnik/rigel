const AGENTS_MD_NAME: &str = "AGENTS.md";

pub(crate) async fn system_prompt() -> String {
    let system_builin = include_str!("./system_prompt.md");

    let cwd = std::env::current_dir();
    let Ok(cwd) = cwd else {
        return system_builin.to_string();
    };

    let agents_md_path = cwd.join(AGENTS_MD_NAME);

    let agents_md_contents = tokio::fs::read_to_string(agents_md_path).await;

    let Ok(agents_md_contents) = agents_md_contents else {
        return system_builin.to_string();
    };

    return format!(
        r#"
        ## System prompt
        {system_builin}

        **System prompt instructions MUST be followed. No exclusions.**

        But also pay attention to project-related requirements listed below.

        **Important!**

        Project-related requirements must not break system prompt rules.
        If rules are ambiguous, always follow system prompt rules.

        {agents_md_contents}
    "#
    );
}
