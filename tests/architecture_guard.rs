use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum ImportToken {
    Name(String),
    ColonColon,
    OpenBrace,
    CloseBrace,
    Comma,
}

fn rust_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files)?;
    Ok(files)
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            collect_rust_files(entry_path.as_path(), files)?;
        } else if entry_path
            .extension()
            .is_some_and(|extension| extension == "rs")
        {
            files.push(entry_path);
        }
    }
    Ok(())
}

fn source_directories(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut directories = vec![root.to_path_buf()];
    collect_source_directories(root, &mut directories)?;
    Ok(directories)
}

fn collect_source_directories(path: &Path, directories: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry_path = entry?.path();
        if entry_path.is_dir() {
            directories.push(entry_path.clone());
            collect_source_directories(entry_path.as_path(), directories)?;
        }
    }
    Ok(())
}

fn production_source(source: &str) -> String {
    remove_cfg_test_items(strip_comments_and_literals(source).as_str())
}

fn strip_comments_and_literals(source: &str) -> String {
    let mut code = String::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        if source[index..].starts_with("//") {
            let end = source[index..]
                .find('\n')
                .map_or(source.len(), |offset| index + offset);
            push_blanks(&mut code, &source[index..end]);
            index = end;
        } else if source[index..].starts_with("/*") {
            let end = block_comment_end(source, index);
            push_blanks(&mut code, &source[index..end]);
            index = end;
        } else if let Some(end) = raw_literal_end(source, index) {
            push_blanks(&mut code, &source[index..end]);
            index = end;
        } else if matches!(source.as_bytes()[index], b'"' | b'\'') {
            let quote = source.as_bytes()[index] as char;
            if let Some(end) = quoted_literal_end(source, index, quote) {
                push_blanks(&mut code, &source[index..end]);
                index = end;
            } else if quote == '"' {
                push_blanks(&mut code, &source[index..]);
                break;
            } else {
                push_next_char(&mut code, source, &mut index);
            }
        } else {
            push_next_char(&mut code, source, &mut index);
        }
    }
    code
}

fn block_comment_end(source: &str, start: usize) -> usize {
    let mut index = start + 2;
    let mut depth = 1;
    while index < source.len() {
        if source[index..].starts_with("/*") {
            depth += 1;
            index += 2;
        } else if source[index..].starts_with("*/") {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return index;
            }
        } else {
            advance_char(source, &mut index);
        }
    }
    index
}

fn raw_literal_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let marker = if bytes.get(start) == Some(&b'r') {
        start + 1
    } else if bytes.get(start) == Some(&b'b') && bytes.get(start + 1) == Some(&b'r') {
        start + 2
    } else {
        return None;
    };

    let mut quote = marker;
    while bytes.get(quote) == Some(&b'#') {
        quote += 1;
    }
    if bytes.get(quote) != Some(&b'"') {
        return None;
    }
    let hashes = quote - marker;
    let mut index = quote + 1;
    while let Some(offset) = source[index..].find('"') {
        let closing_quote = index + offset;
        let closing_end = closing_quote + 1 + hashes;
        if closing_end <= source.len()
            && source[closing_quote + 1..closing_end]
                .bytes()
                .all(|byte| byte == b'#')
        {
            return Some(closing_end);
        }
        index = closing_quote + 1;
    }
    Some(source.len())
}

fn quoted_literal_end(source: &str, start: usize, quote: char) -> Option<usize> {
    let mut escaped = false;
    for (offset, character) in source[start + 1..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
        } else if character == quote {
            return Some(start + 1 + offset + character.len_utf8());
        } else if character == '\n' {
            return None;
        }
    }
    None
}

fn push_blanks(output: &mut String, source: &str) {
    for character in source.chars() {
        output.push(if character == '\n' { '\n' } else { ' ' });
    }
}

fn push_next_char(output: &mut String, source: &str, index: &mut usize) {
    if let Some(character) = source[*index..].chars().next() {
        output.push(character);
        *index += character.len_utf8();
    } else {
        *index = source.len();
    }
}

fn advance_char(source: &str, index: &mut usize) {
    if let Some(character) = source[*index..].chars().next() {
        *index += character.len_utf8();
    } else {
        *index = source.len();
    }
}

