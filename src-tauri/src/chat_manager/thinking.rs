#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ThinkingSplit {
    pub content: String,
    pub reasoning: String,
}

impl ThinkingSplit {
    pub fn merge_reasoning(mut self, explicit_reasoning: Option<&str>) -> Self {
        if let Some(reasoning) = explicit_reasoning
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if self.reasoning.trim().is_empty() {
                self.reasoning = reasoning.to_string();
            } else if self.reasoning.trim() != reasoning {
                self.reasoning.push_str("\n\n");
                self.reasoning.push_str(reasoning);
            }
        }
        self
    }
}

#[derive(Debug, Default, Clone)]
pub struct ThinkingTagStreamParser {
    in_think: bool,
    close_tag: Option<&'static str>,
    pending: String,
}

const TAG_PAIRS: [(&str, &str); 6] = [
    ("<think>", "</think>"),
    ("<thinking>", "</thinking>"),
    ("<reason>", "</reason>"),
    ("<reasoning>", "</reasoning>"),
    ("<|channel>thought", "<channel|>"),
    ("<|channel>", "<channel|>"),
];

fn partial_suffix_len(buffer: &str, tag: &str) -> usize {
    let buffer_lower = buffer.to_ascii_lowercase();
    let max_len = buffer_lower.len().min(tag.len().saturating_sub(1));
    let mut best = 0;

    for (start, _) in buffer_lower.char_indices() {
        let suffix = &buffer_lower[start..];
        let suffix_len = suffix.len();
        if suffix_len <= max_len && tag.starts_with(suffix) {
            best = best.max(suffix_len);
        }
    }

    best
}

fn partial_suffix_len_any(buffer: &str, tags: &[&str]) -> usize {
    tags.iter()
        .map(|tag| partial_suffix_len(buffer, tag))
        .max()
        .unwrap_or(0)
}

fn earliest_open_tag(buffer: &str) -> Option<(usize, &'static str, &'static str)> {
    let buffer_lower = buffer.to_ascii_lowercase();
    TAG_PAIRS
        .iter()
        .filter_map(|(open_tag, close_tag)| {
            buffer_lower
                .find(open_tag)
                .map(|index| (index, *open_tag, *close_tag))
        })
        .min_by_key(|(index, _, _)| *index)
}

impl ThinkingTagStreamParser {
    /// Start already inside a reasoning block. Used for "forced-open thinking"
    /// templates (Qwen3 "Thinking" / 2507+, DeepSeek-R1 style) whose generation
    /// prefix emits the opening `<think>` into the prompt, so the model output
    /// carries only the reasoning body and a trailing `</think>` with no opening
    /// tag for the parser to key off.
    pub fn started_in_reasoning(close_tag: &'static str) -> Self {
        Self {
            in_think: true,
            close_tag: Some(close_tag),
            pending: String::new(),
        }
    }

    /// The close tag still awaited if this parser ended inside an unclosed
    /// reasoning block, otherwise `None`.
    pub fn ends_in_reasoning(&self) -> Option<&'static str> {
        if self.in_think {
            self.close_tag
        } else {
            None
        }
    }

    pub fn feed(&mut self, chunk: &str) -> ThinkingSplit {
        self.pending.push_str(chunk);
        let mut split = ThinkingSplit::default();

        loop {
            if self.in_think {
                let close_tag = self.close_tag.expect("close tag must exist when in_think");
                let pending_lower = self.pending.to_ascii_lowercase();
                if let Some(index) = pending_lower.find(close_tag) {
                    split.reasoning.push_str(&self.pending[..index]);
                    self.pending.drain(..index + close_tag.len());
                    self.in_think = false;
                    self.close_tag = None;
                    continue;
                }

                let keep = partial_suffix_len(&self.pending, close_tag);
                let emit_len = self.pending.len().saturating_sub(keep);
                if emit_len == 0 {
                    break;
                }
                split.reasoning.push_str(&self.pending[..emit_len]);
                self.pending.drain(..emit_len);
                break;
            }

            if let Some((index, open_tag, close_tag)) = earliest_open_tag(&self.pending) {
                split.content.push_str(&self.pending[..index]);
                self.pending.drain(..index + open_tag.len());
                self.in_think = true;
                self.close_tag = Some(close_tag);
                continue;
            }

            let open_tags = TAG_PAIRS.map(|(open_tag, _)| open_tag);
            let keep = partial_suffix_len_any(&self.pending, &open_tags);
            let emit_len = self.pending.len().saturating_sub(keep);
            if emit_len == 0 {
                break;
            }
            split.content.push_str(&self.pending[..emit_len]);
            self.pending.drain(..emit_len);
            break;
        }

        split
    }

    pub fn finish(&mut self) -> ThinkingSplit {
        let mut split = ThinkingSplit::default();
        if self.in_think {
            split.reasoning.push_str(&self.pending);
        } else {
            split.content.push_str(&self.pending);
        }
        self.close_tag = None;
        self.pending.clear();
        split
    }
}

