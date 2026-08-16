use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    fs::File,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use ignore::WalkBuilder;
use serde_json::Value;
use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

use crate::model::{Edge, Evidence, Graph, Node, NodeKind, Provenance, ScanSummary};

const MAX_DEPTH: usize = 32;
const MAX_ENTRIES: usize = 20_000;
const MAX_SOURCE_FILES: usize = 40;
const MAX_GRAPH_NODES: usize = 40;
const MAX_GRAPH_EDGES: usize = 400;
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;

const JS_QUERY: &str = r#"
(import_statement source: (string) @source) @statement
(export_statement source: (string) @source) @statement
(call_expression
  function: (identifier) @function
  arguments: (arguments (string) @source)
  (#eq? @function "require")) @statement
"#;

const PYTHON_QUERY: &str = r#"
(import_statement name: (dotted_name) @source) @statement
(import_statement name: (aliased_import name: (dotted_name) @source)) @statement
(import_from_statement module_name: [(dotted_name) (relative_import)] @source) @statement
"#;

const RUST_QUERY: &str = r#"
(use_declaration argument: (_) @source) @statement
(extern_crate_declaration name: (identifier) @source) @statement
(mod_item name: (identifier) @source) @statement
"#;

#[derive(Debug)]
pub enum ScanError {
    InvalidRoot(String),
    Limit(String),
    Infrastructure(String),
}

impl fmt::Display for ScanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRoot(message) => write!(formatter, "invalid project: {message}"),
            Self::Limit(message) => write!(formatter, "scan refused: {message}"),
            Self::Infrastructure(message) => write!(formatter, "scan failed: {message}"),
        }
    }
}

impl Error for ScanError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LanguageKind {
    JavaScript,
    TypeScript,
    Tsx,
    Python,
    Rust,
}

#[derive(Clone, Debug)]
struct SourceFile {
    relative: String,
    absolute: PathBuf,
    language: LanguageKind,
    source: String,
    line_count: u32,
}

#[derive(Clone, Debug)]
struct ImportFact {
    source_path: String,
    specifier: String,
    relationship: String,
    line_start: u32,
    line_end: u32,
    ordinal: usize,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum TargetKey {
    File(String),
    Package(String),
    Unresolved {
        source_path: String,
        line: u32,
        ordinal: usize,
        specifier: String,
    },
}

#[derive(Clone, Debug)]
struct ResolvedTarget {
    key: TargetKey,
    provenance: Provenance,
    confidence: f32,
}

impl ResolvedTarget {
    fn extracted(key: TargetKey) -> Self {
        Self {
            key,
            provenance: Provenance::Extracted,
            confidence: 1.0,
        }
    }

    fn inferred(key: TargetKey) -> Self {
        Self {
            key,
            provenance: Provenance::Inferred,
            confidence: 0.72,
        }
    }

    fn with_ambiguous_require(mut self, fact: &ImportFact) -> Self {
        if fact.relationship == "requires" {
            self.provenance = Provenance::Inferred;
            self.confidence = self.confidence.min(0.60);
        }
        self
    }
}

pub fn scan_project(root: impl AsRef<Path>) -> Result<Graph, ScanError> {
    let canonical_root = root
        .as_ref()
        .canonicalize()
        .map_err(|error| ScanError::InvalidRoot(error.to_string()))?;
    if !canonical_root.is_dir() {
        return Err(ScanError::InvalidRoot(format!(
            "{} is not a directory",
            root.as_ref().display()
        )));
    }
    if canonical_root.components().any(|component| {
        matches!(
            component,
            Component::Normal(name)
                if matches!(name.to_str(), Some(".ssh" | ".aws" | ".gnupg"))
        )
    }) {
        return Err(ScanError::InvalidRoot(
            "sensitive roots such as .ssh, .aws, and .gnupg are refused".into(),
        ));
    }
    let repository = canonical_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| ScanError::InvalidRoot("root has no UTF-8 display name".into()))?
        .to_owned();

    let (files, mut summary) = discover_sources(&canonical_root)?;
    let file_paths: BTreeSet<_> = files.iter().map(|file| file.relative.clone()).collect();
    let mut facts = Vec::new();

    for file in &files {
        let (mut extracted, warnings) = extract_imports(file)?;
        facts.append(&mut extracted);
        summary.parse_warnings += warnings;
    }
    facts.sort_by(|left, right| {
        left.source_path
            .cmp(&right.source_path)
            .then_with(|| left.line_start.cmp(&right.line_start))
            .then_with(|| left.line_end.cmp(&right.line_end))
            .then_with(|| left.ordinal.cmp(&right.ordinal))
            .then_with(|| left.specifier.cmp(&right.specifier))
    });

    let file_by_path: BTreeMap<_, _> = files
        .iter()
        .map(|file| (file.relative.clone(), file))
        .collect();
    let mut resolved = Vec::with_capacity(facts.len());
    let mut rust_relationships = BTreeSet::new();
    let mut target_keys = BTreeSet::new();
    for fact in facts {
        let source_file = file_by_path
            .get(&fact.source_path)
            .expect("facts originate from discovered files");
        let target = resolve_target(&canonical_root, source_file, &fact, &file_paths);
        if source_file.language == LanguageKind::Rust
            && target.key == TargetKey::File(fact.source_path.clone())
        {
            continue;
        }
        if source_file.language == LanguageKind::Rust
            && !rust_relationships.insert((
                fact.source_path.clone(),
                target.key.clone(),
                fact.relationship.clone(),
                fact.line_start,
                fact.line_end,
            ))
        {
            continue;
        }
        target_keys.insert(target.key.clone());
        resolved.push((fact, target));
    }
    if resolved.len() > MAX_GRAPH_EDGES {
        return Err(ScanError::Limit(format!(
            "{} static import edges exceed the first-version limit of {MAX_GRAPH_EDGES}",
            resolved.len()
        )));
    }

    let non_file_targets = target_keys
        .iter()
        .filter(|target| !matches!(target, TargetKey::File(_)))
        .count();
    if files.len() + non_file_targets > MAX_GRAPH_NODES {
        return Err(ScanError::Limit(format!(
            "{} graph nodes exceed the readable first-version limit of {MAX_GRAPH_NODES}; scan a smaller project or subdirectory",
            files.len() + non_file_targets
        )));
    }

    let mut ids = BTreeMap::new();
    for file in &files {
        ids.insert(
            TargetKey::File(file.relative.clone()),
            stable_id("N", &format!("file\0{}", file.relative)),
        );
    }
    for target in target_keys {
        ids.entry(target.clone()).or_insert_with(|| match &target {
            TargetKey::Package(package) => stable_id("X", &format!("package\0{package}")),
            TargetKey::Unresolved {
                source_path,
                line,
                ordinal,
                specifier,
            } => stable_id(
                "U",
                &format!("unresolved\0{source_path}\0{line}\0{ordinal}\0{specifier}"),
            ),
            TargetKey::File(path) => stable_id("N", &format!("file\0{path}")),
        });
    }

