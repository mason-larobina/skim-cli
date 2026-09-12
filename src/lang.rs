//! Language profiles: which tree-sitter grammar parses a file, which nodes are
//! declarations (to emit in a skim), which declarations own a brace/body we may
//! descend into, and which nodes are comments.

use std::path::Path;

use tree_sitter::Language;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Java,
    Markdown,
    Plain,
}

impl Lang {
    pub fn from_path(path: &Path) -> Lang {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref()
        {
            Some("java") => Lang::Java,
            Some("md") | Some("markdown") => Lang::Markdown,
            _ => Lang::Plain,
        }
    }

    pub fn from_name(name: &str) -> Option<Lang> {
        match name.to_ascii_lowercase().as_str() {
            "java" => Some(Lang::Java),
            "md" | "markdown" => Some(Lang::Markdown),
            "plain" | "text" | "txt" => Some(Lang::Plain),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::Java => "java",
            Lang::Markdown => "markdown",
            Lang::Plain => "plain",
        }
    }

    pub fn language(self) -> Option<Language> {
        match self {
            Lang::Java => Some(tree_sitter_java::LANGUAGE.into()),
            Lang::Markdown => Some(tree_sitter_md::LANGUAGE.into()),
            Lang::Plain => None,
        }
    }

    pub fn is_doc(self) -> bool {
        self == Lang::Markdown
    }

    pub fn is_parseable(self) -> bool {
        self.language().is_some()
    }

    /// Source extensions this language claims. Used by pair discovery.
    pub fn source_extensions() -> &'static [&'static str] {
        &["java"]
    }

    pub fn doc_extensions() -> &'static [&'static str] {
        &["md", "markdown"]
    }

    /// Node kinds emitted as a declaration line in a skim.
    pub fn def_kinds(self) -> &'static [&'static str] {
        match self {
            Lang::Java => &[
                "class_declaration",
                "interface_declaration",
                "enum_declaration",
                "enum_constant",
                "record_declaration",
                "annotation_type_declaration",
                "annotation_type_element_declaration",
                "method_declaration",
                "constructor_declaration",
                "field_declaration",
            ],
            Lang::Markdown => &["atx_heading", "setext_heading"],
            Lang::Plain => &[],
        }
    }

    /// Declaration kinds whose body we descend into (types/namespaces). An
    /// executable body is elided, never walked.
    pub fn type_kinds(self) -> &'static [&'static str] {
        match self {
            Lang::Java => &[
                "class_declaration",
                "interface_declaration",
                "enum_declaration",
                "record_declaration",
                "annotation_type_declaration",
            ],
            Lang::Markdown => &[],
            Lang::Plain => &[],
        }
    }

    pub fn comment_kinds(self) -> &'static [&'static str] {
        match self {
            Lang::Java => &["line_comment", "block_comment"],
            Lang::Markdown => &[],
            Lang::Plain => &[],
        }
    }

    /// `(open, close)` delimiters when emitting a type declaration with the
    /// members indented between them.
    pub fn braces(self) -> (&'static str, &'static str) {
        match self {
            Lang::Java => (" {", "}"),
            Lang::Markdown => ("", ""),
            Lang::Plain => ("", ""),
        }
    }
}
