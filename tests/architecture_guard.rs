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
    let tokens = import_tokens(source);
    let mut paths = Vec::new();
    for start in 0..tokens.len() {
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

fn path_matches(path: &[String], forbidden: &str) -> bool {
    let forbidden_segments: Vec<&str> = forbidden
        .trim_end_matches(':')
        .split("::")
        .filter(|segment| !segment.is_empty())
        .collect();
    path.starts_with(
        forbidden_segments
            .iter()
            .map(|segment| (*segment).to_owned())
            .collect::<Vec<_>>()
            .as_slice(),
    )
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
    assert_forbidden_imports(root.as_path(), &["crate::controllers", "controllers::"])
}

#[test]
fn shared_contains_only_domain_neutral_primitives() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shared");
    assert_forbidden_imports(
        root.as_path(),
        &[
            "crate::cmd",
            "crate::controllers",
            "crate::contracts",
            "crate::entities",
            "crate::prompts",
            "crate::tools",
            "crate::use_cases",
        ],
    )
}

#[test]
fn tools_do_not_depend_on_shared_services() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tools");
    assert_forbidden_imports(root.as_path(), &["crate::shared", "shared::"])
}

#[test]
fn contracts_do_not_contain_runtime_framework_types() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/contracts");
    assert_forbidden_imports(
        root.as_path(),
        &["rig::", "reqwest::", "dialoguer::", "tokio::"],
    )
}

#[test]
fn use_cases_do_not_depend_on_controllers_prompts_or_rig() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/use_cases");
    assert_forbidden_imports(
        root.as_path(),
        &[
            "crate::controllers",
            "controllers::",
            "crate::adapters",
            "adapters::",
            "crate::prompts",
            "rig::",
        ],
    )
}

#[test]
fn tools_do_not_depend_on_controllers_or_use_cases() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tools");
    assert_forbidden_imports(
        root.as_path(),
        &[
            "crate::controllers",
            "controllers::",
            "crate::use_cases",
            "use_cases::",
            "crate::adapters",
            "adapters::",
        ],
    )
}

#[test]
fn entities_do_not_depend_on_contracts_or_runtime_adapters() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/entities");
    assert_forbidden_imports(
        root.as_path(),
        &[
            "crate::contracts",
            "contracts::",
            "crate::adapters",
            "adapters::",
        ],
    )
}

#[test]
fn controllers_do_not_depend_on_use_cases_or_runtime_adapters() -> anyhow::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/controllers");
    assert_forbidden_imports(
        root.as_path(),
        &[
            "crate::use_cases",
            "use_cases::",
            "crate::adapters",
            "adapters::",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::{actual_import_paths, path_matches};

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