    let manifest_entries = manifest_entry_paths(&canonical_root, &file_paths);
    let mut nodes = Vec::new();
    for file in &files {
        let exact_entry = manifest_entries.contains(&file.relative)
            || file.relative.ends_with("/__main__.py")
            || file.relative == "__main__.py";
        nodes.push(Node {
            id: ids[&TargetKey::File(file.relative.clone())].clone(),
            group: group_for_path(&file.relative),
            label: label_for_path(&file.relative),
            kind: if exact_entry {
                NodeKind::Entry
            } else {
                NodeKind::Module
            },
            evidence: Evidence {
                path: file.relative.clone(),
                line_start: 1,
                line_end: file.line_count.max(1),
            },
        });
    }

    let mut first_fact_for_target = BTreeMap::new();
    for (fact, target) in &resolved {
        first_fact_for_target
            .entry(target.key.clone())
            .or_insert(fact);
    }
    for (target, id) in &ids {
        let Some(fact) = first_fact_for_target.get(target) else {
            continue;
        };
        match target {
            TargetKey::Package(package) => nodes.push(Node {
                id: id.clone(),
                group: "EXTERNAL".into(),
                label: package.clone(),
                kind: NodeKind::ExternalPackage,
                evidence: fact_evidence(fact),
            }),
            TargetKey::Unresolved { specifier, .. } => nodes.push(Node {
                id: id.clone(),
                group: "UNRESOLVED".into(),
                label: format!("? {specifier}"),
                kind: NodeKind::Unresolved,
                evidence: fact_evidence(fact),
            }),
            TargetKey::File(_) => {}
        }
    }
    nodes.sort_by(|left, right| {
        node_kind_order(&left.kind)
            .cmp(&node_kind_order(&right.kind))
            .then_with(|| left.evidence.path.cmp(&right.evidence.path))
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut edges = Vec::with_capacity(resolved.len());
    for (fact, target) in resolved {
        let source = ids[&TargetKey::File(fact.source_path.clone())].clone();
        let target_id = ids[&target.key].clone();
        let key = format!(
            "edge\0{source}\0{target_id}\0{}\0{}\0{}\0{}",
            fact.relationship, fact.source_path, fact.line_start, fact.ordinal
        );
        let evidence = fact_evidence(&fact);
        edges.push(Edge {
            id: stable_id("E", &key),
            source,
            target: target_id,
            relationship: fact.relationship,
            provenance: target.provenance,
            confidence: target.confidence,
            evidence,
            import_specifier: Some(fact.specifier),
        });
    }
    summary.inferred_edges = edges
        .iter()
        .filter(|edge| edge.provenance == Provenance::Inferred)
        .count() as u32;

    Ok(Graph {
        schema_version: 2,
        repository,
        nodes,
        edges,
        flows: Vec::new(),
        scan_summary: summary,
    })
}

fn discover_sources(root: &Path) -> Result<(Vec<SourceFile>, ScanSummary), ScanError> {
    let mut builder = WalkBuilder::new(root);
    builder
        .standard_filters(true)
        .hidden(false)
        .git_global(false)
        .require_git(false)
        .follow_links(false)
        .sort_by_file_path(|left, right| left.cmp(right))
        .filter_entry(|entry| entry.depth() == 0 || !excluded_entry(entry.path()));

    let mut candidates = Vec::new();
    let mut skipped = BTreeMap::new();
    let mut supported_seen = 0_u32;
    let mut visited = 0_usize;
    let mut traversal_errors = 0_u32;
    for result in builder.build() {
        visited += 1;
        if visited > MAX_ENTRIES {
            return Err(ScanError::Limit(format!(
                "more than {MAX_ENTRIES} filesystem entries were visited"
            )));
        }
        let entry = match result {
            Ok(entry) => entry,
            Err(_) => {
                traversal_errors += 1;
                continue;
            }
        };
        if entry.depth() > MAX_DEPTH {
            return Err(ScanError::Limit(format!(
                "filesystem depth exceeds the limit of {MAX_DEPTH} at a repository-relative entry"
            )));
        }
        if entry.file_type().is_some_and(|kind| kind.is_file())
            && language_for_path(entry.path()).is_some()
        {
            supported_seen += 1;
            let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
            if secret_path(entry.path()) || has_hidden_component(relative) {
                increment(&mut skipped, "hidden_or_secret");
            } else {
                candidates.push(entry.into_path());
            }
        }
    }
    if candidates.len() > MAX_SOURCE_FILES {
        return Err(ScanError::Limit(format!(
            "{} supported source files exceed the readable first-version limit of {MAX_SOURCE_FILES}; scan a smaller project or subdirectory",
            candidates.len()
        )));
    }

    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    for absolute in candidates {
        let canonical = match absolute.canonicalize() {
            Ok(path) if path.starts_with(root) => path,
            _ => {
                increment(&mut skipped, "unsafe_path");
                continue;
            }
        };
        let relative = match canonical.strip_prefix(root).ok().and_then(normalized_path) {
            Some(path) => path,
            None => {
                increment(&mut skipped, "non_utf8_path");
                continue;
            }
        };
        let language = language_for_path(&canonical).expect("candidate language was checked");
        let source = match read_bounded(&canonical) {
            Ok(source) => source,
            Err(ReadFailure::Oversized) => {
                increment(&mut skipped, "oversized");
                continue;
            }
            Err(ReadFailure::Binary) => {
                increment(&mut skipped, "binary");
                continue;
            }
            Err(ReadFailure::NonUtf8) => {
                increment(&mut skipped, "non_utf8_content");
                continue;
            }
            Err(ReadFailure::Io) => {
                increment(&mut skipped, "unreadable");
                continue;
            }
        };
        total_bytes = total_bytes.saturating_add(source.len() as u64);
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(ScanError::Limit(format!(
                "parsed source exceeds {} MiB",
                MAX_TOTAL_BYTES / 1024 / 1024
            )));
        }
        let line_count = source.lines().count().max(1) as u32;
        files.push(SourceFile {
            relative,
            absolute: canonical,
            language,
            source,
            line_count,
        });
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    let files_skipped = skipped.values().copied().sum();
    let files_scanned = files.len() as u32;
    Ok((
        files,
        ScanSummary {
            source: "local_static_scan".into(),
            files_discovered: supported_seen,
            files_scanned,
            files_skipped,
            skipped_by_reason: skipped,
            parse_warnings: 0,
            traversal_errors,
            inferred_edges: 0,
        },
    ))
}

#[derive(Clone, Copy, Debug)]
enum ReadFailure {
    Oversized,
    Binary,
    NonUtf8,
    Io,
}

fn read_bounded(path: &Path) -> Result<String, ReadFailure> {
    let file = File::open(path).map_err(|_| ReadFailure::Io)?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadFailure::Io)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(ReadFailure::Oversized);
    }
    if bytes.contains(&0) {
        return Err(ReadFailure::Binary);
    }
    String::from_utf8(bytes).map_err(|_| ReadFailure::NonUtf8)
}

