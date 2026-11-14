use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use tokio::fs;

use crate::{
    controllers::storage::{
        paths::HistoryPaths,
        record::{ChatLine, RecordId, RecordKind, SCHEMA_VERSION},
    },
    entities::{
        conversation::conversation_message::ConversationMessage, transcript::entry::TranscriptEntry,
    },
};

/// One archived history segment read from a numeric `chat.<N>.jsonl` file.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HistoryArchive {
    /// Archive file name used by UI boundaries and diagnostics.
    pub(crate) file_name: String,
    /// Messages stored in this sealed segment.
    pub(crate) messages: Vec<ConversationMessage>,
}

/// Everything needed to start a chat from durable storage.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LoadedHistory {
    /// Model context from the active segment only.
    pub(crate) context: Vec<ConversationMessage>,
    /// Sealed segments in numeric order; never sent to the model.
    pub(crate) archives: Vec<HistoryArchive>,
    /// Effective system prompt stored at the start of the active segment.
    pub(crate) system_prompt: Option<String>,
    /// Number of unreadable lines skipped while loading.
    pub(crate) skipped_lines: usize,
    /// Transcript entries projected from archives and the active segment.
    pub(crate) entries: Vec<TranscriptEntry>,
    /// Next record identifier that keeps ids unique across the family.
    pub(crate) next_id: RecordId,
}

/// Reads the active JSONL segment and its sealed archives without repairing them.
pub(crate) struct HistoryReader {
    paths: HistoryPaths,
}

impl HistoryReader {
    /// Creates a reader over one history family.
    pub(crate) fn new(paths: HistoryPaths) -> Self {
        Self { paths }
    }

    /// Loads model context and archives, applying the recovery matrix for missing files.
    pub(crate) async fn load(&self) -> anyhow::Result<LoadedHistory> {
        let archive_files = self.archive_files().await?;
        let active = self
            .read_segment(self.paths.active())
            .await
            .context("failed to read the active chat history")?;
        let archives = self.read_archives(archive_files).await?;
        if active.is_none() && !archives.is_empty() {
            anyhow::bail!(
                "active chat history '{}' is missing while archive segments exist",
                self.paths.active().display()
            );
        }
        let parsed_active = active.unwrap_or_else(ParsedSegment::empty);
        let mut skipped_lines = parsed_active.skipped;
        let (context, system_prompt, conversion_skipped) =
            apply_lines(parsed_active.lines.as_slice());
        skipped_lines += conversion_skipped;
        let mut next_id = max_id(parsed_active.lines.as_slice());
        let archive_segments = archives
            .iter()
            .map(|archive| (archive.file_name.clone(), archive.segment.lines.clone()))
            .collect::<Vec<_>>();
        let entries =
            crate::controllers::model_runtime::history::transcript_projection::project_history(
                archive_segments.as_slice(),
                parsed_active.lines.as_slice(),
            );
        let mut loaded_archives = Vec::with_capacity(archives.len());
        for archive in archives {
            skipped_lines += archive.segment.skipped;
            let (messages, _system_prompt, conversion_skipped) =
                apply_lines(archive.segment.lines.as_slice());
            skipped_lines += conversion_skipped;
            next_id = next_id.max(max_id(archive.segment.lines.as_slice()));
            loaded_archives.push(HistoryArchive {
                file_name: archive.file_name,
                messages,
            });
        }
        Ok(LoadedHistory {
            context,
            archives: loaded_archives,
            system_prompt,
            skipped_lines,
            entries,
            next_id: next_id.map_or(RecordId::new(1), RecordId::next),
        })
    }

    async fn read_archives(
        &self,
        files: Vec<(usize, String, PathBuf)>,
    ) -> anyhow::Result<Vec<ReadArchive>> {
        let mut archives = Vec::with_capacity(files.len());
        for (_index, file_name, path) in files {
            let segment = self
                .read_segment(path)
                .await
                .with_context(|| format!("failed to read archive `{file_name}`"))?
                .ok_or_else(|| {
                    anyhow::anyhow!("archive `{file_name}` disappeared while loading")
                })?;
            archives.push(ReadArchive { file_name, segment });
        }
        Ok(archives)
    }

