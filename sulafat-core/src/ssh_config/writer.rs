//! Surgical, in-memory editing of parsed [`Segment`]s.
//!
//! Every function here mutates only the lines that actually changed: unmapped directives,
//! comments and blank lines inside an edited block are left untouched, and every other block in
//! the file is never even visited. File I/O (atomic write, backup, permissions) lives in
//! [`super::SshConfig::save`], not here — this module only ever touches the in-memory model.

use super::{BlockLine, KnownDirective, ManagedBlock, RawLine, Segment, SshHost};

fn find_managed_index(segments: &[Segment], alias: &str) -> Option<usize> {
    segments
        .iter()
        .position(|s| matches!(s, Segment::Managed(b) if b.alias == alias))
}

fn find_known_index(block: &ManagedBlock, directive: KnownDirective) -> Option<usize> {
    block
        .lines
        .iter()
        .position(|l| matches!(l, BlockLine::Known { directive: d, .. } if *d == directive))
}

fn last_known_index(block: &ManagedBlock) -> Option<usize> {
    block
        .lines
        .iter()
        .rposition(|l| matches!(l, BlockLine::Known { .. }))
}

/// Split a raw line into `(leading_whitespace_and_keyword, terminator)`, so a replacement value
/// can be spliced in while keeping the original indentation, keyword casing and line ending.
fn split_keyword_and_terminator(line: &str) -> (&str, &str) {
    let content = super::parser::strip_terminator(line);
    let terminator = &line[content.len()..];
    let keyword_start = content.len() - content.trim_start().len();
    let after_ws = &content[keyword_start..];
    let keyword_len = after_ws.find(char::is_whitespace).unwrap_or(after_ws.len());
    (&content[..keyword_start + keyword_len], terminator)
}

fn replace_line_value(line: &mut RawLine, new_value: &str) {
    let (keyword_part, terminator) = split_keyword_and_terminator(&line.0);
    line.0 = format!("{keyword_part} {new_value}{terminator}");
}

fn set_header_alias(header: &mut RawLine, new_alias: &str) {
    let (keyword_part, terminator) = split_keyword_and_terminator(&header.0);
    header.0 = format!("{keyword_part} {new_alias}{terminator}");
}

fn build_known_line(directive: KnownDirective, value: &str) -> RawLine {
    RawLine(format!("    {} {value}\n", directive.keyword()))
}

fn apply_known_field(block: &mut ManagedBlock, directive: KnownDirective, new_value: Option<&str>) {
    match (find_known_index(block, directive), new_value) {
        (Some(idx), None) => {
            block.lines.remove(idx);
        }
        (Some(idx), Some(v)) => {
            if let BlockLine::Known { line, .. } = &mut block.lines[idx] {
                replace_line_value(line, v);
            }
        }
        (None, Some(v)) => {
            let insert_at = last_known_index(block)
                .map(|i| i + 1)
                .unwrap_or(block.lines.len());
            block.lines.insert(
                insert_at,
                BlockLine::Known {
                    directive,
                    line: RawLine(format!(
                        "    {} {v}{}",
                        directive.keyword(),
                        line_ending(block)
                    )),
                },
            );
        }
        (None, None) => {}
    }
}

fn line_ending(block: &ManagedBlock) -> &'static str {
    if block.header.0.ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

// Longest-common-subsequence anchors, using linear space (Hirschberg), so advanced
// options can grow without allocating a quadratic table. Equal lines keep their
// original slots, including repeated directives and their exact raw terminators.
fn lcs_lengths(a: &[&str], b: &[&str], reverse: bool) -> Vec<usize> {
    let mut row = vec![0; b.len() + 1];
    for i in 0..a.len() {
        let mut diagonal = 0;
        for j in 0..b.len() {
            let old = row[j + 1];
            let (ai, bj) = if reverse {
                (a.len() - 1 - i, b.len() - 1 - j)
            } else {
                (i, j)
            };
            row[j + 1] = if a[ai] == b[bj] {
                diagonal + 1
            } else {
                row[j + 1].max(row[j])
            };
            diagonal = old;
        }
    }
    row
}

