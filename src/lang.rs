//! Language profiles, gated by Cargo features.
//!
//! Each compiled-in language contributes a [`Profile`]: the tree-sitter grammar,
//! the node kinds that are declarations (to emit in a skim), the ones whose
//! bodies we descend into, the ones that form an elidable body, and the comment
//! kinds. Grammars are optional dependencies; a language only exists in a build
//! when its `lang-*` feature is on.
//!
//! Add a language by adding the optional dependency, the feature, a `fn`
//! returning its grammar, a `static Profile`, and one `PROFILES` entry.

use std::path::Path;

use tree_sitter::Language;

pub struct Profile {
    pub name: &'static str,
    /// File extensions (without the dot) this profile claims.
    pub extensions: &'static [&'static str],
    pub language: fn() -> Language,
    /// Node kinds emitted as a declaration line.
    pub def_kinds: &'static [&'static str],
    /// Declaration kinds whose body we descend into (types/namespaces).
    pub type_kinds: &'static [&'static str],
    /// Node kinds that form an elidable body / a container to descend into.
    pub body_kinds: &'static [&'static str],
    pub comment_kinds: &'static [&'static str],
    /// `(open, close)` around a type's indented members.
    pub braces: (&'static str, &'static str),
    /// Suffix appended to an elided executable body.
    pub elide: &'static str,
    /// Search descendants for a body when the grammar exposes none as a field
    /// (Kotlin functions, Go type declarations).
    pub scan_body: bool,
    /// Markdown-style document (rendered as a heading/quote skeleton).
    pub doc: bool,
}

macro_rules! grammar_fn {
    ($fname:ident, $path:path) => {
        fn $fname() -> Language {
            $path.into()
        }
    };
}

// -- Markdown is always compiled in (the document renderer). -----------------

grammar_fn!(markdown, tree_sitter_md::LANGUAGE);

static MARKDOWN: Profile = Profile {
    name: "markdown",
    extensions: &["md", "markdown"],
    language: markdown,
    def_kinds: &["atx_heading", "setext_heading"],
    type_kinds: &[],
    body_kinds: &[],
    comment_kinds: &[],
    braces: ("", ""),
    elide: "",
    scan_body: false,
    doc: true,
};

macro_rules! language {
    (
        $static:ident, $fname:ident, $feature:literal, $krate:ident, $method:ident,
        $name:literal, $exts:expr, $defs:expr, $types:expr, $bodies:expr, $comments:expr,
        $braces:expr, $elide:literal, $scan:literal
    ) => {
        #[cfg(feature = $feature)]
        grammar_fn!($fname, $krate::$method);

        #[cfg(feature = $feature)]
        static $static: Profile = Profile {
            name: $name,
            extensions: $exts,
            language: $fname,
            def_kinds: $defs,
            type_kinds: $types,
            body_kinds: $bodies,
            comment_kinds: $comments,
            braces: $braces,
            elide: $elide,
            scan_body: $scan,
            doc: false,
        };
    };
}

language!(
    JAVA,
    java_fn,
    "lang-java",
    tree_sitter_java,
    LANGUAGE,
    "java",
    &["java"],
    &[
        "class_declaration",
        "interface_declaration",
        "enum_declaration",
        "enum_constant",
        "record_declaration",
        "annotation_type_declaration",
        "annotation_type_element_declaration",
        "method_declaration",
        "constructor_declaration",
        "compact_constructor_declaration",
        "field_declaration"
    ],
    &[
        "class_declaration",
        "interface_declaration",
        "enum_declaration",
        "record_declaration",
        "annotation_type_declaration"
    ],
    &[
        "class_body",
        "interface_body",
        "enum_body",
        "annotation_type_body",
        "constructor_body",
        "block"
    ],
    &["line_comment", "block_comment"],
    (" {", "}"),
    " { … }",
    false
);

language!(
    PYTHON,
    python_fn,
    "lang-python",
    tree_sitter_python,
    LANGUAGE,
    "python",
    &["py", "pyi"],
    &["function_definition", "class_definition"],
    &["class_definition"],
    &["block"],
    &["comment"],
    ("", ""),
    " …",
    false
);