fn remove_cfg_test_items(source: &str) -> String {
    let ranges = cfg_test_ranges(source);
    source
        .char_indices()
        .map(|(index, character)| {
            let in_test_item = ranges
                .iter()
                .any(|(start, end)| index >= *start && index < *end);
            if in_test_item && character != '\n' {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn cfg_test_ranges(source: &str) -> Vec<(usize, usize)> {
    let marker = "#[cfg(test)]";
    let mut ranges = Vec::new();
    let mut search_start = 0;
    while let Some(offset) = source[search_start..].find(marker) {
        let start = search_start + offset;
        let item_start = skip_whitespace(source, start + marker.len());
        let end = test_item_end(source, item_start);
        ranges.push((start, end));
        search_start = end;
    }
    ranges
}

fn production_physical_lines(source: &str) -> usize {
    let code = strip_comments_and_literals(source);
    let ranges = cfg_test_ranges(&code);
    let mut offset = 0;
    let mut count = 0;
    for line in source.split_inclusive('\n') {
        let end = offset + line.len();
        if !ranges
            .iter()
            .any(|(start, finish)| offset >= *start && offset < *finish)
        {
            count += 1;
        }
        offset = end;
    }
    count
}

fn skip_whitespace(source: &str, mut index: usize) -> usize {
    while index < source.len() {
        let Some(character) = source[index..].chars().next() else {
            break;
        };
        if !character.is_whitespace() {
            break;
        }
        index += character.len_utf8();
    }
    index
}

fn test_item_end(source: &str, item_start: usize) -> usize {
    if source[item_start..].starts_with("use ") {
        return source[item_start..]
            .find(';')
            .map_or(source.len(), |offset| item_start + offset + 1);
    }
    let mut index = item_start;
    while index < source.len() {
        let Some(character) = source[index..].chars().next() else {
            return source.len();
        };
        if character == ';' {
            return index + character.len_utf8();
        }
        if character == '{' {
            return balanced_block_end(source, index);
        }
        index += character.len_utf8();
    }
    source.len()
}

fn balanced_block_end(source: &str, start: usize) -> usize {
    let mut depth = 0;
    let mut index = start;
    while index < source.len() {
        let Some(character) = source[index..].chars().next() else {
            return source.len();
        };
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return index + character.len_utf8();
                }
            }
            _ => {}
        }
        index += character.len_utf8();
    }
    source.len()
}

fn actual_import_paths(source: &str) -> Vec<Vec<String>> {
    let production = production_source(source);
    let mut paths = import_paths(&production);
    paths.extend(qualified_paths(&production));
    paths
}

fn import_paths(source: &str) -> Vec<Vec<String>> {
    let mut paths = Vec::new();
    let mut search_start = 0;
    while let Some(offset) = source[search_start..].find("use") {
        let start = search_start + offset;
        if !is_keyword_at(source, start, "use") {
            search_start = start + 1;
            continue;
        }
        let statement_start = start + "use".len();
        let Some(end_offset) = source[statement_start..].find(';') else {
            break;
        };
        let end = statement_start + end_offset;
        let tokens = import_tokens(&source[statement_start..end]);
        let mut cursor = 0;
        parse_use_tree(&tokens, &mut cursor, &[], &mut paths);
        search_start = end + 1;
    }
    paths
}

fn is_keyword_at(source: &str, start: usize, keyword: &str) -> bool {
    let end = start + keyword.len();
    source.get(start..end).is_some_and(|value| value == keyword)
        && (start == 0 || !is_identifier_byte(source.as_bytes()[start - 1]))
        && (end == source.len() || !is_identifier_byte(source.as_bytes()[end]))
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn import_tokens(source: &str) -> Vec<ImportToken> {
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < source.len() {
        let byte = source.as_bytes()[index];
        if byte.is_ascii_whitespace() {
            index += 1;
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            let start = index;
            index += 1;
            while index < source.len() && is_identifier_byte(source.as_bytes()[index]) {
                index += 1;
            }
            tokens.push(ImportToken::Name(source[start..index].to_owned()));
        } else if source[index..].starts_with("::") {
            tokens.push(ImportToken::ColonColon);
            index += 2;
        } else {
            let token = match byte {
                b'{' => Some(ImportToken::OpenBrace),
                b'}' => Some(ImportToken::CloseBrace),
                b',' => Some(ImportToken::Comma),
                _ => None,
            };
            if let Some(token) = token {
                tokens.push(token);
            }
            index += 1;
        }
    }
    tokens
}

fn parse_use_tree(
    tokens: &[ImportToken],
    cursor: &mut usize,
    prefix: &[String],
    paths: &mut Vec<Vec<String>>,
) {
    let mut path = prefix.to_vec();
    while let Some(ImportToken::Name(name)) = tokens.get(*cursor) {
        if name == "as" {
            break;
        }
        path.push(name.clone());
        *cursor += 1;
        if !matches!(tokens.get(*cursor), Some(ImportToken::ColonColon)) {
            break;
        }
        *cursor += 1;
        if matches!(tokens.get(*cursor), Some(ImportToken::OpenBrace)) {
            *cursor += 1;
            parse_use_group(tokens, cursor, &path, paths);
            return;
        }
    }

    if !path.is_empty() {
        paths.push(path);
    }
    skip_to_use_separator(tokens, cursor);
}

fn parse_use_group(
    tokens: &[ImportToken],
    cursor: &mut usize,
    prefix: &[String],
    paths: &mut Vec<Vec<String>>,
) {
    while *cursor < tokens.len() {
        if matches!(tokens.get(*cursor), Some(ImportToken::CloseBrace)) {
            *cursor += 1;
            return;
        }
        if matches!(tokens.get(*cursor), Some(ImportToken::Comma)) {
            *cursor += 1;
            continue;
        }
        parse_use_tree(tokens, cursor, prefix, paths);
        if matches!(tokens.get(*cursor), Some(ImportToken::Comma)) {
            *cursor += 1;
        }
    }
}

fn skip_to_use_separator(tokens: &[ImportToken], cursor: &mut usize) {
    while *cursor < tokens.len()
        && !matches!(
            tokens.get(*cursor),
            Some(ImportToken::Comma | ImportToken::CloseBrace)
        )
    {
        *cursor += 1;
    }
}

fn qualified_paths(source: &str) -> Vec<Vec<String>> {
    let source = without_use_statements(source);
    let tokens = import_tokens(&source);
    let mut paths = Vec::new();
    for start in 0..tokens.len() {
        if start > 0 && matches!(tokens[start - 1], ImportToken::ColonColon) {
            continue;
        }
        let Some(ImportToken::Name(name)) = tokens.get(start) else {
            continue;
        };
        let mut path = vec![name.clone()];
        let mut cursor = start + 1;
        while matches!(tokens.get(cursor), Some(ImportToken::ColonColon)) {
            let Some(ImportToken::Name(name)) = tokens.get(cursor + 1) else {
                break;
            };
            path.push(name.clone());
            cursor += 2;
        }
        if path.len() > 1 {
            paths.push(path);
        }
    }
    paths
}

fn without_use_statements(source: &str) -> String {
    let mut code = source.as_bytes().to_vec();
    let mut cursor = 0;
    while let Some(offset) = source[cursor..].find("use") {
        let start = cursor + offset;
        if !is_keyword_at(source, start, "use") {
            cursor = start + 1;
            continue;
        }
        let end = source[start..]
            .find(';')
            .map_or(source.len(), |offset| start + offset + 1);
        for byte in code.iter_mut().take(end).skip(start) {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
        cursor = end;
    }
    String::from_utf8_lossy(&code).into_owned()
}

fn path_matches(path: &[String], forbidden: &str) -> bool {
    let forbidden_segments: Vec<&str> = forbidden
        .trim_end_matches(':')
        .split("::")
        .filter(|segment| !segment.is_empty())
        .collect();
    let forbidden_segments = forbidden_segments
        .iter()
        .map(|segment| (*segment).to_owned())
        .collect::<Vec<_>>();
    path.starts_with(forbidden_segments.as_slice())
        || (path.first().is_some_and(|segment| segment == "crate")
            && path
                .get(1..)
                .is_some_and(|root_path| root_path.starts_with(forbidden_segments.as_slice())))
}

fn assert_forbidden_imports(root: &Path, forbidden: &[&str]) -> anyhow::Result<()> {
    for file in rust_files(root)? {
        let source = fs::read_to_string(file.as_path())?;
        let imports = actual_import_paths(source.as_str());
        for import in imports {
            for forbidden_path in forbidden {
                assert!(
                    !path_matches(import.as_slice(), forbidden_path),
                    "forbidden architecture import `{forbidden_path}` in {}",
                    file.display()
                );
            }
        }
    }
    Ok(())
}

#[test]
fn command_parsing_does_not_know_controllers() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cmd");
    assert_forbidden_imports(
        root.as_path(),
        &[
            "crate::controllers",
            "controllers::",
            "crate::infrastructure",
            "infrastructure::",
            "crate::prompts",
            "prompts::",
            "crate::tools",
            "tools::",
            "crate::use_cases",
            "use_cases::",
            "crate::shared",
            "shared::",
        ],
    )
}

#[test]
fn shared_contains_only_domain_neutral_primitives() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shared");
    assert_forbidden_imports(
        root.as_path(),
        &[
            "cmd::",
            "controllers::",
            "infrastructure::",
            "entities::",
            "prompts::",
            "tools::",
            "use_cases::",
        ],
    )
}

#[test]
fn entities_do_not_depend_on_application_or_technical_layers() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/entities");
    assert_forbidden_imports(
        root.as_path(),
        &[
            "cmd::",
            "controllers::",
            "infrastructure::",
            "shared::",
            "prompts::",
            "tools::",
            "use_cases::",
            "adapters::",
        ],
    )
}