    async fn read_segment(&self, path: PathBuf) -> anyhow::Result<Option<ParsedSegment>> {
        match fs::read_to_string(path.as_path()).await {
            Ok(content) => {
                let file_name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                Ok(Some(parse_content(content.as_str(), file_name.as_str())?))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => {
                Err(error).with_context(|| format!("failed to read '{}'", path.display()))
            }
        }
    }

    async fn archive_files(&self) -> anyhow::Result<Vec<(usize, String, PathBuf)>> {
        let mut entries = match fs::read_dir(self.paths.directory()).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut files = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let Some(index) = HistoryPaths::archive_index(file_name.as_str()) else {
                continue;
            };
            if entry.file_type().await?.is_file() {
                files.push((index, file_name, entry.path()));
            }
        }
        files.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
        Ok(files)
    }
}

struct ReadArchive {
    file_name: String,
    segment: ParsedSegment,
}

struct ParsedSegment {
    lines: Vec<ChatLine>,
    skipped: usize,
}

impl ParsedSegment {
    fn empty() -> Self {
        Self {
            lines: Vec::new(),
            skipped: 0,
        }
    }
}

/// Parses complete JSONL lines, skipping torn tails and unreadable records.
fn parse_content(content: &str, file_name: &str) -> anyhow::Result<ParsedSegment> {
    if content.is_empty() {
        return Ok(ParsedSegment::empty());
    }
    let has_trailing_newline = content.ends_with('\n');
    let parts = content.split('\n').collect::<Vec<_>>();
    let last_index = parts.len().saturating_sub(1);
    let mut segment = ParsedSegment::empty();
    for (index, part) in parts.iter().enumerate() {
        let part = part.strip_suffix('\r').unwrap_or(part);
        if part.is_empty() {
            continue;
        }
        if index == last_index && !has_trailing_newline {
            segment.skipped += 1;
            continue;
        }
        parse_line(part, file_name, &mut segment)?;
    }
    Ok(segment)
}

fn parse_line(part: &str, file_name: &str, segment: &mut ParsedSegment) -> anyhow::Result<()> {
    match serde_json::from_str::<ChatLine>(part) {
        Ok(line) if line.v != SCHEMA_VERSION => anyhow::bail!(
            "unsupported history schema version {} in `{file_name}`",
            line.v
        ),
        Ok(line) if !line.has_payload() => segment.skipped += 1,
        Ok(line) => segment.lines.push(line),
        Err(_) => segment.skipped += 1,
    }
    Ok(())
}

fn apply_lines(lines: &[ChatLine]) -> (Vec<ConversationMessage>, Option<String>, usize) {
    let mut messages = Vec::new();
    let mut skipped = 0;
    for line in lines {
        match line.to_conversation_messages() {
            Ok(replacement) => {
                if line.kind == RecordKind::Snapshot {
                    messages = replacement;
                } else {
                    messages.extend(replacement);
                }
            }
            Err(_) => skipped += 1,
        }
    }
    let system_prompt = messages.first().and_then(|message| match message {
        ConversationMessage::System(text) => Some(text.clone()),
        _ => None,
    });
    if system_prompt.is_some() {
        messages.drain(..1);
    }
    (messages, system_prompt, skipped)
}

fn max_id(lines: &[ChatLine]) -> Option<RecordId> {
    lines.iter().map(|line| line.id).max()
}

#[cfg(test)]
mod tests {
    use super::{SCHEMA_VERSION, apply_lines, parse_content};
    use crate::controllers::storage::record::{ChatLine, RecordId};
    use crate::entities::conversation::conversation_message::ConversationMessage;

