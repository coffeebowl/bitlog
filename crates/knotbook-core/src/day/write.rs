//! Writes day files in canonical form.

use chrono::NaiveTime;

use super::sections::escape_block_headings;
use super::{Block, BlockId, Day, FORMAT};

impl Day {
    /// The content of the day file. The front matter is written in canonical
    /// form, the day note and block texts exactly as they are, except for
    /// headings that would start a block when read back.
    pub fn to_markdown(&self) -> String {
        let ids: Vec<BlockId> = self.blocks.iter().map(|block| block.id.clone()).collect();
        let mut parts = vec![
            format!("---\n{}---", self.front_matter()),
            format!("# {}", self.date.format("%Y-%m-%d")),
        ];
        if !self.note.is_empty() {
            parts.push(escape_block_headings(&self.note, &ids));
        }
        parts.extend(self.blocks.iter().map(|block| section(block, &ids)));
        // One blank line between the parts, one line break at the end.
        parts.join("\n\n") + "\n"
    }

    /// Fixed field order, all strings quoted, one block per line, unknown
    /// fields last. Every line ends with a line break.
    fn front_matter(&self) -> String {
        let mut lines = vec![
            format!("format: {FORMAT}"),
            format!("date: {}", quote(&self.date.format("%Y-%m-%d").to_string())),
            format!("kind: {}", quote(&self.kind)),
        ];
        if let Some(location) = &self.location {
            lines.push(format!("location: {}", quote(location.as_str())));
        }
        if !self.tags.is_empty() {
            let tags: Vec<String> = self.tags.iter().map(|tag| quote(tag)).collect();
            lines.push(format!("tags: [{}]", tags.join(", ")));
        }
        if let Some(energy) = self.energy {
            lines.push(format!("energy: {energy}"));
        }
        let work: Vec<String> = [("start", self.work_start), ("end", self.work_end)]
            .into_iter()
            .filter_map(|(key, time)| Some(format!("{key}: {}", quote_time(time?))))
            .collect();
        if !work.is_empty() {
            lines.push(format!("work: {{ {} }}", work.join(", ")));
        }
        if !self.blocks.is_empty() {
            lines.push("blocks:".to_owned());
        }
        lines.extend(self.blocks.iter().map(|block| {
            format!(
                "  - {{ id: {}, start: {}, end: {}, project: {} }}",
                quote(block.id.as_str()),
                quote_time(block.start),
                quote_time(block.end),
                quote(block.project.as_str()),
            )
        }));
        lines.extend(self.unknown_fields.iter().map(|(key, value)| {
            // JSON is valid YAML 1.2, and it quotes every string.
            let value = serde_json::to_string(value).expect("JSON values always serialize");
            format!("{}: {value}", yaml_key(key))
        }));
        lines.iter().map(|line| format!("{line}\n")).collect()
    }
}

/// `## Title {#id}`, then the text, if there is one.
fn section(block: &Block, ids: &[BlockId]) -> String {
    let heading = if block.title.is_empty() {
        format!("## {{#{}}}", block.id)
    } else {
        format!("## {} {{#{}}}", block.title, block.id)
    };
    if block.text.is_empty() {
        heading
    } else {
        format!("{heading}\n\n{}", escape_block_headings(&block.text, ids))
    }
}

/// A YAML string in double quotes. JSON escaping is valid there.
fn quote(text: &str) -> String {
    serde_json::to_string(text).expect("strings always serialize")
}

fn quote_time(time: NaiveTime) -> String {
    quote(&time.format("%H:%M").to_string())
}