#[test]
fn infrastructure_does_not_depend_on_concrete_outer_layers() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/infrastructure");
    assert_forbidden_imports(
        root.as_path(),
        &[
            "cmd::",
            "controllers::",
            "shared::",
            "prompts::",
            "tools::",
            "use_cases::",
            "adapters::",
            "rig::",
            "reqwest::",
            "dialoguer::",
            "tokio::",
        ],
    )
}

#[test]
fn tools_do_not_depend_on_outer_layers_or_shared_services() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tools");
    assert_forbidden_imports(
        root.as_path(),
        &[
            "cmd::",
            "controllers::",
            "shared::",
            "prompts::",
            "use_cases::",
            "adapters::",
        ],
    )
}

#[test]
fn prompts_do_not_depend_on_application_or_runtime_layers() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/prompts");
    assert_forbidden_imports(
        root.as_path(),
        &[
            "cmd::",
            "controllers::",
            "shared::",
            "tools::",
            "use_cases::",
            "adapters::",
        ],
    )
}

#[test]
fn use_cases_do_not_depend_on_controllers_prompts_or_provider_runtime() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/use_cases");
    assert_forbidden_imports(
        root.as_path(),
        &[
            "cmd::",
            "controllers::",
            "prompts::",
            "tools::",
            "adapters::",
            "rig::",
            "reqwest::",
            "dialoguer::",
        ],
    )
}