    fn message_line(id: u64, text: &str) -> String {
        serde_json::json!({
            "v": SCHEMA_VERSION,
            "kind": "message",
            "id": id,
            "message": {"role": "user", "content": [{"type": "text", "text": text}]}
        })
        .to_string()
    }

    #[test]
    fn complete_lines_are_parsed_in_order() -> anyhow::Result<()> {
        let content = format!(
            "{}\n{}\n",
            message_line(1, "first"),
            message_line(2, "second")
        );
        let parsed = parse_content(content.as_str(), "chat.jsonl")?;

        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.skipped, 0);
        assert_eq!(parsed.lines[0].id, RecordId::new(1));
        Ok(())
    }

    #[test]
    fn torn_tail_is_skipped() -> anyhow::Result<()> {
        let content = format!("{}\n{{\"v\":1,\"kind\":\"mess", message_line(1, "first"));
        let parsed = parse_content(content.as_str(), "chat.jsonl")?;

        assert_eq!(parsed.lines.len(), 1);
        assert_eq!(parsed.skipped, 1);
        Ok(())
    }

    #[test]
    fn unreadable_lines_are_skipped_without_rewriting() -> anyhow::Result<()> {
        let content = format!(
            "{}\nnot json\n{}\n",
            message_line(1, "first"),
            message_line(2, "second")
        );
        let parsed = parse_content(content.as_str(), "chat.jsonl")?;

        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.skipped, 1);
        Ok(())
    }

    #[test]
    fn unsupported_schema_version_is_an_error() -> anyhow::Result<()> {
        let content = format!(
            "{{\"v\":{},\"kind\":\"summary\",\"id\":1,\"text\":\"x\"}}\n",
            SCHEMA_VERSION + 1
        );
        let result = parse_content(content.as_str(), "chat.jsonl");

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn snapshots_replace_previous_messages() -> anyhow::Result<()> {
        let first = ChatLine::message(
            RecordId::new(1),
            &ConversationMessage::User("old".to_string()),
        )?;
        let snapshot = ChatLine::snapshot(
            RecordId::new(2),
            &[ConversationMessage::Assistant("summary".to_string())],
        )?;
        let (messages, system_prompt, skipped) = apply_lines(&[first, snapshot]);

        assert_eq!(
            messages,
            [ConversationMessage::Assistant("summary".to_string())]
        );
        assert_eq!(system_prompt, None);
        assert_eq!(skipped, 0);
        Ok(())
    }

    #[test]
    fn summary_and_message_records_extend_the_context() -> anyhow::Result<()> {
        let summary = ChatLine::summary(RecordId::new(1), "summary");
        let message = ChatLine::message(
            RecordId::new(2),
            &ConversationMessage::User("next".to_string()),
        )?;
        let (messages, _, _) = apply_lines(&[summary, message]);

        assert_eq!(
            messages,
            [
                ConversationMessage::Assistant("summary".to_string()),
                ConversationMessage::User("next".to_string()),
            ]
        );
        Ok(())
    }

    #[test]
    fn leading_system_message_is_extracted_from_the_context() -> anyhow::Result<()> {
        let system = ChatLine::message(
            RecordId::new(1),
            &ConversationMessage::System("rules".to_string()),
        )?;
        let summary = ChatLine::summary(RecordId::new(2), "summary");
        let (messages, system_prompt, _) = apply_lines(&[system, summary]);

        assert_eq!(system_prompt.as_deref(), Some("rules"));
        assert_eq!(
            messages,
            [ConversationMessage::Assistant("summary".to_string())]
        );
        Ok(())
    }

    #[test]
    fn record_kind_is_stable_in_serialized_lines() -> anyhow::Result<()> {
        let line = ChatLine::summary(RecordId::new(7), "x");
        let value = serde_json::to_value(&line)?;

        assert_eq!(value["kind"], serde_json::json!("summary"));
        Ok(())
    }
}