/// Detect whether a fully rendered prompt ends inside an unclosed reasoning
/// block, returning the close tag the model output is expected to emit without a
/// matching opening tag. Balanced `<think>…</think>` pairs from prior turns are
/// ignored — only a trailing, still-open block counts.
pub fn trailing_open_reasoning_tag(prompt: &str) -> Option<&'static str> {
    let mut parser = ThinkingTagStreamParser::default();
    parser.feed(prompt);
    parser.ends_in_reasoning()
}

fn split_thinking_tags_seeded(text: &str, forced_open: Option<&'static str>) -> ThinkingSplit {
    let mut parser = match forced_open {
        Some(close_tag) => ThinkingTagStreamParser::started_in_reasoning(close_tag),
        None => ThinkingTagStreamParser::default(),
    };
    let mut split = parser.feed(text);
    let tail = parser.finish();
    split.content.push_str(&tail.content);
    split.reasoning.push_str(&tail.reasoning);
    split
}

pub fn split_thinking_tags(text: &str) -> ThinkingSplit {
    split_thinking_tags_seeded(text, None)
}

pub fn normalize_thinking_content(
    content: Option<&str>,
    explicit_reasoning: Option<&str>,
) -> ThinkingSplit {
    normalize_thinking_content_seeded(content, explicit_reasoning, None)
}

/// Like [`normalize_thinking_content`], but when `forced_open` is `Some` the
/// content channel is parsed as if it already began inside a reasoning block —
/// for templates that emit the opening `<think>` into the prompt.
pub fn normalize_thinking_content_seeded(
    content: Option<&str>,
    explicit_reasoning: Option<&str>,
    forced_open: Option<&'static str>,
) -> ThinkingSplit {
    let mut split = content
        .map(|text| split_thinking_tags_seeded(text, forced_open))
        .unwrap_or_default()
        .merge_reasoning(explicit_reasoning);

    split.content = split.content.trim().to_string();
    split.reasoning = split.reasoning.trim().to_string();
    split
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_trailing_open_think_from_forced_open_template() {
        // Qwen3 "Thinking"/2507 generation prefix opens <think> without closing.
        let prompt = "<|im_start|>assistant\n<think>\n";
        assert_eq!(trailing_open_reasoning_tag(prompt), Some("</think>"));
    }

    #[test]
    fn ignores_balanced_think_pairs_from_history() {
        let prompt = "<|im_start|>assistant\n<think>\nprior\n</think>\n\nanswer<|im_end|>\n";
        assert_eq!(trailing_open_reasoning_tag(prompt), None);
    }

    #[test]
    fn seeded_parser_captures_leading_reasoning_without_open_tag() {
        // Model output when the opening <think> lived in the prompt.
        // Raw split preserves whitespace; trimming is normalize's job.
        let split = split_thinking_tags_seeded("weighing options\n</think>\n\nFinal answer.", Some("</think>"));
        assert_eq!(split.reasoning, "weighing options\n");
        assert_eq!(split.content, "\n\nFinal answer.");
    }

    #[test]
    fn seeded_parser_handles_split_streaming_chunks() {
        let mut parser = ThinkingTagStreamParser::started_in_reasoning("</think>");
        let mut reasoning = String::new();
        let mut content = String::new();
        for chunk in ["thinking ", "out lou", "d</thi", "nk>visible"] {
            let split = parser.feed(chunk);
            reasoning.push_str(&split.reasoning);
            content.push_str(&split.content);
        }
        let tail = parser.finish();
        reasoning.push_str(&tail.reasoning);
        content.push_str(&tail.content);
        assert_eq!(reasoning, "thinking out loud");
        assert_eq!(content, "visible");
    }

    #[test]
    fn without_forced_open_leading_reasoning_leaks_to_content() {
        // Regression guard: this is the broken behavior the seeding fixes.
        let split = split_thinking_tags("weighing options\n</think>\n\nFinal answer.");
        assert!(split.reasoning.is_empty());
        assert!(split.content.contains("weighing options"));
    }

    #[test]
    fn normalize_seeded_moves_body_to_reasoning() {
        let split = normalize_thinking_content_seeded(
            Some("step one\nstep two\n</think>\n\nDone."),
            None,
            Some("</think>"),
        );
        assert_eq!(split.reasoning, "step one\nstep two");
        assert_eq!(split.content, "Done.");
    }
}