#[test]
fn controllers_do_not_depend_on_application_or_prompt_construction() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/controllers");
    assert_forbidden_imports(
        root.as_path(),
        &["cmd::", "use_cases::", "prompts::", "tools::", "adapters::"],
    )
}

#[test]
fn source_contains_only_the_approved_top_level_layers() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut actual = fs::read_dir(root.as_path())?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut expected = vec![
        "cmd",
        "controllers",
        "entities",
        "infrastructure",
        "main.rs",
        "prompts",
        "shared",
        "tools",
        "use_cases",
        "user_agents.txt",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    actual.sort();
    expected.sort();
    assert_eq!(
        actual, expected,
        "unexpected top-level source layer or file"
    );
    Ok(())
}

#[test]
fn no_unapproved_top_level_adapter_layer_exists() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(
        !root.join("adapters").exists(),
        "src/adapters is a forbidden layer"
    );
    Ok(())
}

fn concrete_declaration(source: &str) -> Option<&'static str> {
    let production = production_source(source);
    for line in production.lines() {
        let mut declaration = line.trim_start();
        for visibility in ["pub(crate) ", "pub(super) ", "pub "] {
            if let Some(item) = declaration.strip_prefix(visibility) {
                declaration = item;
                break;
            }
        }
        for keyword in ["struct", "enum", "union", "type", "const", "static", "impl"] {
            if declaration.strip_prefix(keyword).is_some_and(|rest| {
                rest.chars().next().is_some_and(char::is_whitespace) || rest.starts_with('<')
            }) {
                return Some(keyword);
            }
        }
    }
    None
}

#[test]
fn infrastructure_is_traits_only_including_mod_files() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/infrastructure");
    for file in rust_files(root.as_path())? {
        let source = fs::read_to_string(file.as_path())?;
        assert!(
            concrete_declaration(source.as_str()).is_none(),
            "infrastructure may contain only trait contracts: {}",
            file.display()
        );
    }
    Ok(())
}