fn extract_imports(file: &SourceFile) -> Result<(Vec<ImportFact>, u32), ScanError> {
    let language = tree_sitter_language(file.language);
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|error| ScanError::Infrastructure(error.to_string()))?;
    let tree = parser
        .parse(file.source.as_bytes(), None)
        .ok_or_else(|| ScanError::Infrastructure("parser returned no tree".into()))?;
    let parse_warnings = u32::from(tree.root_node().has_error());
    if file.language == LanguageKind::Rust {
        let facts = extract_rust_imports(file, &tree, &language)?;
        return Ok((facts, parse_warnings));
    }
    let query_source = if file.language == LanguageKind::Python {
        PYTHON_QUERY
    } else {
        JS_QUERY
    };
    let query = Query::new(&language, query_source)
        .map_err(|error| ScanError::Infrastructure(error.to_string()))?;
    let source_capture = query
        .capture_index_for_name("source")
        .expect("scanner query has source capture");
    let statement_capture = query
        .capture_index_for_name("statement")
        .expect("scanner query has statement capture");
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), file.source.as_bytes());
    let mut facts = Vec::new();
    let mut ordinal = 0_usize;
    while let Some(query_match) = matches.next() {
        let source_node = query_match
            .captures
            .iter()
            .find(|capture| capture.index == source_capture)
            .map(|capture| capture.node);
        let statement_node = query_match
            .captures
            .iter()
            .find(|capture| capture.index == statement_capture)
            .map(|capture| capture.node);
        let (Some(source_node), Some(statement_node)) = (source_node, statement_node) else {
            continue;
        };
        if source_node.has_error() || statement_node.has_error() {
            continue;
        }
        if statement_node.kind() == "call_expression" {
            let valid_require = statement_node
                .child_by_field_name("arguments")
                .filter(|arguments| arguments.named_child_count() == 1)
                .and_then(|arguments| arguments.named_child(0))
                .is_some_and(|argument| {
                    argument.kind() == "string" && argument.byte_range() == source_node.byte_range()
                });
            if !valid_require {
                continue;
            }
        }
        let raw = source_node
            .utf8_text(file.source.as_bytes())
            .map_err(|error| ScanError::Infrastructure(error.to_string()))?;
        let specifier = if file.language == LanguageKind::Python {
            raw.to_owned()
        } else {
            strip_js_string(raw).unwrap_or_default().to_owned()
        };
        if specifier.is_empty() || specifier == "__future__" {
            continue;
        }
        let relationship = if statement_node.kind() == "export_statement" {
            "re_exports"
        } else if statement_node.kind() == "call_expression" {
            "requires"
        } else {
            "imports"
        };
        ordinal += 1;
        facts.push(ImportFact {
            source_path: file.relative.clone(),
            specifier,
            relationship: relationship.into(),
            line_start: statement_node.start_position().row as u32 + 1,
            line_end: inclusive_end_line(statement_node),
            ordinal,
        });
    }
    Ok((facts, parse_warnings))
}

fn extract_rust_imports(
    file: &SourceFile,
    tree: &tree_sitter::Tree,
    language: &Language,
) -> Result<Vec<ImportFact>, ScanError> {
    let query = Query::new(language, RUST_QUERY)
        .map_err(|error| ScanError::Infrastructure(error.to_string()))?;
    let source_capture = query
        .capture_index_for_name("source")
        .expect("Rust scanner query has source capture");
    let statement_capture = query
        .capture_index_for_name("statement")
        .expect("Rust scanner query has statement capture");
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), file.source.as_bytes());
    let inline_module_bodies = rust_inline_module_body_ranges(tree.root_node());
    let mut facts = Vec::new();
    let mut ordinal = 0_usize;
    while let Some(query_match) = matches.next() {
        let source_node = query_match
            .captures
            .iter()
            .find(|capture| capture.index == source_capture)
            .map(|capture| capture.node);
        let statement_node = query_match
            .captures
            .iter()
            .find(|capture| capture.index == statement_capture)
            .map(|capture| capture.node);
        let (Some(source_node), Some(statement_node)) = (source_node, statement_node) else {
            continue;
        };
        if source_node.has_error() || statement_node.has_error() {
            continue;
        }
        if inline_module_bodies.iter().any(|body| {
            body.start <= statement_node.start_byte() && statement_node.end_byte() <= body.end
        }) {
            continue;
        }
        if statement_node.kind() == "mod_item"
            && statement_node.child_by_field_name("body").is_some()
        {
            continue;
        }
        let relationship = if statement_node.kind() == "mod_item" {
            "declares_module"
        } else {
            "imports"
        };
        let specifiers = if statement_node.kind() == "use_declaration" {
            rust_use_specifiers(source_node, file.source.as_bytes())?
        } else {
            let raw = source_node
                .utf8_text(file.source.as_bytes())
                .map_err(|error| ScanError::Infrastructure(error.to_string()))?;
            BTreeSet::from([raw.to_owned()])
        };
        for specifier in specifiers {
            if specifier.is_empty() || matches!(specifier.as_str(), "self" | "super") {
                continue;
            }
            ordinal += 1;
            facts.push(ImportFact {
                source_path: file.relative.clone(),
                specifier,
                relationship: relationship.into(),
                line_start: statement_node.start_position().row as u32 + 1,
                line_end: inclusive_end_line(statement_node),
                ordinal,
            });
        }
    }
    Ok(facts)
}

fn rust_inline_module_body_ranges(root: tree_sitter::Node<'_>) -> Vec<std::ops::Range<usize>> {
    fn collect(node: tree_sitter::Node<'_>, ranges: &mut Vec<std::ops::Range<usize>>) {
        if node.kind() == "mod_item"
            && let Some(body) = node.child_by_field_name("body")
        {
            ranges.push(body.byte_range());
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            collect(child, ranges);
        }
    }
    let mut ranges = Vec::new();
    collect(root, &mut ranges);
    ranges
}

fn rust_use_specifiers(
    node: tree_sitter::Node<'_>,
    source: &[u8],
) -> Result<BTreeSet<String>, ScanError> {
    rust_use_paths(node, source)
        .map(|paths| paths.into_iter().map(|path| path.join("::")).collect())
}