fn anchors(a: &[&str], b: &[&str], offset: (usize, usize), out: &mut Vec<(usize, usize)>) {
    if a.is_empty() || b.is_empty() {
        return;
    }
    if a.len() == 1 {
        if let Some(j) = b.iter().position(|line| *line == a[0]) {
            out.push((offset.0, offset.1 + j));
        }
        return;
    }
    // Trim unchanged ends before the dynamic-programming step. A small edit to
    // a large advanced-options block should only compare the changed middle.
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    if prefix + suffix > 0 {
        out.extend((0..prefix).map(|i| (offset.0 + i, offset.1 + i)));
        anchors(
            &a[prefix..a.len() - suffix],
            &b[prefix..b.len() - suffix],
            (offset.0 + prefix, offset.1 + prefix),
            out,
        );
        out.extend((0..suffix).map(|i| {
            (
                offset.0 + a.len() - suffix + i,
                offset.1 + b.len() - suffix + i,
            )
        }));
        return;
    }
    let mid = a.len() / 2;
    let split = {
        let left = lcs_lengths(&a[..mid], b, false);
        let right = lcs_lengths(&a[mid..], b, true);
        (0..=b.len())
            .max_by_key(|&j| left[j] + right[b.len() - j])
            .unwrap()
    };
    anchors(&a[..mid], &b[..split], offset, out);
    anchors(
        &a[mid..],
        &b[split..],
        (offset.0 + mid, offset.1 + split),
        out,
    );
}

/// Edit advanced text in place around unchanged lines. Replacements reuse old slots;
/// insertions go before the next unchanged advanced line, or after the last old slot.
/// The text editor has no markers for known fields: it cannot express moving a line
/// across one of those fields. Existing lines are never regrouped around them.
fn replace_extra_lines(block: &mut ManagedBlock, extra: &str) {
    let old: Vec<(usize, RawLine)> = block
        .lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            if let BlockLine::Other(line) = l {
                Some((i, line.clone()))
            } else {
                None
            }
        })
        .collect();
    let old_text: Vec<&str> = old
        .iter()
        .map(|(_, line)| super::parser::strip_terminator(&line.0))
        .collect();
    if old_text.join("\n") == extra {
        return;
    }
    let new: Vec<&str> = if extra.is_empty() {
        vec![]
    } else {
        extra.split('\n').collect()
    };
    let mut matches = Vec::new();
    anchors(&old_text, &new, (0, 0), &mut matches);
    matches.push((old.len(), new.len()));
    let mut replacements: Vec<Option<RawLine>> = vec![None; block.lines.len()];
    let mut insertions: Vec<Vec<RawLine>> = vec![vec![]; block.lines.len() + 1];
    let mut start = (0, 0);
    for (end_old, end_new) in matches {
        let paired = (end_old - start.0).min(end_new - start.1);
        for k in 0..paired {
            let (idx, line) = &old[start.0 + k];
            let content = super::parser::strip_terminator(&line.0);
            replacements[*idx] = Some(RawLine(format!(
                "{}{}",
                new[start.1 + k],
                &line.0[content.len()..]
            )));
        }
        let boundary = if end_old > start.0 {
            old[end_old - 1].0 + 1
        } else if end_old < old.len() {
            old[end_old].0
        } else {
            old.last()
                .map(|(idx, _)| idx + 1)
                .unwrap_or(block.lines.len())
        };
        for line in &new[start.1 + paired..end_new] {
            insertions[boundary].push(RawLine(format!("{line}{}", line_ending(block))));
        }
        if end_old < old.len() {
            replacements[old[end_old].0] = Some(old[end_old].1.clone());
        }
        start = (end_old + 1, end_new + 1);
    }
    let mut result = Vec::new();
    for (i, line) in block.lines.drain(..).enumerate() {
        result.extend(insertions[i].drain(..).map(BlockLine::Other));
        match line {
            BlockLine::Known { .. } => result.push(line),
            BlockLine::Other(_) => {
                if let Some(line) = replacements[i].take() {
                    result.push(BlockLine::Other(line));
                }
            }
        }
    }
    result.extend(
        insertions
            .last_mut()
            .unwrap()
            .drain(..)
            .map(BlockLine::Other),
    );
    block.lines = result;
}

fn port_value(host: &SshHost) -> Option<String> {
    host.port.map(|p| p.to_string())
}

fn rewrite_block(block: &mut ManagedBlock, host: &SshHost) {
    if host.alias != block.alias {
        set_header_alias(&mut block.header, &host.alias);
        block.alias = host.alias.clone();
    }
    let previous = super::host_from_managed(block);
    for (directive, old, new) in [
        (
            KnownDirective::HostName,
            previous.host_name.as_deref(),
            host.host_name.as_deref(),
        ),
        (
            KnownDirective::User,
            previous.user.as_deref(),
            host.user.as_deref(),
        ),
        (
            KnownDirective::Port,
            port_value(&previous).as_deref(),
            port_value(host).as_deref(),
        ),
        (
            KnownDirective::IdentityFile,
            previous.identity_file.as_deref(),
            host.identity_file.as_deref(),
        ),
        (
            KnownDirective::ProxyJump,
            previous.proxy_jump.as_deref(),
            host.proxy_jump.as_deref(),
        ),
    ] {
        if old != new {
            apply_known_field(block, directive, new);
        }
    }
    replace_extra_lines(block, &host.extra);
    // An insertion after an unterminated final line needs a separator. Otherwise
    // leave every original terminator untouched, including the last line at EOF.
    let ending = line_ending(block);
    if !block.lines.is_empty() && !block.header.0.ends_with('\n') {
        block.header.0.push_str(ending);
    }
    let len = block.lines.len();
    for line in block.lines.iter_mut().take(len.saturating_sub(1)) {
        let raw = match line {
            BlockLine::Known { line, .. } | BlockLine::Other(line) => line,
        };
        if !raw.0.ends_with('\n') {
            raw.0.push_str(ending);
        }
    }
    // Advanced edits may add/remove a first occurrence of a known directive.
    // Reclassify so a subsequent edit in the same model sees the saved text.
    block.lines = super::parser::classify_body(
        std::mem::take(&mut block.lines)
            .into_iter()
            .map(|line| match line {
                BlockLine::Known { line, .. } | BlockLine::Other(line) => line,
            })
            .collect(),
    );
}