language!(
    JAVASCRIPT,
    javascript_fn,
    "lang-javascript",
    tree_sitter_javascript,
    LANGUAGE,
    "javascript",
    &["js", "mjs", "cjs", "jsx"],
    &[
        "function_declaration",
        "generator_function_declaration",
        "class_declaration",
        "method_definition",
        "lexical_declaration",
        "variable_declaration",
        "field_definition"
    ],
    &["class_declaration"],
    &["class_body", "statement_block"],
    &["comment", "html_comment"],
    (" {", "}"),
    " { … }",
    false
);

language!(
    TYPESCRIPT,
    typescript_fn,
    "lang-typescript",
    tree_sitter_typescript,
    LANGUAGE_TYPESCRIPT,
    "typescript",
    &["ts", "mts", "cts"],
    &[
        "function_declaration",
        "generator_function_declaration",
        "class_declaration",
        "abstract_class_declaration",
        "interface_declaration",
        "enum_declaration",
        "method_definition",
        "internal_module",
        "module",
        "lexical_declaration",
        "variable_declaration",
        "type_alias_declaration",
        "public_field_definition",
        "property_signature",
        "method_signature",
        "call_signature",
        "construct_signature",
        "index_signature",
        "enum_assignment",
        "property_identifier"
    ],
    &[
        "class_declaration",
        "abstract_class_declaration",
        "interface_declaration",
        "enum_declaration",
        "internal_module",
        "module"
    ],
    &["class_body", "object_type", "enum_body", "statement_block"],
    &["comment", "html_comment"],
    (" {", "}"),
    " { … }",
    false
);

language!(
    TSX,
    tsx_fn,
    "lang-typescript",
    tree_sitter_typescript,
    LANGUAGE_TSX,
    "tsx",
    &["tsx"],
    &[
        "function_declaration",
        "generator_function_declaration",
        "class_declaration",
        "abstract_class_declaration",
        "interface_declaration",
        "enum_declaration",
        "method_definition",
        "internal_module",
        "module",
        "lexical_declaration",
        "variable_declaration",
        "type_alias_declaration",
        "public_field_definition",
        "property_signature",
        "method_signature",
        "call_signature",
        "construct_signature",
        "index_signature",
        "enum_assignment",
        "property_identifier"
    ],
    &[
        "class_declaration",
        "abstract_class_declaration",
        "interface_declaration",
        "enum_declaration",
        "internal_module",
        "module"
    ],
    &["class_body", "object_type", "enum_body", "statement_block"],
    &["comment", "html_comment"],
    (" {", "}"),
    " { … }",
    false
);

language!(
    RUST,
    rust_fn,
    "lang-rust",
    tree_sitter_rust,
    LANGUAGE,
    "rust",
    &["rs"],
    &[
        "function_item",
        "struct_item",
        "enum_item",
        "enum_variant",
        "trait_item",
        "impl_item",
        "mod_item",
        "const_item",
        "static_item",
        "type_item",
        "union_item",
        "macro_definition",
        "field_declaration",
        "function_signature_item"
    ],
    &[
        "struct_item",
        "enum_item",
        "trait_item",
        "impl_item",
        "mod_item",
        "union_item"
    ],
    &[
        "declaration_list",
        "field_declaration_list",
        "enum_variant_list",
        "block"
    ],
    &["line_comment", "block_comment"],
    (" {", "}"),
    " { … }",
    false
);

language!(
    GO,
    go_fn,
    "lang-go",
    tree_sitter_go,
    LANGUAGE,
    "go",
    &["go"],
    &[
        "function_declaration",
        "method_declaration",
        "type_declaration",
        "const_declaration",
        "var_declaration",
        "field_declaration",
        "method_elem"
    ],
    &["type_declaration"],
    &["field_declaration_list", "interface_type", "struct_type"],
    &["comment"],
    (" {", "}"),
    " { … }",
    false
);

language!(
    C,
    c_fn,
    "lang-c",
    tree_sitter_c,
    LANGUAGE,
    "c",
    &["c", "h"],
    &[
        "function_definition",
        "struct_specifier",
        "union_specifier",
        "enum_specifier",
        "declaration",
        "type_definition",
        "preproc_def",
        "preproc_function_def",
        "field_declaration",
        "enumerator"
    ],
    &["struct_specifier", "union_specifier", "enum_specifier"],
    &[
        "field_declaration_list",
        "enumerator_list",
        "compound_statement"
    ],
    &["comment"],
    (" {", "}"),
    " { … }",
    false
);