fn rust_use_paths(
    node: tree_sitter::Node<'_>,
    source: &[u8],
) -> Result<Vec<Vec<String>>, ScanError> {
    match node.kind() {
        "identifier" | "crate" | "self" | "super" => {
            let text = node
                .utf8_text(source)
                .map_err(|error| ScanError::Infrastructure(error.to_string()))?;
            Ok(vec![vec![text.to_owned()]])
        }
        "scoped_identifier" => {
            let Some(name) = node.child_by_field_name("name") else {
                return Ok(Vec::new());
            };
            let mut prefixes = match node.child_by_field_name("path") {
                Some(path) => rust_use_paths(path, source)?,
                None => vec![Vec::new()],
            };
            let names = rust_use_paths(name, source)?;
            append_rust_paths(&mut prefixes, &names);
            Ok(prefixes)
        }
        "use_as_clause" => node
            .child_by_field_name("path")
            .map(|path| rust_use_paths(path, source))
            .unwrap_or_else(|| Ok(Vec::new())),
        "use_wildcard" => {
            let mut cursor = node.walk();
            let child = node.named_children(&mut cursor).next();
            child
                .map(|path| rust_use_paths(path, source))
                .unwrap_or_else(|| Ok(Vec::new()))
        }
        "scoped_use_list" => {
            let prefixes = node
                .child_by_field_name("path")
                .map(|path| rust_use_paths(path, source))
                .transpose()?
                .unwrap_or_else(|| vec![Vec::new()]);
            let suffixes = node
                .child_by_field_name("list")
                .map(|list| rust_use_paths(list, source))
                .transpose()?
                .unwrap_or_default();
            Ok(combine_rust_paths(&prefixes, &suffixes))
        }
        "use_list" => {
            let mut paths = Vec::new();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                paths.extend(rust_use_paths(child, source)?);
            }
            Ok(paths)
        }
        _ => Ok(Vec::new()),
    }
}

fn append_rust_paths(prefixes: &mut Vec<Vec<String>>, suffixes: &[Vec<String>]) {
    *prefixes = combine_rust_paths(prefixes, suffixes);
}

fn combine_rust_paths(prefixes: &[Vec<String>], suffixes: &[Vec<String>]) -> Vec<Vec<String>> {
    prefixes
        .iter()
        .flat_map(|prefix| {
            suffixes.iter().map(move |suffix| {
                if suffix.as_slice() == ["self"] {
                    return prefix.clone();
                }
                let mut combined = prefix.clone();
                combined.extend(suffix.iter().cloned());
                combined
            })
        })
        .collect()
}

fn resolve_target(
    root: &Path,
    file: &SourceFile,
    fact: &ImportFact,
    file_paths: &BTreeSet<String>,
) -> ResolvedTarget {
    let resolved = if file.language == LanguageKind::Rust {
        resolve_rust(root, file, fact, file_paths)
    } else if file.language == LanguageKind::Python {
        resolve_python(file, fact, file_paths)
    } else if fact.specifier.starts_with("./") || fact.specifier.starts_with("../") {
        resolve_relative_js(file, fact, file_paths)
    } else if fact.specifier.starts_with("node:") {
        ResolvedTarget::extracted(TargetKey::Package(fact.specifier.clone()))
    } else {
        let package = package_root(&fact.specifier);
        if package.is_some_and(|package| declared_package(root, &file.absolute, package)) {
            ResolvedTarget::inferred(TargetKey::Package(
                package.expect("checked package").to_owned(),
            ))
        } else {
            ResolvedTarget::extracted(unresolved(fact))
        }
    };
    resolved.with_ambiguous_require(fact)
}

fn resolve_rust(
    root: &Path,
    file: &SourceFile,
    fact: &ImportFact,
    file_paths: &BTreeSet<String>,
) -> ResolvedTarget {
    if fact.relationship == "declares_module" {
        return resolve_rust_module_declaration(file, fact, file_paths);
    }
    let segments: Vec<_> = fact.specifier.split("::").collect();
    let Some(first) = segments.first().copied() else {
        return ResolvedTarget::extracted(unresolved(fact));
    };
    if first == "crate" {
        return resolve_rust_crate_path(&segments[1..], fact, file_paths);
    }
    if first == "self" || first == "super" {
        return resolve_rust_relative_path(file, &segments, fact, file_paths);
    }
    if rust_package_name(root).is_some_and(|name| rust_crate_name(&name) == first) {
        return resolve_rust_crate_path(&segments[1..], fact, file_paths);
    }
    if matches!(first, "std" | "core" | "alloc" | "proc_macro") {
        return ResolvedTarget::extracted(TargetKey::Package(first.to_owned()));
    }
    if declared_rust_package(root, &file.absolute, first) {
        return ResolvedTarget::inferred(TargetKey::Package(first.to_owned()));
    }
    ResolvedTarget::extracted(unresolved(fact))
}

fn resolve_rust_module_declaration(
    file: &SourceFile,
    fact: &ImportFact,
    file_paths: &BTreeSet<String>,
) -> ResolvedTarget {
    let path = Path::new(&file.relative);
    let parent = path.parent().unwrap_or(Path::new(""));
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("");
    let owner = if matches!(stem, "lib" | "main" | "mod")
        || path.components().count() == 2
            && matches!(
                path.components()
                    .next()
                    .and_then(|part| part.as_os_str().to_str()),
                Some("tests" | "examples")
            ) {
        parent.to_path_buf()
    } else {
        parent.join(stem)
    };
    let direct = owner.join(format!("{}.rs", fact.specifier));
    let nested = owner.join(&fact.specifier).join("mod.rs");
    let candidates = [direct, nested]
        .into_iter()
        .filter_map(|path| normalized_path(&path))
        .filter(|path| file_paths.contains(path))
        .collect::<BTreeSet<_>>();
    if candidates.len() == 1 {
        ResolvedTarget::extracted(TargetKey::File(
            candidates.into_iter().next().expect("one Rust module"),
        ))
    } else {
        ResolvedTarget::extracted(unresolved(fact))
    }
}

fn resolve_rust_relative_path(
    file: &SourceFile,
    segments: &[&str],
    fact: &ImportFact,
    file_paths: &BTreeSet<String>,
) -> ResolvedTarget {
    let Some(mut module) = rust_module_segments(&file.relative) else {
        return ResolvedTarget::extracted(unresolved(fact));
    };
    let mut index = 0;
    if segments.first() == Some(&"self") {
        index = 1;
    }
    while segments.get(index) == Some(&"super") {
        if module.pop().is_none() {
            return ResolvedTarget::extracted(unresolved(fact));
        }
        index += 1;
    }
    module.extend(
        segments[index..]
            .iter()
            .map(|segment| (*segment).to_owned()),
    );
    resolve_rust_module_segments(&module, fact, file_paths)
}