fn build_new_block(host: &SshHost) -> ManagedBlock {
    let mut block = ManagedBlock {
        alias: host.alias.clone(),
        header: RawLine(format!("Host {}\n", host.alias)),
        lines: Vec::new(),
    };
    for (directive, value) in [
        (KnownDirective::HostName, host.host_name.clone()),
        (KnownDirective::User, host.user.clone()),
        (KnownDirective::Port, port_value(host)),
        (KnownDirective::IdentityFile, host.identity_file.clone()),
        (KnownDirective::ProxyJump, host.proxy_jump.clone()),
    ] {
        if let Some(v) = value {
            block.lines.push(BlockLine::Known {
                directive,
                line: build_known_line(directive, &v),
            });
        }
    }
    if !host.extra.is_empty() {
        for line in host.extra.split('\n') {
            block
                .lines
                .push(BlockLine::Other(RawLine(format!("{line}\n"))));
        }
    }
    block
}

fn last_raw_line_mut(segments: &mut [Segment]) -> Option<&mut RawLine> {
    match segments.last_mut()? {
        Segment::Raw(lines) => lines.last_mut(),
        Segment::Managed(block) => match block.lines.last_mut() {
            Some(BlockLine::Known { line, .. }) => Some(line),
            Some(BlockLine::Other(line)) => Some(line),
            None => Some(&mut block.header),
        },
    }
}

fn push_raw_line(segments: &mut Vec<Segment>, text: &str) {
    match segments.last_mut() {
        Some(Segment::Raw(lines)) => lines.push(RawLine(text.to_string())),
        _ => segments.push(Segment::Raw(vec![RawLine(text.to_string())])),
    }
}

/// Make sure a brand-new block can be appended cleanly: terminate the file's last line if it was
/// missing a trailing newline, then add one blank separator line (skipped for an empty file).
fn ensure_appendable(segments: &mut Vec<Segment>) {
    if segments.is_empty() {
        return;
    }
    if let Some(last) = last_raw_line_mut(segments) {
        if !last.0.ends_with('\n') {
            last.0.push('\n');
        }
    }
    push_raw_line(segments, "\n");
}

pub(super) fn upsert(segments: &mut Vec<Segment>, host: &SshHost) {
    match find_managed_index(segments, &host.alias) {
        Some(idx) => {
            if let Segment::Managed(block) = &mut segments[idx] {
                rewrite_block(block, host);
            }
        }
        None => {
            ensure_appendable(segments);
            segments.push(Segment::Managed(build_new_block(host)));
        }
    }
}

/// Upsert under a possibly-renamed alias: `previous_alias` locates the block to edit even when
/// `host.alias` differs from it (renaming), falling back to appending a new block otherwise.
pub(super) fn upsert_renaming(
    segments: &mut Vec<Segment>,
    previous_alias: Option<&str>,
    host: &SshHost,
) {
    let idx = previous_alias.and_then(|a| find_managed_index(segments, a));
    match idx {
        Some(idx) => {
            if let Segment::Managed(block) = &mut segments[idx] {
                rewrite_block(block, host);
            }
        }
        None => upsert(segments, host),
    }
}