language!(
    CPP,
    cpp_fn,
    "lang-cpp",
    tree_sitter_cpp,
    LANGUAGE,
    "cpp",
    &["cc", "cpp", "cxx", "c++", "hpp", "hh", "hxx"],
    &[
        "function_definition",
        "class_specifier",
        "struct_specifier",
        "union_specifier",
        "enum_specifier",
        "namespace_definition",
        "template_declaration",
        "declaration",
        "field_declaration",
        "type_definition",
        "preproc_def",
        "preproc_function_def",
        "enumerator"
    ],
    &[
        "class_specifier",
        "struct_specifier",
        "union_specifier",
        "enum_specifier",
        "namespace_definition"
    ],
    &[
        "field_declaration_list",
        "declaration_list",
        "enumerator_list",
        "compound_statement"
    ],
    &["comment"],
    (" {", "}"),
    " { … }",
    false
);

language!(
    CSHARP,
    csharp_fn,
    "lang-csharp",
    tree_sitter_c_sharp,
    LANGUAGE,
    "csharp",
    &["cs", "c#"],
    &[
        "class_declaration",
        "interface_declaration",
        "struct_declaration",
        "enum_declaration",
        "record_declaration",
        "namespace_declaration",
        "method_declaration",
        "constructor_declaration",
        "destructor_declaration",
        "accessor_declaration",
        "property_declaration",
        "field_declaration",
        "delegate_declaration",
        "event_declaration",
        "local_function_statement",
        "enum_member_declaration"
    ],
    &[
        "class_declaration",
        "interface_declaration",
        "struct_declaration",
        "enum_declaration",
        "record_declaration",
        "namespace_declaration"
    ],
    &[
        "declaration_list",
        "enum_member_declaration_list",
        "accessor_list",
        "block"
    ],
    &["comment"],
    (" {", "}"),
    " { … }",
    false
);

language!(
    RUBY,
    ruby_fn,
    "lang-ruby",
    tree_sitter_ruby,
    LANGUAGE,
    "ruby",
    &["rb", "rake", "gemspec"],
    &["class", "module", "method", "singleton_method"],
    &["class", "module"],
    &["body_statement"],
    &["comment"],
    ("", ""),
    " …",
    false
);

language!(
    PHP,
    php_fn,
    "lang-php",
    tree_sitter_php,
    LANGUAGE_PHP,
    "php",
    &["php"],
    &[
        "class_declaration",
        "interface_declaration",
        "trait_declaration",
        "enum_declaration",
        "function_definition",
        "method_declaration",
        "property_declaration",
        "const_declaration",
        "namespace_definition"
    ],
    &[
        "class_declaration",
        "interface_declaration",
        "trait_declaration",
        "enum_declaration",
        "namespace_definition"
    ],
    &[
        "declaration_list",
        "enum_declaration_list",
        "compound_statement"
    ],
    &["comment"],
    (" {", "}"),
    " { … }",
    false
);

language!(
    KOTLIN,
    kotlin_fn,
    "lang-kotlin",
    tree_sitter_kotlin_ng,
    LANGUAGE,
    "kotlin",
    &["kt", "kts"],
    &[
        "class_declaration",
        "object_declaration",
        "function_declaration",
        "property_declaration",
        "type_alias",
        "enum_entry",
        "secondary_constructor"
    ],
    &["class_declaration", "object_declaration"],
    &["class_body", "function_body", "enum_class_body"],
    &["line_comment", "block_comment"],
    (" {", "}"),
    " { … }",
    true
);