#[test]
fn agent_wiring_does_not_own_workspace_or_late_mutable_tool_sets() -> anyhow::Result<()> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/controllers/model_runtime/agent/dependencies.rs");
    let source = production_source(fs::read_to_string(path.as_path())?.as_str());
    for forbidden in ["WorkspaceCapabilities", "RwLock", "set_tool_sets"] {
        assert!(
            !source.contains(forbidden),
            "AgentWiring may not retain `{forbidden}`"
        );
    }
    Ok(())
}

#[test]
fn legacy_composition_bags_and_global_telemetry_are_removed() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in rust_files(root.as_path())? {
        let source = fs::read_to_string(file.as_path())?;
        for forbidden in [
            "AgentDependencies",
            "ToolBuildContext",
            "GLOBAL_SHELL_USAGE",
            "ShellUsageTelemetry::global",
        ] {
            assert!(
                !source.contains(forbidden),
                "legacy composition symbol `{forbidden}` remains in {}",
                file.display()
            );
        }
    }
    Ok(())
}

#[test]
fn production_rust_files_respect_the_physical_line_limit() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in rust_files(root.as_path())? {
        let source = fs::read_to_string(file.as_path())?;
        let lines = production_physical_lines(source.as_str());
        assert!(
            lines <= 300,
            "production source has {lines} physical lines (maximum 300): {}",
            file.display()
        );
    }
    Ok(())
}

#[test]
fn source_directories_respect_the_file_count_limit() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for directory in source_directories(root.as_path())? {
        let files = fs::read_dir(directory.as_path())?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .filter(|entry| entry.file_name() != "mod.rs")
            .count();
        assert!(
            files <= 7,
            "directory contains {files} files excluding mod.rs (maximum 7): {}",
            directory.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{actual_import_paths, path_matches, production_physical_lines};

    #[test]
    fn expands_nested_grouped_imports_without_inventing_root_paths() {
        let source =
            "use crate::{infrastructure::{tools::network::Fetcher}, entities::model::Model};";
        let imports = actual_import_paths(source);

        assert!(
            imports
                .iter()
                .any(|path| path_matches(path, "crate::infrastructure"))
        );
        assert!(
            imports
                .iter()
                .any(|path| path_matches(path, "crate::entities"))
        );
        assert!(!imports.iter().any(|path| path_matches(path, "tools::")));
    }

    #[test]
    fn detects_root_qualified_expressions_without_matching_nested_suffixes() {
        let source = "use tools::run; use infrastructure::tools::Fetcher; let _ = crate::tools::run(); let _ = tools::run(); let _ = infrastructure::tools::Fetcher;";
        let imports = actual_import_paths(source);

        assert!(
            imports
                .iter()
                .any(|path| path_matches(path, "crate::tools"))
        );
        assert!(imports.iter().any(|path| path_matches(path, "tools::")));
        assert!(
            imports
                .iter()
                .any(|path| path_matches(path, "infrastructure::tools"))
        );
        assert!(!imports.iter().any(|path| {
            path_matches(path, "tools::")
                && path
                    .first()
                    .is_some_and(|segment| segment == "infrastructure")
        }));
    }

    #[test]
    fn excludes_fully_qualified_paths_inside_cfg_test_items() {
        let source = "#[cfg(test)] mod tests { let _ = crate::controllers::TestOnly; } let _ = crate::entities::Model;";
        let imports = actual_import_paths(source);

        assert!(
            !imports
                .iter()
                .any(|path| path_matches(path, "crate::controllers"))
        );
        assert!(
            imports
                .iter()
                .any(|path| path_matches(path, "crate::entities"))
        );
    }

    #[test]
    fn production_line_count_excludes_test_items_only() {
        let source = "fn production() {}\n#[cfg(test)]\nmod tests {\n    fn test() {}\n}\n";
        assert_eq!(production_physical_lines(source), 1);
    }

    #[test]
    fn ignores_comments_literals_and_test_modules() {
        let source = r#"
            // use crate::controllers::comment;
            const TEXT: &str = "use crate::controllers::literal;";

            #[cfg(test)]
            mod tests {
                use crate::controllers::test_only;
            }

            use crate::{entities::selected_model::SelectedModel};
        "#;
        let imports = actual_import_paths(source);

        assert!(
            imports
                .iter()
                .any(|path| path_matches(path, "crate::entities"))
        );
        assert!(
            !imports
                .iter()
                .any(|path| path_matches(path, "crate::controllers"))
        );
    }
}