fn resolve_rust_crate_path(
    segments: &[&str],
    fact: &ImportFact,
    file_paths: &BTreeSet<String>,
) -> ResolvedTarget {
    let owned: Vec<_> = segments
        .iter()
        .map(|segment| (*segment).to_owned())
        .collect();
    resolve_rust_module_segments(&owned, fact, file_paths)
}

fn resolve_rust_module_segments(
    segments: &[String],
    fact: &ImportFact,
    file_paths: &BTreeSet<String>,
) -> ResolvedTarget {
    for length in (1..=segments.len()).rev() {
        let module = segments[..length].join("/");
        for candidate in [format!("src/{module}.rs"), format!("src/{module}/mod.rs")] {
            if file_paths.contains(&candidate) {
                return ResolvedTarget::extracted(TargetKey::File(candidate));
            }
        }
    }
    if segments.len() <= 1 {
        for candidate in ["src/lib.rs", "src/main.rs"] {
            if file_paths.contains(candidate) {
                let target = TargetKey::File(candidate.to_owned());
                return if segments.is_empty() {
                    ResolvedTarget::extracted(target)
                } else {
                    ResolvedTarget::inferred(target)
                };
            }
        }
    }
    ResolvedTarget::extracted(unresolved(fact))
}

fn rust_module_segments(path: &str) -> Option<Vec<String>> {
    let path = Path::new(path);
    if path.components().next()?.as_os_str() != "src" {
        return None;
    }
    let relative = path.strip_prefix("src").ok()?;
    let mut parts: Vec<_> = relative
        .components()
        .map(|part| part.as_os_str().to_str().map(str::to_owned))
        .collect::<Option<_>>()?;
    let file = parts.pop()?;
    let stem = Path::new(&file).file_stem()?.to_str()?;
    if !matches!(stem, "lib" | "main" | "mod") {
        parts.push(stem.to_owned());
    }
    Some(parts)
}

fn resolve_relative_js(
    file: &SourceFile,
    fact: &ImportFact,
    file_paths: &BTreeSet<String>,
) -> ResolvedTarget {
    let Some(base) = lexical_join(
        Path::new(&file.relative).parent().unwrap_or(Path::new("")),
        Path::new(&fact.specifier),
    ) else {
        return ResolvedTarget::extracted(unresolved(fact));
    };
    let Some(base) = normalized_path(&base) else {
        return ResolvedTarget::extracted(unresolved(fact));
    };
    let explicit = language_for_path(Path::new(&base)).is_some();
    let mut candidates = BTreeSet::new();
    if explicit {
        if file_paths.contains(&base) {
            candidates.insert(base);
        }
    } else {
        for extension in ["js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts"] {
            let direct = format!("{base}.{extension}");
            if file_paths.contains(&direct) {
                candidates.insert(direct);
            }
            let index = format!("{base}/index.{extension}");
            if file_paths.contains(&index) {
                candidates.insert(index);
            }
        }
    }
    if candidates.len() == 1 {
        let target = TargetKey::File(candidates.into_iter().next().expect("one candidate"));
        ResolvedTarget::extracted(target)
    } else {
        ResolvedTarget::extracted(unresolved(fact))
    }
}

fn resolve_python(
    file: &SourceFile,
    fact: &ImportFact,
    file_paths: &BTreeSet<String>,
) -> ResolvedTarget {
    let specifier = fact.specifier.as_str();
    let mut candidates = BTreeSet::new();
    if specifier.starts_with('.') {
        let dot_count = specifier
            .chars()
            .take_while(|character| *character == '.')
            .count();
        let suffix = &specifier[dot_count..];
        if suffix.is_empty() {
            return ResolvedTarget::extracted(unresolved(fact));
        }
        let mut base = Path::new(&file.relative)
            .parent()
            .unwrap_or(Path::new(""))
            .to_path_buf();
        for _ in 1..dot_count {
            if !base.pop() {
                return ResolvedTarget::extracted(unresolved(fact));
            }
        }
        let module = suffix.replace('.', "/");
        if let Some(path) = lexical_join(&base, Path::new(&module)) {
            add_python_candidates(&path, file_paths, &mut candidates);
        }
    } else {
        let module = specifier.replace('.', "/");
        add_python_candidates(Path::new(&module), file_paths, &mut candidates);
        add_python_candidates(
            Path::new("src").join(&module).as_path(),
            file_paths,
            &mut candidates,
        );
    }
    if candidates.len() == 1 {
        let target = TargetKey::File(candidates.into_iter().next().expect("one candidate"));
        if specifier.starts_with('.') {
            ResolvedTarget::extracted(target)
        } else {
            ResolvedTarget::inferred(target)
        }
    } else {
        ResolvedTarget::extracted(unresolved(fact))
    }
}

fn add_python_candidates(
    base: &Path,
    file_paths: &BTreeSet<String>,
    candidates: &mut BTreeSet<String>,
) {
    let Some(base) = normalized_path(base) else {
        return;
    };
    for candidate in [format!("{base}.py"), format!("{base}/__init__.py")] {
        if file_paths.contains(&candidate) {
            candidates.insert(candidate);
        }
    }
}

fn declared_package(root: &Path, importer: &Path, package: &str) -> bool {
    let mut directory = importer.parent();
    while let Some(current) = directory {
        if !current.starts_with(root) {
            break;
        }
        let manifest = current.join("package.json");
        if let Ok(value) = read_manifest(root, &manifest) {
            for field in [
                "dependencies",
                "devDependencies",
                "peerDependencies",
                "optionalDependencies",
            ] {
                if value
                    .get(field)
                    .and_then(Value::as_object)
                    .is_some_and(|dependencies| dependencies.contains_key(package))
                {
                    return true;
                }
            }
        }
        if current == root {
            break;
        }
        directory = current.parent();
    }
    false
}

fn manifest_entry_paths(root: &Path, file_paths: &BTreeSet<String>) -> BTreeSet<String> {
    let mut entries = BTreeSet::new();
    if let Ok(value) = read_manifest(root, &root.join("package.json")) {
        let mut raw = Vec::new();
        for field in ["main", "module", "browser"] {
            if let Some(path) = value.get(field).and_then(Value::as_str) {
                raw.push(path.to_owned());
            }
        }
        match value.get("bin") {
            Some(Value::String(path)) => raw.push(path.clone()),
            Some(Value::Object(bin_entries)) => raw.extend(
                bin_entries
                    .values()
                    .filter_map(Value::as_str)
                    .map(str::to_owned),
            ),
            _ => {}
        }
        if let Some(exports) = value.get("exports") {
            collect_root_export_paths(exports, &mut raw);
        }
        entries.extend(
            raw.into_iter()
                .filter_map(|path| lexical_join(Path::new(""), Path::new(&path)))
                .filter_map(|path| normalized_path(&path))
                .filter(|path| file_paths.contains(path)),
        );
    }
    entries.extend(
        file_paths
            .iter()
            .filter(|path| path.as_str() == "src/main.rs" || path.starts_with("src/bin/"))
            .cloned(),
    );
    entries
}