language!(
    SWIFT,
    swift_fn,
    "lang-swift",
    tree_sitter_swift,
    LANGUAGE,
    "swift",
    &["swift"],
    &[
        "class_declaration",
        "function_declaration",
        "protocol_declaration",
        "init_declaration",
        "deinit_declaration",
        "property_declaration",
        "typealias_declaration",
        "enum_entry",
        "protocol_function_declaration",
        "protocol_property_declaration",
        "subscript_declaration",
        "associatedtype_declaration"
    ],
    &["class_declaration", "protocol_declaration"],
    &[
        "class_body",
        "function_body",
        "protocol_body",
        "computed_property"
    ],
    &["comment", "multiline_comment"],
    (" {", "}"),
    " { … }",
    false
);

language!(
    SCALA,
    scala_fn,
    "lang-scala",
    tree_sitter_scala,
    LANGUAGE,
    "scala",
    &["scala", "sc", "sbt"],
    &[
        "class_definition",
        "enum_definition",
        "function_definition",
        "function_declaration",
        "given_definition",
        "object_definition",
        "trait_definition",
        "val_definition",
        "var_definition",
        "val_declaration",
        "var_declaration",
        "type_definition",
        "extension_definition",
        "package_object",
        "package_clause",
        "enum_case_definitions"
    ],
    &[
        "class_definition",
        "enum_definition",
        "object_definition",
        "trait_definition",
        "given_definition",
        "package_object",
        "package_clause"
    ],
    &["template_body", "enum_body", "indented_block", "block"],
    &["block_comment", "comment", "xml_comment"],
    (" {", "}"),
    " { … }",
    false
);

language!(
    LUA,
    lua_fn,
    "lang-lua",
    tree_sitter_lua,
    LANGUAGE,
    "lua",
    &["lua"],
    &["function_declaration", "variable_declaration"],
    &[],
    &["block"],
    &["comment"],
    ("", ""),
    " …",
    false
);

language!(
    BASH,
    bash_fn,
    "lang-bash",
    tree_sitter_bash,
    LANGUAGE,
    "bash",
    &["sh", "bash", "zsh", "shell"],
    &["function_definition"],
    &[],
    &["compound_statement"],
    &["comment"],
    ("", ""),
    " …",
    false
);

language!(
    HTML,
    html_fn,
    "lang-html",
    tree_sitter_html,
    LANGUAGE,
    "html",
    &["html", "htm"],
    &[],
    &[],
    &[],
    &["comment"],
    ("", ""),
    "",
    false
);

language!(
    CSS,
    css_fn,
    "lang-css",
    tree_sitter_css,
    LANGUAGE,
    "css",
    &["css"],
    &["rule_set", "media_statement", "keyframes_statement"],
    &[],
    &["block"],
    &["comment", "js_comment"],
    (" {", "}"),
    " { … }",
    true
);

language!(
    JSON,
    json_fn,
    "lang-json",
    tree_sitter_json,
    LANGUAGE,
    "json",
    &["json", "jsonc"],
    &[],
    &[],
    &[],
    &["comment"],
    ("", ""),
    "",
    false
);

language!(
    YAML,
    yaml_fn,
    "lang-yaml",
    tree_sitter_yaml,
    LANGUAGE,
    "yaml",
    &["yaml", "yml"],
    &[],
    &[],
    &[],
    &["comment"],
    ("", ""),
    "",
    false
);

language!(
    TOML,
    toml_fn,
    "lang-toml",
    tree_sitter_toml_ng,
    LANGUAGE,
    "toml",
    &["toml"],
    &[],
    &[],
    &[],
    &["comment"],
    ("", ""),
    "",
    false
);

/// Every profile compiled into this build, markdown first.
pub static PROFILES: &[&Profile] = &[
    &MARKDOWN,
    #[cfg(feature = "lang-java")]
    &JAVA,
    #[cfg(feature = "lang-python")]
    &PYTHON,
    #[cfg(feature = "lang-javascript")]
    &JAVASCRIPT,
    #[cfg(feature = "lang-typescript")]
    &TYPESCRIPT,
    #[cfg(feature = "lang-typescript")]
    &TSX,
    #[cfg(feature = "lang-rust")]
    &RUST,
    #[cfg(feature = "lang-go")]
    &GO,
    #[cfg(feature = "lang-c")]
    &C,
    #[cfg(feature = "lang-cpp")]
    &CPP,
    #[cfg(feature = "lang-csharp")]
    &CSHARP,
    #[cfg(feature = "lang-ruby")]
    &RUBY,
    #[cfg(feature = "lang-php")]
    &PHP,
    #[cfg(feature = "lang-kotlin")]
    &KOTLIN,
    #[cfg(feature = "lang-swift")]
    &SWIFT,
    #[cfg(feature = "lang-scala")]
    &SCALA,
    #[cfg(feature = "lang-lua")]
    &LUA,
    #[cfg(feature = "lang-bash")]
    &BASH,
    #[cfg(feature = "lang-html")]
    &HTML,
    #[cfg(feature = "lang-css")]
    &CSS,
    #[cfg(feature = "lang-json")]
    &JSON,
    #[cfg(feature = "lang-yaml")]
    &YAML,
    #[cfg(feature = "lang-toml")]
    &TOML,
];

