//! Code blocks: their language, code and fences.

use std::ops::Range;

use super::{Frame, line_start, next_line_start};

/// A code block, fenced or indented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlock {
    /// With the fences.
    pub range: Range<usize>,
    /// As named after the opening fence, like `rust` in ```` ```rust ````,
    /// or empty.
    pub language: String,
    /// Without the fences.
    pub code: Range<usize>,
    /// The lines of the opening fence and, if there is one, the closing
    /// fence, with their line breaks. None for indented code.
    pub fences: Vec<Range<usize>>,
}

/// The code block of `frame`, with the `info` string after its opening
/// fence, if it has fences.
pub(super) fn code_block(text: &str, frame: &Frame, info: Option<&str>) -> CodeBlock {
    let range = frame.range.clone();
    let Some(info) = info else {
        let code = match (frame.children.first(), frame.children.last()) {
            (Some(first), Some(last)) => first.start..last.end,
            _ => range.clone(),
        };
        return CodeBlock {
            range,
            language: String::new(),
            code,
            fences: Vec::new(),
        };
    };
    // Info strings may go on, as in `rust,ignore` or `js title="a"`.
    let language = info
        .split(|c: char| c == ',' || c.is_whitespace())
        .next()
        .unwrap_or_default()
        .to_owned();
    let opening_end = next_line_start(text, range.start);
    let code = match (frame.children.first(), frame.children.last()) {
        (Some(first), Some(last)) => first.start..last.end,
        _ => opening_end..opening_end,
    };
    let opening = line_start(text, range.start)..opening_end;
    let last = line_start(text, range.end - 1);
    let last_line = text[last..range.end].trim_start_matches([' ', '>']);
    let closing = (last >= opening_end
        && (last_line.starts_with("```") || last_line.starts_with("~~~")))
    .then(|| last..next_line_start(text, range.end - 1));
    let fences = std::iter::once(opening).chain(closing).collect();
    CodeBlock {
        range,
        language,
        code,
        fences,
    }
}

#[cfg(test)]
mod tests {
    use crate::markdown::{MarkdownMode, markdown_formatting};

    #[test]
    fn code_blocks_and_their_fences() {
        let text = "```rust,ignore\nlet a = 1;\nlet b = 2;\n```\n\n~~~\n~~~\n\n    indented\n\n```sh\nopen";
        let blocks = markdown_formatting(text, MarkdownMode::Block).code_blocks;
        let parts: Vec<(&str, &str, Vec<&str>)> = blocks
            .iter()
            .map(|block| {
                let fences = block
                    .fences
                    .iter()
                    .map(|fence| &text[fence.clone()])
                    .collect();
                (block.language.as_str(), &text[block.code.clone()], fences)
            })
            .collect();
        assert_eq!(
            parts,
            [
                (
                    "rust",
                    "let a = 1;\nlet b = 2;\n",
                    vec!["```rust,ignore\n", "```\n"]
                ),
                ("", "", vec!["~~~\n", "~~~\n"]),
                ("", "indented\n", vec![]),
                // Not closed.
                ("sh", "open", vec!["```sh\n"]),
            ]
        );
    }
}