fn collect_root_export_paths(exports: &Value, paths: &mut Vec<String>) {
    let root_export = match exports {
        Value::Object(entries) if entries.keys().any(|key| key.starts_with('.')) => {
            entries.get(".")
        }
        _ => Some(exports),
    };
    if let Some(root_export) = root_export {
        collect_runtime_export_paths(root_export, paths);
    }
}

fn collect_runtime_export_paths(value: &Value, paths: &mut Vec<String>) {
    match value {
        Value::String(path) => paths.push(path.clone()),
        Value::Array(fallbacks) => {
            for fallback in fallbacks {
                collect_runtime_export_paths(fallback, paths);
            }
        }
        Value::Object(conditions) => {
            for (condition, target) in conditions {
                if condition != "types" && !condition.starts_with('.') {
                    collect_runtime_export_paths(target, paths);
                }
            }
        }
        _ => {}
    }
}

fn rust_package_name(root: &Path) -> Option<String> {
    read_toml_manifest(root, &root.join("Cargo.toml"))
        .ok()?
        .get("package")?
        .get("name")?
        .as_str()
        .map(str::to_owned)
}

fn declared_rust_package(root: &Path, importer: &Path, package: &str) -> bool {
    let mut directory = importer.parent();
    while let Some(current) = directory {
        if !current.starts_with(root) {
            break;
        }
        let manifest = current.join("Cargo.toml");
        if let Ok(value) = read_toml_manifest(root, &manifest)
            && rust_manifest_declares(&value, package)
        {
            return true;
        }
        if current == root {
            break;
        }
        directory = current.parent();
    }
    false
}

fn rust_manifest_declares(manifest: &toml::Value, package: &str) -> bool {
    ["dependencies", "dev-dependencies", "build-dependencies"]
        .into_iter()
        .filter_map(|table| manifest.get(table)?.as_table())
        .any(|dependencies| {
            dependencies
                .keys()
                .any(|name| rust_crate_name(name) == package)
        })
        || manifest
            .get("workspace")
            .and_then(|workspace| workspace.get("dependencies"))
            .and_then(toml::Value::as_table)
            .is_some_and(|dependencies| {
                dependencies
                    .keys()
                    .any(|name| rust_crate_name(name) == package)
            })
}

fn rust_crate_name(package: &str) -> String {
    package.replace('-', "_")
}

fn read_manifest(root: &Path, path: &Path) -> Result<Value, io::Error> {
    let metadata = path.symlink_metadata()?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::other("symlink manifests are not read"));
    }
    let canonical = path.canonicalize()?;
    if !canonical.starts_with(root) {
        return Err(io::Error::other("manifest escapes project root"));
    }
    let source =
        read_bounded(&canonical).map_err(|_| io::Error::other("manifest is not readable"))?;
    serde_json::from_str(&source).map_err(io::Error::other)
}

fn read_toml_manifest(root: &Path, path: &Path) -> Result<toml::Value, io::Error> {
    let metadata = path.symlink_metadata()?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::other("symlink manifests are not read"));
    }
    let canonical = path.canonicalize()?;
    if !canonical.starts_with(root) {
        return Err(io::Error::other("manifest escapes project root"));
    }
    let source =
        read_bounded(&canonical).map_err(|_| io::Error::other("manifest is not readable"))?;
    toml::from_str(&source).map_err(io::Error::other)
}

fn tree_sitter_language(kind: LanguageKind) -> Language {
    match kind {
        LanguageKind::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        LanguageKind::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        LanguageKind::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        LanguageKind::Python => tree_sitter_python::LANGUAGE.into(),
        LanguageKind::Rust => tree_sitter_rust::LANGUAGE.into(),
    }
}

fn language_for_path(path: &Path) -> Option<LanguageKind> {
    let name = path.file_name()?.to_str()?;
    if name.ends_with(".d.ts") || name.ends_with(".pyi") {
        return None;
    }
    match path.extension()?.to_str()? {
        "js" | "jsx" | "mjs" | "cjs" => Some(LanguageKind::JavaScript),
        "ts" | "mts" | "cts" => Some(LanguageKind::TypeScript),
        "tsx" => Some(LanguageKind::Tsx),
        "py" => Some(LanguageKind::Python),
        "rs" => Some(LanguageKind::Rust),
        _ => None,
    }
}

fn excluded_entry(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    [
        ".git",
        "node_modules",
        "vendor",
        "dist",
        "build",
        "out",
        "target",
        "coverage",
        ".next",
        ".nuxt",
        ".venv",
        "venv",
        "env",
        "__pycache__",
        ".tox",
        ".mypy_cache",
        ".pytest_cache",
        ".ruff_cache",
        ".ssh",
        ".aws",
        ".gnupg",
    ]
    .contains(&name)
}

fn secret_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return true;
    };
    name == ".env"
        || name.starts_with(".env.")
        || matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("pem" | "key" | "p12" | "pfx")
        )
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || matches!(name, ".npmrc" | ".pypirc")
}

fn has_hidden_component(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(part) => part
            .to_str()
            .is_some_and(|name| name.starts_with('.') && name != "." && name != ".."),
        _ => false,
    })
}

fn normalized_path(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?.to_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts.join("/"))
}

fn lexical_join(base: &Path, addition: &Path) -> Option<PathBuf> {
    let mut parts: Vec<_> = base
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_os_string()),
            _ => None,
        })
        .collect();
    for component in addition.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts.into_iter().collect())
}

fn package_root(specifier: &str) -> Option<&str> {
    if specifier.starts_with('#') || specifier.starts_with("@/") {
        return None;
    }
    if specifier.starts_with('@') {
        let second_slash = specifier.match_indices('/').nth(1).map(|(index, _)| index);
        Some(&specifier[..second_slash.unwrap_or(specifier.len())])
    } else {
        Some(specifier.split('/').next()?)
    }
}

fn strip_js_string(raw: &str) -> Option<&str> {
    let first = raw.as_bytes().first().copied()?;
    let last = raw.as_bytes().last().copied()?;
    if raw.len() >= 2 && first == last && matches!(first, b'\'' | b'"') {
        Some(&raw[1..raw.len() - 1])
    } else {
        None
    }
}

fn inclusive_end_line(node: tree_sitter::Node<'_>) -> u32 {
    let end = node.end_position();
    let zero_based = if end.column == 0 && end.row > node.start_position().row {
        end.row - 1
    } else {
        end.row
    };
    zero_based as u32 + 1
}

fn unresolved(fact: &ImportFact) -> TargetKey {
    TargetKey::Unresolved {
        source_path: fact.source_path.clone(),
        line: fact.line_start,
        ordinal: fact.ordinal,
        specifier: fact.specifier.clone(),
    }
}