fn profile_for_extension(ext: &str) -> Option<&'static Profile> {
    let ext = ext.to_ascii_lowercase();
    PROFILES
        .iter()
        .copied()
        .find(|p| p.extensions.contains(&ext.as_str()))
}

#[derive(Clone, Copy)]
pub struct Lang {
    profile: Option<&'static Profile>,
}

impl PartialEq for Lang {
    fn eq(&self, other: &Self) -> bool {
        match (self.profile, other.profile) {
            (Some(a), Some(b)) => std::ptr::eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl Eq for Lang {}

impl std::fmt::Debug for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl Lang {
    pub const fn plain() -> Lang {
        Lang { profile: None }
    }

    pub fn markdown() -> Lang {
        Lang {
            profile: Some(&MARKDOWN),
        }
    }

    pub fn from_path(path: &Path) -> Lang {
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) => match profile_for_extension(ext) {
                Some(p) => Lang { profile: Some(p) },
                None => Lang::plain(),
            },
            None => Lang::plain(),
        }
    }

    pub fn from_name(name: &str) -> Option<Lang> {
        if matches!(name.to_ascii_lowercase().as_str(), "plain" | "text" | "txt") {
            return Some(Lang::plain());
        }
        if let Some(p) = PROFILES.iter().copied().find(|p| p.name == name) {
            return Some(Lang { profile: Some(p) });
        }
        profile_for_extension(name).map(|p| Lang { profile: Some(p) })
    }

    pub fn profile(self) -> Option<&'static Profile> {
        self.profile
    }

    pub fn name(self) -> &'static str {
        self.profile.map(|p| p.name).unwrap_or("plain")
    }

    pub fn language(self) -> Option<Language> {
        self.profile.map(|p| (p.language)())
    }

    pub fn is_doc(self) -> bool {
        self.profile.map(|p| p.doc).unwrap_or(false)
    }

    pub fn is_parseable(self) -> bool {
        self.profile.is_some()
    }

    pub fn def_kinds(self) -> &'static [&'static str] {
        self.profile.map(|p| p.def_kinds).unwrap_or(&[])
    }

    pub fn type_kinds(self) -> &'static [&'static str] {
        self.profile.map(|p| p.type_kinds).unwrap_or(&[])
    }

    pub fn body_kinds(self) -> &'static [&'static str] {
        self.profile.map(|p| p.body_kinds).unwrap_or(&[])
    }

    pub fn comment_kinds(self) -> &'static [&'static str] {
        self.profile.map(|p| p.comment_kinds).unwrap_or(&[])
    }

    pub fn braces(self) -> (&'static str, &'static str) {
        self.profile.map(|p| p.braces).unwrap_or(("", ""))
    }

    pub fn elide(self) -> &'static str {
        self.profile.map(|p| p.elide).unwrap_or("")
    }

    pub fn scan_body(self) -> bool {
        self.profile.map(|p| p.scan_body).unwrap_or(false)
    }

    /// All source (non-document) extensions compiled in.
    pub fn source_extensions() -> Vec<&'static str> {
        let mut out = Vec::new();
        for p in PROFILES {
            if p.doc {
                continue;
            }
            for e in p.extensions {
                if !out.contains(e) {
                    out.push(*e);
                }
            }
        }
        out
    }

    pub fn is_source_extension(ext: &str) -> bool {
        profile_for_extension(ext).map(|p| !p.doc).unwrap_or(false)
    }
}