pub(super) fn remove(segments: &mut Vec<Segment>, alias: &str) -> bool {
    match find_managed_index(segments, alias) {
        Some(idx) => {
            segments.remove(idx);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::parser::{parse, render};
    use super::*;

    fn parsed(text: &str) -> Vec<Segment> {
        parse(text)
    }

    #[test]
    fn editing_a_field_preserves_unrelated_lines() {
        let mut segments = parsed("# comment\nHost prod\n    HostName 10.0.0.1\n    Compression yes\n\nHost other\n    User x\n");
        // A real caller round-trips `extra` from the host it just loaded (here, "Compression
        // yes", the block's one unmapped directive) unless the user edited "Opções avançadas".
        let host = SshHost {
            alias: "prod".into(),
            host_name: Some("10.0.0.2".into()),
            extra: "    Compression yes".into(),
            ..Default::default()
        };
        upsert(&mut segments, &host);
        let out = render(&segments);
        assert_eq!(out, "# comment\nHost prod\n    HostName 10.0.0.2\n    Compression yes\n\nHost other\n    User x\n");
    }

    #[test]
    fn clearing_a_field_removes_its_line() {
        let mut segments = parsed("Host prod\n    HostName 10.0.0.1\n    User admin\n");
        let host = SshHost {
            alias: "prod".into(),
            host_name: Some("10.0.0.1".into()),
            user: None,
            ..Default::default()
        };
        upsert(&mut segments, &host);
        assert_eq!(render(&segments), "Host prod\n    HostName 10.0.0.1\n");
    }

    #[test]
    fn adding_a_new_field_inserts_after_known_lines() {
        let mut segments = parsed("Host prod\n    HostName 10.0.0.1\n    # note\n");
        // Round-trips the existing "# note" extra line, as a real caller would after loading it.
        let host = SshHost {
            alias: "prod".into(),
            host_name: Some("10.0.0.1".into()),
            user: Some("admin".into()),
            extra: "    # note".into(),
            ..Default::default()
        };
        upsert(&mut segments, &host);
        assert_eq!(
            render(&segments),
            "Host prod\n    HostName 10.0.0.1\n    User admin\n    # note\n"
        );
    }

    #[test]
    fn advanced_options_text_replaces_extra_lines_only() {
        let mut segments =
            parsed("Host prod\n    HostName 10.0.0.1\n    Compression yes\n    # old note\n");
        let host = SshHost {
            alias: "prod".into(),
            host_name: Some("10.0.0.1".into()),
            extra: "ServerAliveInterval 30\n# new note".into(),
            ..Default::default()
        };
        upsert(&mut segments, &host);
        assert_eq!(
            render(&segments),
            "Host prod\n    HostName 10.0.0.1\nServerAliveInterval 30\n# new note\n"
        );
    }

    #[test]
    fn new_host_is_appended_with_separator() {
        let mut segments = parsed("Host prod\n    HostName 10.0.0.1\n");
        let host = SshHost {
            alias: "staging".into(),
            host_name: Some("10.0.0.2".into()),
            ..Default::default()
        };
        upsert(&mut segments, &host);
        assert_eq!(
            render(&segments),
            "Host prod\n    HostName 10.0.0.1\n\nHost staging\n    HostName 10.0.0.2\n"
        );
    }

    #[test]
    fn new_host_on_file_missing_trailing_newline_still_terminates_previous_block() {
        let mut segments = parsed("Host prod\n    HostName 10.0.0.1");
        let host = SshHost {
            alias: "staging".into(),
            host_name: Some("10.0.0.2".into()),
            ..Default::default()
        };
        upsert(&mut segments, &host);
        assert_eq!(
            render(&segments),
            "Host prod\n    HostName 10.0.0.1\n\nHost staging\n    HostName 10.0.0.2\n"
        );
    }

    #[test]
    fn new_host_on_empty_file_has_no_leading_separator() {
        let mut segments = parsed("");
        let host = SshHost {
            alias: "staging".into(),
            host_name: Some("10.0.0.2".into()),
            ..Default::default()
        };
        upsert(&mut segments, &host);
        assert_eq!(render(&segments), "Host staging\n    HostName 10.0.0.2\n");
    }

    #[test]
    fn removing_a_host_deletes_its_whole_block() {
        let mut segments =
            parsed("Host prod\n    HostName 10.0.0.1\n\nHost staging\n    User dev\n");
        assert!(remove(&mut segments, "prod"));
        assert_eq!(render(&segments), "\nHost staging\n    User dev\n");
    }

    #[test]
    fn removing_an_unknown_alias_is_a_no_op_and_reports_false() {
        let mut segments = parsed("Host prod\n    HostName 10.0.0.1\n");
        assert!(!remove(&mut segments, "ghost"));
        assert_eq!(render(&segments), "Host prod\n    HostName 10.0.0.1\n");
    }

    #[test]
    fn renaming_alias_updates_header_and_keeps_matching_by_previous_alias() {
        let mut segments = parsed("Host prod\n    HostName 10.0.0.1\n");
        let host = SshHost {
            alias: "prod-db".into(),
            host_name: Some("10.0.0.1".into()),
            ..Default::default()
        };
        upsert_renaming(&mut segments, Some("prod"), &host);
        assert_eq!(render(&segments), "Host prod-db\n    HostName 10.0.0.1\n");
    }
}