fn fact_evidence(fact: &ImportFact) -> Evidence {
    Evidence {
        path: fact.source_path.clone(),
        line_start: fact.line_start,
        line_end: fact.line_end,
    }
}

fn group_for_path(path: &str) -> String {
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() == 1 {
        return "ROOT".into();
    }
    if matches!(parts[0], "apps" | "packages" | "services" | "modules") && parts.len() > 2 {
        return format!("{}/{}", parts[0], parts[1]);
    }
    if matches!(parts[0], "src" | "lib" | "app") && parts.len() > 2 {
        return parts[1].to_owned();
    }
    parts[0].to_owned()
}

fn label_for_path(path: &str) -> String {
    let path = Path::new(path);
    let without_extension = path.with_extension("");
    normalized_path(&without_extension).unwrap_or_else(|| path.display().to_string())
}

fn node_kind_order(kind: &NodeKind) -> u8 {
    match kind {
        NodeKind::Entry => 0,
        NodeKind::ExternalPackage | NodeKind::ExternalService | NodeKind::ExternalSystem => 2,
        NodeKind::Unresolved => 3,
        _ => 1,
    }
}

fn stable_id(prefix: &str, key: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{prefix}{hash:016x}")
}

fn increment(map: &mut BTreeMap<String, u32>, key: &str) {
    *map.entry(key.to_owned()).or_default() += 1;
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::scan_project;

    fn temp_project(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "wireglyph-{name}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(root: &Path, path: &str, source: &str) {
        let target = root.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, source).unwrap();
    }

    #[test]
    fn scans_js_imports_with_exact_evidence_and_unresolved_targets() {
        let root = temp_project("js");
        write(
            &root,
            "package.json",
            r#"{"dependencies":{"zod":"1"},"main":"src/main.ts"}"#,
        );
        write(
            &root,
            "src/main.ts",
            "import './setup';\nimport { z } from 'zod';\nconst missing = require('./missing');\nexport { thing } from './thing';\nimport(dynamicName);\nrequire('zod', dynamicName);\nrequire(dynamicName);\n",
        );
        write(&root, "src/setup.ts", "export const ready = true;\n");
        write(&root, "src/thing.ts", "export const thing = 1;\n");

        let graph = scan_project(&root).unwrap();
        assert_eq!(graph.schema_version, 2);
        assert_eq!(
            graph.scan_summary.health(),
            crate::model::ScanHealth::Complete
        );
        assert_eq!(graph.edges.len(), 4);
        assert_eq!(graph.scan_summary.inferred_edges, 2);
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.relationship == "re_exports" && edge.evidence.line_start == 4)
        );
        assert!(graph.nodes.iter().any(|node| matches!(
            node.kind,
            crate::model::NodeKind::ExternalPackage
        ) && node.label == "zod"));
        assert!(graph.nodes.iter().any(|node| matches!(
            node.kind,
            crate::model::NodeKind::Unresolved
        ) && node.label.contains("./missing")));
        assert!(graph.flows.is_empty());
        assert!(graph.edges.iter().any(|edge| {
            edge.relationship == "requires" && edge.provenance == crate::model::Provenance::Inferred
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.import_specifier.as_deref() == Some("zod")
                && edge.provenance == crate::model::Provenance::Inferred
        }));

        let first = serde_json::to_string(&graph).unwrap();
        let second = serde_json::to_string(&scan_project(&root).unwrap()).unwrap();
        assert_eq!(first, second);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn package_exports_select_runtime_entries_without_types_or_subpaths() {
        let root = temp_project("package-exports");
        write(
            &root,
            "package.json",
            r#"{
                "exports": {
                    ".": {
                        "types": "./index.d.ts",
                        "import": "./index.js",
                        "require": "./index.cjs"
                    },
                    "./extra": "./extra.js"
                }
            }"#,
        );
        write(&root, "index.js", "export const value = true;\n");
        write(&root, "index.cjs", "module.exports = true;\n");
        write(
            &root,
            "index.d.ts",
            "export declare const value: boolean;\n",
        );
        write(&root, "index.test-d.ts", "export {};\n");
        write(&root, "extra.js", "export const extra = true;\n");

        let graph = scan_project(&root).unwrap();
        let entry_paths = graph
            .nodes
            .iter()
            .filter(|node| matches!(node.kind, crate::model::NodeKind::Entry))
            .map(|node| node.evidence.path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(entry_paths, ["index.cjs", "index.js"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn package_exports_supports_a_root_condition_map() {
        let root = temp_project("package-root-conditions");
        write(
            &root,
            "package.json",
            r#"{"exports":{"types":"./index.d.ts","default":"./index.js"}}"#,
        );
        write(&root, "index.js", "export const value = true;\n");
        write(
            &root,
            "index.d.ts",
            "export declare const value: boolean;\n",
        );
        write(&root, "index.test-d.ts", "export {};\n");

        let graph = scan_project(&root).unwrap();
        let entry = graph
            .nodes
            .iter()
            .find(|node| matches!(node.kind, crate::model::NodeKind::Entry))
            .expect("runtime export should be selected");

        assert_eq!(entry.evidence.path, "index.js");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scans_python_imports_without_claiming_environment_packages() {
        let root = temp_project("python");
        write(
            &root,
            "app/__main__.py",
            "from .service import run\nimport requests\n",
        );
        write(&root, "app/service.py", "def run():\n    return True\n");

        let graph = scan_project(&root).unwrap();
        assert_eq!(graph.edges.len(), 2);
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.evidence.path == "app/service.py")
        );
        assert!(graph.nodes.iter().any(|node| matches!(
            node.kind,
            crate::model::NodeKind::Unresolved
        ) && node.label.contains("requests")));
        assert!(graph.edges.iter().any(|edge| {
            edge.import_specifier.as_deref() == Some(".service")
                && edge.provenance == crate::model::Provenance::Extracted
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scans_rust_modules_packages_and_grouped_uses_with_exact_evidence() {
        let root = temp_project("rust");
        write(
            &root,
            "Cargo.toml",
            r#"[package]
name = "rust-map"
version = "0.1.0"

[dependencies]
serde-json = { package = "serde_json", version = "1" }
"#,
        );
        write(
            &root,
            "src/main.rs",
            "mod worker;\nmod inline { use super::*; }\nuse crate::{model::Thing, worker::run};\nuse crate::missing::Thing as MissingThing;\nuse std::path::Path;\nuse serde_json::Value;\nfn main() {}\n",
        );
        write(
            &root,
            "src/worker.rs",
            "use crate::model::Thing;\nuse crate::RootItem;\npub fn run() -> Thing { Thing }\n",
        );
        write(&root, "src/model.rs", "pub struct Thing;\n");

        let graph = scan_project(&root).unwrap();
        assert_eq!(graph.scan_summary.files_scanned, 3);
        assert_eq!(graph.scan_summary.parse_warnings, 0);
        assert!(graph.nodes.iter().any(|node| {
            node.evidence.path == "src/main.rs"
                && matches!(node.kind, crate::model::NodeKind::Entry)
        }));
        assert!(graph.nodes.iter().any(|node| {
            node.evidence.path == "src/worker.rs"
                && matches!(node.kind, crate::model::NodeKind::Module)
        }));
        assert!(graph.nodes.iter().any(|node| {
            node.label == "std" && matches!(node.kind, crate::model::NodeKind::ExternalPackage)
        }));
        assert!(graph.nodes.iter().any(|node| {
            node.label == "serde_json"
                && matches!(node.kind, crate::model::NodeKind::ExternalPackage)
        }));
        assert!(!graph.nodes.iter().any(|node| {
            matches!(node.kind, crate::model::NodeKind::Unresolved) && node.label.contains("super")
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.relationship == "declares_module"
                && edge.evidence.path == "src/main.rs"
                && edge.evidence.line_start == 1
                && edge.import_specifier.as_deref() == Some("worker")
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.evidence.path == "src/main.rs"
                && edge.evidence.line_start == 3
                && edge.import_specifier.as_deref() == Some("crate::model::Thing")
        }));
        assert!(graph.nodes.iter().any(|node| {
            matches!(node.kind, crate::model::NodeKind::Unresolved)
                && node.label.contains("crate::missing::Thing")
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.import_specifier.as_deref() == Some("crate::RootItem")
                && edge.provenance == crate::model::Provenance::Inferred
                && graph
                    .nodes
                    .iter()
                    .any(|node| node.id == edge.target && node.evidence.path == "src/main.rs")
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.import_specifier.as_deref() == Some("serde_json::Value")
                && edge.provenance == crate::model::Provenance::Inferred
        }));

        let first = serde_json::to_string(&graph).unwrap();
        let second = serde_json::to_string(&scan_project(&root).unwrap()).unwrap();
        assert_eq!(first, second);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn extracts_each_python_module_and_reports_syntax_warnings() {
        let root = temp_project("python-multiple");
        write(
            &root,
            "main.py",
            "import foo, bar as renamed\nfrom broken import (\n",
        );
        write(&root, "foo.py", "FOO = 1\n");
        write(&root, "bar.py", "BAR = 1\n");

        let graph = scan_project(&root).unwrap();
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(graph.scan_summary.parse_warnings, 1);
        assert_eq!(graph.scan_summary.inferred_edges, 2);
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.import_specifier.as_deref() == Some("foo"))
        );
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.import_specifier.as_deref() == Some("bar"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn skips_hidden_source_and_refuses_excessive_depth() {
        let root = temp_project("limits");
        write(&root, "main.ts", "export const main = true;\n");
        write(&root, ".env.ts", "export const token = 'not-read';\n");
        let graph = scan_project(&root).unwrap();
        assert_eq!(graph.scan_summary.files_scanned, 1);
        assert_eq!(graph.scan_summary.files_skipped, 1);
        assert_eq!(graph.scan_summary.skipped_by_reason["hidden_or_secret"], 1);
        assert_eq!(
            graph.scan_summary.health(),
            crate::model::ScanHealth::Partial
        );

        let mut deep = String::new();
        for index in 0..=super::MAX_DEPTH {
            deep.push_str(&format!("d{index}/"));
        }
        write(&root, &format!("{deep}deep.ts"), "export {};\n");
        assert!(matches!(
            scan_project(&root),
            Err(super::ScanError::Limit(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn does_not_read_symlinked_manifests_outside_the_project() {
        use std::os::unix::fs::symlink;

        let root = temp_project("manifest-root");
        let outside = temp_project("manifest-outside");
        write(
            &outside,
            "package.json",
            r#"{"dependencies":{"outside-pkg":"1"}}"#,
        );
        symlink(outside.join("package.json"), root.join("package.json")).unwrap();
        write(&root, "main.ts", "import value from 'outside-pkg';\n");
        let graph = scan_project(&root).unwrap();
        assert!(graph.nodes.iter().any(|node| {
            matches!(node.kind, crate::model::NodeKind::Unresolved)
                && node.label.contains("outside-pkg")
        }));
        assert!(!graph.nodes.iter().any(|node| {
            matches!(node.kind, crate::model::NodeKind::ExternalPackage)
                && node.label == "outside-pkg"
        }));

        let nested = temp_project("manifest-nested");
        fs::create_dir_all(nested.join("packages/app")).unwrap();
        symlink(
            outside.join("package.json"),
            nested.join("packages/app/package.json"),
        )
        .unwrap();
        write(
            &nested,
            "packages/app/main.ts",
            "import value from 'outside-pkg';\n",
        );
        let graph = scan_project(&nested).unwrap();
        assert!(graph.nodes.iter().any(|node| {
            matches!(node.kind, crate::model::NodeKind::Unresolved)
                && node.label.contains("outside-pkg")
        }));

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
        fs::remove_dir_all(nested).unwrap();
    }

    #[test]
    fn scans_an_explicit_root_even_when_its_name_is_normally_excluded() {
        let parent = temp_project("excluded-root-parent");
        let root = parent.join("build");
        fs::create_dir_all(&root).unwrap();
        write(&root, "main.ts", "export const main = true;\n");

        let graph = scan_project(&root).unwrap();
        assert_eq!(graph.scan_summary.files_scanned, 1);
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.evidence.path == "main.ts")
        );
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn refuses_explicit_sensitive_roots_but_allows_generated_roots() {
        let parent = temp_project("sensitive-root-parent");
        for name in [".ssh", ".aws", ".gnupg"] {
            let root = parent.join(name);
            fs::create_dir_all(&root).unwrap();
            write(&root, "main.ts", "export const secret = true;\n");
            assert!(matches!(
                scan_project(&root),
                Err(super::ScanError::InvalidRoot(_))
            ));
        }
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn require_relationships_are_inferred_because_the_binding_is_ambiguous() {
        let root = temp_project("shadowed-require");
        write(
            &root,
            "main.js",
            "require('./module.js');\nfunction load(require) { require('./shadowed.js'); }\n",
        );
        write(&root, "module.js", "module.exports = true;\n");
        write(&root, "shadowed.js", "module.exports = false;\n");

        let graph = scan_project(&root).unwrap();
        assert_eq!(graph.edges.len(), 2);
        assert!(
            graph
                .edges
                .iter()
                .all(|edge| edge.provenance == crate::model::Provenance::Inferred)
        );
        assert_eq!(graph.scan_summary.inferred_edges, 2);
        assert!(graph.flows.is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