/// Keys that YAML reads as plain strings stay bare, all others are quoted.
fn yaml_key(key: &str) -> String {
    let plain = key.starts_with(|c: char| c.is_ascii_alphabetic())
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    // Bare words that YAML 1.1 readers such as Obsidian's take for booleans or null.
    let reserved = matches!(
        key.to_ascii_lowercase().as_str(),
        "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n"
    );
    if plain && !reserved {
        key.to_owned()
    } else {
        quote(key)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    fn fixture(date: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/sample-vault/daily/2026/09")
            .join(format!("{date}.md"));
        fs::read_to_string(path).unwrap()
    }

    fn parse(text: &str) -> Day {
        Day::parse(text).unwrap().0
    }

    #[test]
    fn canonical_file_stays_byte_identical() {
        let text = fixture("2026-09-21");
        assert_eq!(parse(&text).to_markdown(), text);
    }

    #[test]
    fn other_files_keep_their_content() {
        for date in ["2026-09-22", "2026-09-23"] {
            let day = parse(&fixture(date));
            let written = day.to_markdown();
            assert_eq!(parse(&written), day, "{date}");
            // Writing is canonical: a second round changes nothing.
            assert_eq!(parse(&written).to_markdown(), written, "{date}");
        }
    }

    #[test]
    fn reformatted_file_becomes_canonical() {
        let written = parse(&fixture("2026-09-22")).to_markdown();
        let front_matter = written.split("---\n").nth(1).unwrap();
        assert_eq!(
            front_matter,
            "format: 1\n\
             date: \"2026-09-22\"\n\
             kind: \"work\"\n\
             location: \"office\"\n\
             tags: [\"review\"]\n\
             work: { start: \"08:45\", end: \"17:15\" }\n\
             blocks:\n  \
             - { id: \"p1q2\", start: \"08:45\", end: \"10:00\", project: \"webshop\" }\n  \
             - { id: \"r3s4\", start: \"10:00\", end: \"10:15\", project: \"meetings\" }\n  \
             - { id: \"t5u6\", start: \"10:15\", end: \"12:15\", project: \"infra\" }\n  \
             - { id: \"v7w8\", start: \"12:15\", end: \"13:00\", project: \"pause\" }\n  \
             - { id: \"x9y0\", start: \"13:00\", end: \"17:15\", project: \"webshop\" }\n\
             mood: \"focused\"\n"
        );
        // The missing date heading is added.
        assert!(written.contains("---\n\n# 2026-09-22\n\nOffice day"));
    }

    #[test]
    fn minimal_day() {
        let day = parse("---\nformat: 1\ndate: \"2026-01-05\"\n---\n");
        assert_eq!(
            day.to_markdown(),
            "---\nformat: 1\ndate: \"2026-01-05\"\nkind: \"work\"\n---\n\n# 2026-01-05\n"
        );
        assert_eq!(Day::new(day.date), day);
    }

    #[test]
    fn unknown_fields_and_odd_strings_survive() {
        let text = "---\nformat: 1\ndate: \"2026-01-05\"\nkind: \"say \\\"no\\\"\"\n\
                    tags: [\"a: b\", \"#x\"]\nwork: { start: \"09:00\" }\n\
                    \"yes\": 1\nnested: { list: [1, \"two\", null], flag: true }\n---\n";
        let day = parse(text);
        let written = day.to_markdown();
        assert!(
            written.contains("work: { start: \"09:00\" }\n"),
            "{written}"
        );
        assert!(written.contains("\"yes\": 1\n"), "{written}");
        assert_eq!(parse(&written), day);
    }

    #[test]
    fn untitled_blocks_and_texts() {
        let text = "---\nformat: 1\ndate: \"2026-01-05\"\nblocks:\n  \
                    - { id: \"aaaa\", start: \"09:00\", end: \"10:00\", project: \"x\" }\n  \
                    - { id: \"bbbb\", start: \"10:00\", end: \"11:00\", project: \"x\" }\n---\n\
                    \n\n\n## {#bbbb}\n\n\nText\n\n\n";
        let written = parse(text).to_markdown();
        // Missing headings are added, blank lines are made regular.
        assert!(
            written.ends_with("# 2026-01-05\n\n## {#aaaa}\n\n## {#bbbb}\n\nText\n"),
            "{written}"
        );
    }

    #[test]
    fn duplicate_heading_stays_text() {
        // The second heading of block bbbb is text of aaaa. Written in order
        // of start times, it would come before the real one.
        let text = "---\nformat: 1\ndate: \"2026-01-05\"\nblocks:\n  \
                    - { id: \"aaaa\", start: \"09:00\", end: \"10:00\", project: \"x\" }\n  \
                    - { id: \"bbbb\", start: \"10:00\", end: \"11:00\", project: \"x\" }\n---\n\
                    \n## Later {#bbbb}\n\nKeep me.\n\n## Earlier {#aaaa}\n\nNotes\n\n## Copy {#bbbb}\n\nCopied.\n";
        let day = parse(text);
        let reread = parse(&day.to_markdown());
        assert_eq!(reread.blocks[1], day.blocks[1]);
        assert_eq!(
            reread.blocks[0].text,
            "Notes\n\n\\## Copy {#bbbb}\n\nCopied."
        );
    }
}
