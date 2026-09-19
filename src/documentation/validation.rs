//! Validation for the complete rendered documentation graph.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::{language::LanguageCatalog, migration::MigrationCatalog, stdlib::StandardLibrary};

use super::{DocumentationPage, DocumentationReference, intra_doc};

pub(super) fn validate(
    reference: &DocumentationReference,
    library: &StandardLibrary,
) -> Vec<String> {
    // Exhaustive graph validation covers page identity, prose links, and
    // lexical code links. Focused tests exercise semantic example links
    // without recompiling every example in the catalog.
    let reference = reference.with_lexical_examples();
    let reference = &reference;
    let mut errors = Vec::new();
    errors.extend(
        LanguageCatalog::new()
            .validate()
            .into_iter()
            .map(|error| format!("language catalog: {error}")),
    );
    errors.extend(
        library
            .validate()
            .into_iter()
            .map(|error| format!("standard-library catalog: {error}")),
    );
    errors.extend(
        MigrationCatalog::default()
            .validate()
            .into_iter()
            .map(|error| format!("migration catalog: {error}")),
    );

    let index = reference.index();
    let mut indexed_uris = HashSet::new();
    let mut pages = BTreeMap::new();
    insert_page(reference, "/index.md", None, &mut pages, &mut errors);
    for entry in &index {
        if !indexed_uris.insert(entry.uri.as_str()) {
            errors.push(format!("duplicate documentation URI `{}`", entry.uri));
        }
        if entry.title.trim().is_empty() {
            errors.push(format!("documentation page `{}` has no title", entry.uri));
        }
        if entry.kind.trim().is_empty() {
            errors.push(format!("documentation page `{}` has no kind", entry.uri));
        }
        if entry.summary.trim().is_empty() {
            errors.push(format!("documentation page `{}` has no summary", entry.uri));
        }
        if entry
            .signature
            .as_deref()
            .is_some_and(|signature| signature.trim().is_empty())
        {
            errors.push(format!(
                "documentation page `{}` has an empty signature",
                entry.uri
            ));
        }
        insert_page(
            reference,
            &entry.uri,
            Some(entry.title.as_str()),
            &mut pages,
            &mut errors,
        );
    }

    for page in pages.values() {
        validate_page_links(page, &pages, library, &mut errors);
    }
    errors
}

fn insert_page(
    reference: &DocumentationReference,
    uri: &str,
    indexed_title: Option<&str>,
    pages: &mut BTreeMap<String, DocumentationPage>,
    errors: &mut Vec<String>,
) {
    let Some(page) = reference.page(uri) else {
        errors.push(format!("documentation index links to missing page `{uri}`"));
        return;
    };
    if page.uri != uri {
        errors.push(format!(
            "documentation page `{uri}` reports canonical URI `{}`",
            page.uri
        ));
    }
    if let Some(indexed_title) = indexed_title
        && page.title != indexed_title
    {
        errors.push(format!(
            "documentation page `{uri}` is titled `{}` but its index entry is titled `{indexed_title}`",
            page.title
        ));
    }
    if page.markdown.trim().is_empty() {
        errors.push(format!("documentation page `{uri}` has no Markdown"));
    }
    if pages.insert(uri.to_owned(), page).is_some() {
        errors.push(format!(
            "documentation page `{uri}` is rendered more than once"
        ));
    }
}

fn validate_page_links(
    page: &DocumentationPage,
    pages: &BTreeMap<String, DocumentationPage>,
    library: &StandardLibrary,
    errors: &mut Vec<String>,
) {
    for label in intra_doc::unresolved_links(&page.markdown) {
        errors.push(format!(
            "documentation page `{}` has unresolved intra-doc link `[`{label}`]`",
            page.uri
        ));
    }
    for label in intra_doc::resolvable_plain_code_spans(&page.markdown, library) {
        errors.push(format!(
            "documentation page `{}` uses plain code span `` `{label}` `` for a known symbol; write `` [`{label}`] ``",
            page.uri
        ));
    }
    for target in link_targets(&page.markdown) {
        if is_external_target(&target) {
            continue;
        }
        let (path, fragment) = target
            .split_once('#')
            .map_or((target.as_str(), None), |(path, fragment)| {
                (path, Some(fragment))
            });
        let target_uri = match resolve_uri(&page.uri, path) {
            Ok(uri) => uri,
            Err(error) => {
                errors.push(format!(
                    "documentation page `{}` has invalid link `{target}`: {error}",
                    page.uri
                ));
                continue;
            }
        };
        let Some(target_page) = pages.get(&target_uri) else {
            errors.push(format!(
                "documentation page `{}` links to missing page `{target_uri}` through `{target}`",
                page.uri
            ));
            continue;
        };
        if let Some(fragment) = fragment
            && !fragment.is_empty()
            && !heading_anchors(&target_page.markdown).contains(fragment)
        {
            errors.push(format!(
                "documentation page `{}` links to missing heading `#{fragment}` on `{target_uri}`",
                page.uri
            ));
        }
    }
}

fn link_targets(markdown: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut fenced = false;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        targets.extend(delimited_targets(line, "](", ')'));
        targets.extend(quoted_attribute_targets(line, "href=\""));
    }
    targets
}

fn delimited_targets(line: &str, opening: &str, closing: char) -> Vec<String> {
    let mut targets = Vec::new();
    let mut rest = line;
    while let Some((_, after_opening)) = rest.split_once(opening) {
        let Some((target, after_target)) = after_opening.split_once(closing) else {
            break;
        };
        targets.push(target.trim_matches(['<', '>']).to_owned());
        rest = after_target;
    }
    targets
}

fn quoted_attribute_targets(line: &str, opening: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut rest = line;
    while let Some((_, after_opening)) = rest.split_once(opening) {
        let Some((target, after_target)) = after_opening.split_once('"') else {
            break;
        };
        targets.push(target.to_owned());
        rest = after_target;
    }
    targets
}

fn is_external_target(target: &str) -> bool {
    target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
        || target.starts_with("command:")
}

fn resolve_uri(current_uri: &str, target: &str) -> Result<String, &'static str> {
    if target.is_empty() {
        return Ok(current_uri.to_owned());
    }
    let mut segments = if target.starts_with('/') {
        Vec::new()
    } else {
        let mut segments = current_uri
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        segments.pop();
        segments
    };
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err("the path escapes the reference root");
                }
            }
            segment => segments.push(segment),
        }
    }
    Ok(format!("/{}", segments.join("/")))
}

fn heading_anchors(markdown: &str) -> BTreeSet<String> {
    let mut anchors = BTreeSet::new();
    let mut duplicates = HashMap::<String, usize>::new();
    let mut fenced = false;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let Some(heading) = trimmed.strip_prefix('#') else {
            continue;
        };
        let heading = heading.trim_start_matches('#').trim();
        if heading.is_empty() {
            continue;
        }
        let base = markdown_anchor(heading);
        let duplicate = duplicates.entry(base.clone()).or_default();
        let anchor = if *duplicate == 0 {
            base
        } else {
            format!("{base}-{duplicate}")
        };
        *duplicate += 1;
        anchors.insert(anchor);
    }
    anchors
}

fn markdown_anchor(heading: &str) -> String {
    let mut anchor = String::new();
    let mut pending_separator = false;
    for character in heading.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            if pending_separator && !anchor.is_empty() {
                anchor.push('-');
            }
            pending_separator = false;
            anchor.push(character);
        } else if character.is_whitespace() || character == '-' {
            pending_separator = true;
        }
    }
    anchor
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    #[test]
    fn complete_reference_graph_is_valid() {
        let reference = DocumentationReference::default();
        let errors = reference.validate();
        assert!(errors.is_empty(), "{errors:#?}");
    }

    #[test]
    fn relative_paths_and_fragments_resolve_canonically() {
        assert_eq!(
            resolve_uri(
                "/stdlib/types/Duration/methods/fromSeconds.md",
                "../index.md"
            ),
            Ok("/stdlib/types/Duration/index.md".to_owned())
        );
        assert_eq!(
            resolve_uri("/guides/asl-porting.md", ""),
            Ok("/guides/asl-porting.md".to_owned())
        );
        assert!(resolve_uri("/index.md", "../outside.md").is_err());
    }

    #[test]
    fn heading_anchors_ignore_code_and_disambiguate_duplicates() {
        let anchors = heading_anchors(
            "# Timer state\n\n```splitscript\n# not a heading\n```\n\n## Timer state\n",
        );
        assert_eq!(
            anchors,
            BTreeSet::from(["timer-state".to_owned(), "timer-state-1".to_owned()])
        );
    }

    #[test]
    fn page_validation_reports_missing_documents_and_fragments() {
        let source = DocumentationPage {
            uri: "/source.md".to_owned(),
            title: "Source".to_owned(),
            markdown: concat!(
                "# Source\n\n",
                "[Missing page](missing.md)\n\n",
                "<a href=\"target.md#missing-heading\">Missing heading</a>\n",
            )
            .to_owned(),
        };
        let target = DocumentationPage {
            uri: "/target.md".to_owned(),
            title: "Target".to_owned(),
            markdown: "# Existing heading\n".to_owned(),
        };
        let pages = BTreeMap::from([
            (source.uri.clone(), source.clone()),
            (target.uri.clone(), target),
        ]);
        let mut errors = Vec::new();
        validate_page_links(&source, &pages, &StandardLibrary::new(), &mut errors);
        assert_eq!(errors.len(), 2, "{errors:#?}");
        assert!(errors[0].contains("missing page `/missing.md`"));
        assert!(errors[1].contains("missing heading `#missing-heading`"));
    }

    #[test]
    fn rendered_reference_snapshot_is_stable() {
        let reference = DocumentationReference::default();
        let snapshot = reference_snapshot(&reference);
        assert_eq!(
            (snapshot.page_count, snapshot.fingerprint),
            (570, 13_514_234_740_956_009_386)
        );
    }

    #[derive(Debug)]
    struct ReferenceSnapshot {
        page_count: usize,
        fingerprint: u64,
    }

    fn reference_snapshot(reference: &DocumentationReference) -> ReferenceSnapshot {
        let index = reference.index();
        let mut rendered = String::new();
        let root = reference.page("/index.md").expect("reference root");
        writeln!(rendered, "{}\t{}", root.uri, root.title).unwrap();
        rendered.push_str(&root.markdown);
        for entry in &index {
            writeln!(
                rendered,
                "\n{}\t{}\t{}\t{}\t{}",
                entry.uri,
                entry.title,
                entry.kind,
                entry.summary,
                entry.signature.as_deref().unwrap_or_default(),
            )
            .unwrap();
        }
        ReferenceSnapshot {
            page_count: index.len() + 1,
            fingerprint: rendered.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
            }),
        }
    }
}
