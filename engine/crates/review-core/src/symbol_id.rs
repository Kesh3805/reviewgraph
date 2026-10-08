#![doc = include_str!("../../../../docs/graph-schema/symbol-id.md")]
//! Implementation of the grammar documented in `docs/graph-schema/symbol-id.md`: parts, escaping,
//! strict parsing and the module-path derivation.
//!
//! Every function here is pure and allocation-only, so ids are identical on the CLI host, in the
//! worker and in tests.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use crate::ids::SymbolId;
use crate::language::Language;
use crate::location::RepoPath;
use crate::symbol::{Hash128, ModulePath, SymbolKind};

/// Longest module path in bytes. Longer module paths are rejected, never truncated.
pub const MAX_MODULE_PATH_BYTES: usize = 1024;
/// Longest qualified-name segment in bytes. Longer segments are rejected, never truncated.
pub const MAX_SEGMENT_BYTES: usize = 256;
/// Longest whole canonical id in bytes.
pub const MAX_ID_BYTES: usize = 2048;
/// Longest language tag in bytes (`[a-z][a-z0-9_]{0,15}`).
pub const MAX_LANG_BYTES: usize = 16;
/// Qualified name of the module symbol (SID-002).
pub const MODULE_SEGMENT: &str = "__module__";
/// Domain separator of the lossy degradation marker.
const LOSSY_DOMAIN: &str = "rg.lossy.v1";
/// Bytes a degraded segment spends on `…~deadbeef` (`…` is three UTF-8 bytes).
const LOSSY_MARKER_BYTES: usize = 12;

/// Node-id prefixes reserved by CG-001. They may never appear as a `SymbolId` language, otherwise
/// a symbol and a graph node would be indistinguishable in a key or a URL.
pub const RESERVED_LANG_PREFIXES: &[&str] = &[
    "repo", "dir", "file", "package", "http", "queue", "db", "env", "pkg", "test",
];

/// Why a symbol id or one of its parts was rejected (ADR-005, SID-001).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SymbolIdError {
    /// The input was empty.
    #[error("symbol id is empty")]
    Empty,
    /// The language tag is not `[a-z][a-z0-9_]{0,15}`.
    #[error("invalid language tag {0:?}")]
    BadLang(String),
    /// The language tag is reserved for graph node ids (CG-001).
    #[error("language tag {0:?} is reserved for graph node ids")]
    ReservedLang(String),
    /// The module path is missing, empty, absolute, escapes the repository or is malformed.
    #[error("invalid module path: {0}")]
    BadModulePath(String),
    /// A qualified-name segment is empty, malformed or undecodable.
    #[error("invalid qualified-name segment: {0}")]
    BadSegment(String),
    /// The kind is not a `SymbolKind::as_id_str()` value.
    #[error("invalid symbol kind {0:?}")]
    BadKind(String),
    /// The ordinal is not a decimal `>= 1` without leading zeros.
    #[error("invalid ordinal {0:?}: expected a decimal >= 1 without leading zeros")]
    BadOrdinal(String),
    /// The input parses but is not the one canonical spelling of its parts.
    #[error("symbol id is not canonical: {0}")]
    NonCanonical(String),
    /// A part exceeds its documented byte limit. Parts are never truncated implicitly.
    #[error("{field} is {actual} bytes, over the {max} byte limit")]
    TooLong {
        /// Which limit was exceeded.
        field: &'static str,
        /// Size of the offending part.
        actual: usize,
        /// The documented limit.
        max: usize,
    },
}

/// The parts of a canonical [`SymbolId`]. Formatting is a pure function of these values.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolIdParts {
    /// Language tag: `ts`, `js`, ... (`Language::id_prefix`).
    pub lang: String,
    /// Repository-relative module path without its final extension.
    pub module_path: ModulePath,
    /// Raw (unescaped) qualified-name segments, outermost first.
    pub qualified_name: Vec<String>,
    /// What kind of declaration the symbol is.
    pub kind: SymbolKind,
    /// Overload/duplicate ordinal (SID-003). `0` means "no ordinal", which formats without `~n`.
    pub ordinal: u16,
}

impl SymbolIdParts {
    /// Parts for one symbol, without an ordinal.
    pub fn new(
        lang: impl Into<String>,
        module_path: ModulePath,
        qualified_name: Vec<String>,
        kind: SymbolKind,
    ) -> Self {
        Self {
            lang: lang.into(),
            module_path,
            qualified_name,
            kind,
            ordinal: 0,
        }
    }

    /// Sets the overload/duplicate ordinal (SID-003). `0` means "no ordinal".
    pub fn with_ordinal(mut self, ordinal: u16) -> Self {
        self.ordinal = ordinal;
        self
    }

    /// The module symbol of one unit: `ts:src/auth/auth.service#__module__/module`.
    pub fn module(lang: impl Into<String>, module_path: ModulePath) -> Self {
        Self::new(
            lang,
            module_path,
            vec![MODULE_SEGMENT.to_owned()],
            SymbolKind::Module,
        )
    }

    /// The canonical id string. Fails instead of truncating, because truncation would silently
    /// create collisions.
    pub fn format(&self) -> Result<SymbolId, SymbolIdError> {
        let lang = self.checked_lang()?;
        if self.qualified_name.is_empty() {
            return Err(SymbolIdError::BadSegment(
                "qualified name has no segment".to_owned(),
            ));
        }
        let mut out: Vec<u8> = Vec::with_capacity(64);
        out.extend_from_slice(lang.as_bytes());
        out.push(b':');
        let module = nfc(self.module_path.as_str());
        check_module_path(&module)?;
        escape_into(&module, Part::Module, &mut out);
        out.push(b'#');
        for (index, segment) in self.qualified_name.iter().enumerate() {
            if index > 0 {
                out.push(b'.');
            }
            let segment = nfc(segment);
            check_segment(&segment)?;
            escape_into(&segment, Part::Segment, &mut out);
        }
        out.push(b'/');
        out.extend_from_slice(self.kind.as_id_str().as_bytes());
        if self.ordinal > 0 {
            out.push(b'~');
            out.extend_from_slice(self.ordinal.to_string().as_bytes());
        }
        if out.len() > MAX_ID_BYTES {
            return Err(SymbolIdError::TooLong {
                field: "symbol id",
                actual: out.len(),
                max: MAX_ID_BYTES,
            });
        }
        Ok(SymbolId::from_canonical_unchecked(
            String::from_utf8(out).map_err(|e| SymbolIdError::BadModulePath(e.to_string()))?,
        ))
    }

    /// The canonical id, degrading over-long parts instead of failing.
    ///
    /// An over-long segment becomes `text…~deadbeef`, where the hash is `blake3` over the full
    /// segment under the `rg.lossy.v1` domain. Degradation is deterministic but lossy, so it is
    /// never used where [`SymbolIdParts::format`] can succeed; callers that need to observe it
    /// count it (`symbol_id_lossy_total`) and can look for the `~`+hex marker.
    pub fn format_lossy(&self) -> SymbolId {
        let lang = self.checked_lang_lossy();
        let mut segments: Vec<String> = self
            .qualified_name
            .iter()
            .map(|s| degrade(s, MAX_SEGMENT_BYTES))
            .collect();
        if segments.is_empty() {
            segments.push(MODULE_SEGMENT.to_owned());
        }
        let mut module = degrade(&nfc(self.module_path.as_str()), MAX_MODULE_PATH_BYTES);
        // Each round either drops one name segment or shrinks the module path to a fixed-size
        // hash, so the loop converges and every candidate stays under the limits.
        for _ in 0..8 {
            let candidate = SymbolIdParts {
                lang: lang.clone(),
                module_path: ModulePath::from_checked(module.clone()),
                qualified_name: segments.clone(),
                kind: self.kind,
                ordinal: self.ordinal,
            };
            if let Ok(id) = candidate.format() {
                return id;
            }
            if segments.len() > 1 {
                let dropped = segments.pop().unwrap_or_default();
                segments.push(format!("…~{}", short_hash(dropped.as_bytes())));
            } else {
                module = format!("h{}", short_hash(module.as_bytes()));
            }
        }
        // Last resort: one short segment and a hashed module path, which always fits.
        let final_parts = SymbolIdParts {
            lang,
            module_path: ModulePath::from_checked(format!("h{}", short_hash(module.as_bytes()))),
            qualified_name: vec![degrade(&segments.first().cloned().unwrap_or_default(), 32)],
            kind: self.kind,
            ordinal: self.ordinal,
        };
        final_parts.format().unwrap_or_else(|_| {
            SymbolId::from_canonical_unchecked(format!(
                "x:h{}#x/{}",
                short_hash(module.as_bytes()),
                self.kind.as_id_str()
            ))
        })
    }

    /// Parses a canonical id string into its parts. Structural errors are reported; canonicality
    /// is checked by [`SymbolId::parse`], which re-formats the parts.
    pub fn parse(raw: &str) -> Result<Self, SymbolIdError> {
        if raw.is_empty() {
            return Err(SymbolIdError::Empty);
        }
        if raw.len() > MAX_ID_BYTES {
            return Err(SymbolIdError::TooLong {
                field: "symbol id",
                actual: raw.len(),
                max: MAX_ID_BYTES,
            });
        }
        let (head, rest) = raw
            .split_once('#')
            .ok_or_else(|| SymbolIdError::BadModulePath("missing `#` separator".to_owned()))?;
        let (lang, module) = head
            .split_once(':')
            .ok_or_else(|| SymbolIdError::BadLang(format!("missing `:` in {head:?}")))?;
        check_lang(lang)?;
        let module = unescape(module, Part::Module).map_err(module_error)?;
        check_module_path(&module)?;

        // The kind separator is the last `/`: a `/` inside a name is always escaped as `%2F`.
        let (qualified, tail) = rest.rsplit_once('/').ok_or_else(|| {
            SymbolIdError::BadSegment(format!("missing `/` before the kind in {rest:?}"))
        })?;
        let (kind_text, ordinal_text) = match tail.rsplit_once('~') {
            Some((kind, ordinal)) => (kind, Some(ordinal)),
            None => (tail, None),
        };
        let kind = SymbolKind::from_id_str(kind_text)
            .ok_or_else(|| SymbolIdError::BadKind(kind_text.to_owned()))?;
        let ordinal = match ordinal_text {
            None => 0,
            Some(text) => parse_ordinal(text)?,
        };

        let mut segments = Vec::new();
        for segment in qualified.split('.') {
            let decoded = unescape(segment, Part::Segment).map_err(segment_error)?;
            check_segment(&decoded)?;
            segments.push(decoded);
        }
        Ok(Self {
            lang: lang.to_owned(),
            module_path: ModulePath::from_checked(module),
            qualified_name: segments,
            kind,
            ordinal,
        })
    }

    /// Language tag after normalization, for display and diagnostics.
    pub fn lang_str(&self) -> &str {
        &self.lang
    }

    fn checked_lang(&self) -> Result<String, SymbolIdError> {
        check_lang(&nfc(&self.lang))?;
        Ok(nfc(&self.lang))
    }

    fn checked_lang_lossy(&self) -> String {
        match self.checked_lang() {
            Ok(lang) => lang,
            Err(_) => format!("x{}", short_hash(self.lang.as_bytes())),
        }
    }
}

impl SymbolId {
    /// Strict parser: rejects anything that is not the one canonical spelling of its parts.
    ///
    /// ```
    /// use review_core::ids::SymbolId;
    ///
    /// let id = SymbolId::parse("ts:src/auth/auth.service#AuthService.authorize/method").unwrap();
    /// assert_eq!(id.as_str(), "ts:src/auth/auth.service#AuthService.authorize/method");
    /// // `%2F` is the canonical spelling of an escaped `/` inside a name segment.
    /// assert!(SymbolId::parse("ts:src/a#a%2Fb/function").is_ok());
    /// // Lowercase hex, and escapes of bytes that need none, are not canonical.
    /// assert!(SymbolId::parse("ts:src/a#a%2fb/function").is_err());
    /// assert!(SymbolId::parse("ts:src/a#a%41/function").is_err());
    /// ```
    pub fn parse(raw: &str) -> Result<Self, SymbolIdError> {
        let parts = SymbolIdParts::parse(raw)?;
        let id = parts.format()?;
        if id.as_str() != raw {
            return Err(SymbolIdError::NonCanonical(format!(
                "canonical form is {:?}",
                id.as_str()
            )));
        }
        Ok(id)
    }

    /// Builds an id from parts, validating them (see [`SymbolIdParts::format`]).
    pub fn from_parts(parts: &SymbolIdParts) -> Result<Self, SymbolIdError> {
        parts.format()
    }

    /// The parts of this id. Only an id produced by [`SymbolId::parse`],
    /// [`SymbolId::from_parts`] or an analyzer can be decomposed; a hand-written string that is not
    /// canonical is rejected here.
    pub fn parts(&self) -> Result<SymbolIdParts, SymbolIdError> {
        SymbolIdParts::parse(self.as_str())
    }
}

impl FromStr for SymbolId {
    type Err = SymbolIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        SymbolId::parse(s)
    }
}

impl SymbolIdParts {
    /// Convenience accessor used by the delta writer and the diagnostics.
    pub fn describe(&self) -> String {
        format!(
            "{}:{}#{}",
            self.lang,
            self.module_path,
            self.qualified_name.join(".")
        )
    }
}

impl fmt::Display for SymbolIdParts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.format() {
            Ok(id) => f.write_str(id.as_str()),
            Err(_) => f.write_str(&self.describe()),
        }
    }
}

/// Two or more files of one language that would produce the same `module_path`, and the paths
/// involved. IDX reports these; the derivation itself keeps the collision out of the ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModulePathCollision {
    /// Language the colliding files belong to; the tag is part of the id, so files of different
    /// languages never collide.
    pub language: Language,
    /// The stripped module path both files map to.
    pub module_path: ModulePath,
    /// The colliding paths, sorted.
    pub paths: Vec<RepoPath>,
}

/// The `module_path` of one file (ADR-005): repository-relative, forward slashes, no leading
/// `./` or `/`, no `..`, final extension stripped, NFC-normalized, case preserved.
///
/// `src/a.service.ts` gives `src/a.service`, `a.d.ts` gives `a.d` (so `a.ts` and `a.d.ts` never
/// share an id) and `a.tsx` gives `a`.
pub fn module_path_for(path: &RepoPath, language: Language) -> ModulePath {
    let stripped = strip_extension_for(path, language);
    let normalized = nfc(&stripped);
    ModulePath::from_checked(normalized)
}

/// Module paths for a whole file set, applying the collision rule: when several files of one
/// language map to the same module path (`a.ts` and `a.tsx`), every file after the first in
/// byte-lexicographic order of the full path keeps its extension (`src/a.tsx`).
pub fn module_paths(entries: &[(Language, RepoPath)]) -> BTreeMap<RepoPath, ModulePath> {
    let mut by_group: BTreeMap<(Language, String), Vec<RepoPath>> = BTreeMap::new();
    for (language, path) in entries {
        by_group
            .entry((*language, strip_extension_for(path, *language)))
            .or_default()
            .push(path.clone());
    }
    let mut out = BTreeMap::new();
    for ((_, stripped), mut paths) in by_group {
        paths.sort();
        for (index, path) in paths.iter().enumerate() {
            // The first path in byte order keeps the stripped module path; every later one keeps
            // its extension so two files never share an id.
            let module = if index == 0 {
                nfc(&stripped)
            } else {
                nfc(path.as_str())
            };
            out.insert(path.clone(), ModulePath::from_checked(module));
        }
    }
    out
}

/// Colliding module paths of a file set, sorted by language and module path (SID-001: reported by
/// IDX so an operator can see which files were disambiguated).
pub fn module_path_collisions(entries: &[(Language, RepoPath)]) -> Vec<ModulePathCollision> {
    let mut by_group: BTreeMap<(Language, String), Vec<RepoPath>> = BTreeMap::new();
    for (language, path) in entries {
        by_group
            .entry((*language, strip_extension_for(path, *language)))
            .or_default()
            .push(path.clone());
    }
    let mut out: Vec<ModulePathCollision> = by_group
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|((language, stripped), mut paths)| {
            paths.sort();
            ModulePathCollision {
                language,
                module_path: ModulePath::from_checked(nfc(&stripped)),
                paths,
            }
        })
        .collect();
    out.sort_by(|a, b| (a.language, &a.module_path).cmp(&(b.language, &b.module_path)));
    out
}

/// Strips the extension an id must not carry. TypeScript and JavaScript use their exact dialect
/// lists (so `a.d.ts` becomes `a.d` and `a.tsx` becomes `a`); every other language falls back to
/// dropping the final extension, which is what [`RepoPath::module_path`] does.
fn strip_extension_for(path: &RepoPath, language: Language) -> String {
    // `.d.ts` is handled by stripping only `.ts`, which leaves the `.d` in place so a declaration
    // file never shares a module path with its implementation.
    const TS: &[&str] = &["mts", "cts", "tsx", "ts"];
    const JS: &[&str] = &["mjs", "cjs", "jsx", "js"];
    let extensions: &[&str] = match language {
        Language::Typescript => TS,
        Language::Javascript => JS,
        _ => return path.module_path().to_owned(),
    };
    let text = path.as_str();
    for extension in extensions {
        if let Some(stem) = text.strip_suffix(&format!(".{extension}")) {
            if !stem.is_empty() && !stem.ends_with('/') {
                return stem.to_owned();
            }
        }
    }
    text.to_owned()
}

fn nfc(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfc().collect()
}

fn check_lang(lang: &str) -> Result<(), SymbolIdError> {
    if lang.is_empty() {
        return Err(SymbolIdError::BadLang(lang.to_owned()));
    }
    if lang.len() > MAX_LANG_BYTES {
        return Err(SymbolIdError::TooLong {
            field: "language tag",
            actual: lang.len(),
            max: MAX_LANG_BYTES,
        });
    }
    let mut bytes = lang.bytes();
    let first = bytes.next().unwrap_or(0);
    if !first.is_ascii_lowercase() {
        return Err(SymbolIdError::BadLang(lang.to_owned()));
    }
    if !bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
        return Err(SymbolIdError::BadLang(lang.to_owned()));
    }
    if RESERVED_LANG_PREFIXES.contains(&lang) {
        return Err(SymbolIdError::ReservedLang(lang.to_owned()));
    }
    Ok(())
}

fn check_module_path(module: &str) -> Result<(), SymbolIdError> {
    if module.is_empty() {
        return Err(SymbolIdError::BadModulePath("empty".to_owned()));
    }
    if module.len() > MAX_MODULE_PATH_BYTES {
        return Err(SymbolIdError::TooLong {
            field: "module_path",
            actual: module.len(),
            max: MAX_MODULE_PATH_BYTES,
        });
    }
    if module.starts_with('/') {
        return Err(SymbolIdError::BadModulePath("absolute".to_owned()));
    }
    if module.starts_with("./") {
        return Err(SymbolIdError::BadModulePath(
            "leading `./` is not allowed".to_owned(),
        ));
    }
    if module.contains('\\') {
        return Err(SymbolIdError::BadModulePath(
            "backslashes are not allowed".to_owned(),
        ));
    }
    if module.contains('\0') {
        return Err(SymbolIdError::BadModulePath("NUL byte".to_owned()));
    }
    for segment in module.split('/') {
        if segment.is_empty() {
            return Err(SymbolIdError::BadModulePath(
                "empty path segment".to_owned(),
            ));
        }
        if segment == "." || segment == ".." {
            return Err(SymbolIdError::BadModulePath(format!(
                "`{segment}` segment escapes the repository"
            )));
        }
    }
    Ok(())
}

fn check_segment(segment: &str) -> Result<(), SymbolIdError> {
    if segment.is_empty() {
        return Err(SymbolIdError::BadSegment("empty".to_owned()));
    }
    if segment.len() > MAX_SEGMENT_BYTES {
        return Err(SymbolIdError::TooLong {
            field: "segment",
            actual: segment.len(),
            max: MAX_SEGMENT_BYTES,
        });
    }
    Ok(())
}

fn module_error(error: UnescapeError) -> SymbolIdError {
    match error {
        UnescapeError::NonCanonical(reason) => SymbolIdError::NonCanonical(reason),
        UnescapeError::Malformed(reason) => SymbolIdError::BadModulePath(reason),
    }
}

fn segment_error(error: UnescapeError) -> SymbolIdError {
    match error {
        UnescapeError::NonCanonical(reason) => SymbolIdError::NonCanonical(reason),
        UnescapeError::Malformed(reason) => SymbolIdError::BadSegment(reason),
    }
}

fn parse_ordinal(text: &str) -> Result<u16, SymbolIdError> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SymbolIdError::BadOrdinal(text.to_owned()));
    }
    if text.len() > 1 && text.starts_with('0') {
        return Err(SymbolIdError::BadOrdinal(text.to_owned()));
    }
    if text == "0" {
        return Err(SymbolIdError::BadOrdinal(text.to_owned()));
    }
    text.parse::<u16>().map_err(|_| SymbolIdError::TooLong {
        field: "ordinal",
        actual: text.len(),
        max: 5,
    })
}

/// Which part of an id a token belongs to. The two escape differently: `/` separates directories
/// in a module path but must be escaped inside a name segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Module,
    Segment,
}

fn needs_escape(byte: u8, part: Part) -> bool {
    matches!(byte, b'%' | b'#' | b'~')
        || byte <= 0x20
        || byte == 0x7f
        || matches!(part, Part::Segment) && matches!(byte, b'/' | b'.')
}

/// Percent-encodes the bytes that could otherwise change the structure of an id. Everything else,
/// including non-ASCII UTF-8, stays literal.
fn escape_into(s: &str, part: Part, out: &mut Vec<u8>) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for &byte in s.as_bytes() {
        if needs_escape(byte, part) {
            out.push(b'%');
            out.push(HEX[(byte >> 4) as usize]);
            out.push(HEX[(byte & 0x0f) as usize]);
        } else {
            out.push(byte);
        }
    }
}

/// Why a percent escape sequence was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
enum UnescapeError {
    /// Decodable, but not the canonical spelling of this id (lowercase hex, or an escape of a
    /// byte that needs none).
    NonCanonical(String),
    /// Not decodable at all.
    Malformed(String),
}

/// Reverses [`escape_into`]. Rejects a `%` that is not followed by two uppercase hex digits, and
/// an escape of a byte that does not need escaping; both are non-canonical spellings.
fn unescape(s: &str, part: Part) -> Result<String, UnescapeError> {
    let bad = |reason: String| UnescapeError::Malformed(reason);
    let non_canonical = |reason: String| UnescapeError::NonCanonical(reason);
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' {
            let hex = bytes
                .get(index + 1..index + 3)
                .ok_or_else(|| bad(format!("truncated escape at byte {index}")))?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return Err(bad(format!(
                    "non-hex escape `%{}`",
                    String::from_utf8_lossy(hex)
                )));
            }
            if hex.iter().any(u8::is_ascii_lowercase) {
                return Err(non_canonical(format!(
                    "lowercase escape `%{}`",
                    String::from_utf8_lossy(hex)
                )));
            }
            let decoded = u8::from_str_radix(std::str::from_utf8(hex).unwrap_or(""), 16)
                .map_err(|e| bad(e.to_string()))?;
            if !needs_escape(decoded, part) {
                return Err(non_canonical(format!(
                    "unnecessary escape `%{}`",
                    String::from_utf8_lossy(hex)
                )));
            }
            out.push(decoded);
            index += 3;
        } else {
            if needs_escape(byte, part) {
                return Err(non_canonical(format!(
                    "unescaped structural byte 0x{byte:02X} in {s:?}"
                )));
            }
            out.push(byte);
            index += 1;
        }
    }
    String::from_utf8(out).map_err(|e| bad(format!("percent escapes are not valid utf-8: {e}")))
}

fn short_hash(bytes: &[u8]) -> String {
    let digest = Hash128::of(LOSSY_DOMAIN, bytes).to_string();
    digest[..8].to_owned()
}

/// Truncates `text` to `max` bytes on a character boundary, appending a deterministic hash marker
/// when it had to cut.
fn degrade(text: &str, max: usize) -> String {
    let normalized = nfc(text);
    if normalized.len() <= max {
        return normalized;
    }
    let budget = max.saturating_sub(LOSSY_MARKER_BYTES);
    let mut end = budget;
    while end > 0 && !normalized.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}…~{}",
        &normalized[..end],
        short_hash(normalized.as_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::SymbolKey;
    use proptest::prelude::*;

    fn parts(module: &str, qn: &[&str], kind: SymbolKind) -> SymbolIdParts {
        SymbolIdParts::new(
            "ts",
            ModulePath::from_checked(module.to_owned()),
            qn.iter().map(|s| (*s).to_owned()).collect(),
            kind,
        )
    }

    #[test]
    fn adr_example_and_module_symbol() {
        let id = parts(
            "src/auth/auth.service",
            &["AuthService", "authorize"],
            SymbolKind::Method,
        )
        .format()
        .unwrap();
        assert_eq!(
            id.as_str(),
            "ts:src/auth/auth.service#AuthService.authorize/method"
        );
        assert_eq!(
            SymbolKey::of(&id).to_string(),
            "97ac70ec2191e38555c6678614fc4699"
        );
        let module = SymbolIdParts::module(
            "ts",
            ModulePath::from_checked("src/auth/auth.service".to_owned()),
        );
        assert_eq!(
            module.format().unwrap().as_str(),
            "ts:src/auth/auth.service#__module__/module"
        );
    }

    #[test]
    fn special_characters_round_trip() {
        let cases: [(&str, &str, SymbolKind); 8] = [
            ("a.b", "ts:src/a#a%2Eb/function", SymbolKind::Function),
            ("x#y", "ts:src/a#x%23y/function", SymbolKind::Function),
            ("p/q", "ts:src/a#p%2Fq/function", SymbolKind::Function),
            ("t~1", "ts:src/a#t%7E1/function", SymbolKind::Function),
            ("100%", "ts:src/a#100%25/function", SymbolKind::Function),
            ("a b", "ts:src/a#a%20b/function", SymbolKind::Function),
            ("üñí", "ts:src/a#üñí/function", SymbolKind::Function),
            ("#x", "ts:src/a#%23x/method", SymbolKind::Method),
        ];
        for (raw, canonical, kind) in cases {
            let id = parts("src/a", &[raw], kind).format().unwrap();
            assert_eq!(id.as_str(), canonical, "{raw}");
            let back = SymbolId::parse(canonical).unwrap().parts().unwrap();
            assert_eq!(back.qualified_name, vec![raw.to_owned()]);
        }
    }

    #[test]
    fn parse_is_strict() {
        assert_eq!(SymbolId::parse(""), Err(SymbolIdError::Empty));
        assert!(matches!(
            SymbolId::parse("ts:src/a#m/function~01"),
            Err(SymbolIdError::BadOrdinal(_))
        ));
        assert!(matches!(
            SymbolId::parse("ts:src/a#m/function~0"),
            Err(SymbolIdError::BadOrdinal(_))
        ));
        assert!(matches!(
            SymbolId::parse("ts:src/a#m/function~"),
            Err(SymbolIdError::BadOrdinal(_))
        ));
        assert!(matches!(
            SymbolId::parse("ts:src/a#m/nope"),
            Err(SymbolIdError::BadKind(_))
        ));
        assert!(matches!(
            SymbolId::parse("TS:src/a#m/function"),
            Err(SymbolIdError::BadLang(_))
        ));
        assert!(matches!(
            SymbolId::parse("1ts:src/a#m/function"),
            Err(SymbolIdError::BadLang(_))
        ));
        assert!(matches!(
            SymbolId::parse("file:src/a#m/function"),
            Err(SymbolIdError::ReservedLang(_))
        ));
        assert!(matches!(
            SymbolId::parse("ts:../a#m/function"),
            Err(SymbolIdError::BadModulePath(_))
        ));
        assert!(matches!(
            SymbolId::parse("ts:/abs/a#m/function"),
            Err(SymbolIdError::BadModulePath(_))
        ));
        assert!(SymbolId::parse("ts:src/a#m.b/function").is_ok());
        assert!(SymbolId::parse("ts:src/a#a%2Fb/function").is_ok());
        assert!(matches!(
            SymbolId::parse("ts:src/a#a%2fb/function"),
            Err(SymbolIdError::NonCanonical(_))
        ));
        assert!(matches!(
            SymbolId::parse("ts:src/a#a%41/function"),
            Err(SymbolIdError::NonCanonical(_))
        ));
        assert!(SymbolId::parse("ts:src/a#m").is_err());
        assert!(SymbolId::parse("src/a#m/function").is_err());
        assert!(matches!(
            SymbolId::parse("ts:src/a#m/function/x"),
            Err(SymbolIdError::BadKind(_))
        ));
    }

    #[test]
    fn split_on_first_hash_and_last_slash() {
        // The first `#` separates the module path, the last `/` the kind, so a `/` inside a name
        // must be escaped and never splits anything.
        let parsed = SymbolId::parse("ts:src/a#b%2Fc/function")
            .unwrap()
            .parts()
            .unwrap();
        assert_eq!(parsed.qualified_name, vec!["b/c".to_owned()]);
        assert_eq!(parsed.kind, SymbolKind::Function);
        assert!(SymbolId::parse("ts:src/a#b/c/function").is_err());
        // A second `#` is a raw structural byte inside a name segment.
        let raw = "ts:src/we#ird#dir#na%2Fme/function";
        assert!(SymbolId::parse(raw).is_err());
    }

    #[test]
    fn module_paths_and_collisions() {
        let a = RepoPath::new("src/a.ts").unwrap();
        let b = RepoPath::new("src/a.tsx").unwrap();
        let c = RepoPath::new("src/a.d.ts").unwrap();
        let d = RepoPath::new("src/a.py").unwrap();
        assert_eq!(module_path_for(&a, Language::Typescript).as_str(), "src/a");
        assert_eq!(module_path_for(&b, Language::Typescript).as_str(), "src/a");
        assert_eq!(
            module_path_for(&c, Language::Typescript).as_str(),
            "src/a.d"
        );
        let entries = [
            (Language::Typescript, a.clone()),
            (Language::Typescript, b.clone()),
            (Language::Typescript, c.clone()),
            (Language::Python, d.clone()),
        ];
        let map = module_paths(&entries);
        assert_eq!(map[&a].as_str(), "src/a");
        assert_eq!(map[&b].as_str(), "src/a.tsx");
        assert_eq!(map[&c].as_str(), "src/a.d");
        let collisions = module_path_collisions(&entries);
        assert_eq!(collisions.len(), 1);
        assert_eq!(collisions[0].paths, vec![a.clone(), b.clone()]);
        assert_eq!(collisions[0].module_path.as_str(), "src/a");
    }

    #[test]
    fn nfc_and_over_long_parts() {
        let decomposed = "e\u{301}";
        let id = parts("src/a", &[decomposed], SymbolKind::Function)
            .format()
            .unwrap();
        assert_eq!(id.parts().unwrap().qualified_name, vec!["é".to_owned()]);
        let long = "x".repeat(MAX_SEGMENT_BYTES + 1);
        assert!(matches!(
            parts("src/a", &[&long], SymbolKind::Function).format(),
            Err(SymbolIdError::TooLong {
                field: "segment",
                ..
            })
        ));
        let lossy = parts("src/a", &[&long], SymbolKind::Function).format_lossy();
        assert!(lossy.as_str().len() <= MAX_ID_BYTES);
        assert!(lossy.as_str().contains("%7E"));
        assert!(SymbolId::parse(lossy.as_str()).is_ok());
        let long_module = "a".repeat(MAX_MODULE_PATH_BYTES + 1);
        let lossy_module = parts(&long_module, &["m"], SymbolKind::Function).format_lossy();
        assert!(SymbolId::parse(lossy_module.as_str()).is_ok());
        let many: Vec<String> = (0..40)
            .map(|i| format!("segment-{i}-{}", "x".repeat(40)))
            .collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let too_long = parts("src/a", &refs, SymbolKind::Function).format();
        assert!(matches!(too_long, Err(SymbolIdError::TooLong { .. })));
        let lossy_many = parts("src/a", &refs, SymbolKind::Function).format_lossy();
        assert!(lossy_many.as_str().len() <= MAX_ID_BYTES);
        assert!(SymbolId::parse(lossy_many.as_str()).is_ok());
    }

    #[test]
    fn bad_lang_is_degraded_lossily() {
        let mut bad = parts("src/a", &["m"], SymbolKind::Function);
        bad.lang = "NOT A LANG".to_owned();
        assert!(bad.format().is_err());
        let id = bad.format_lossy();
        assert!(SymbolId::parse(id.as_str()).is_ok());
    }

    fn segment() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_]{1,10}".prop_map(|s| s)
    }

    fn raw_segment() -> impl Strategy<Value = String> {
        prop_oneof![
            "[a-zA-Z0-9_]{1,10}",
            "[ %#/~.]{1,6}",
            "[a-z]{0,4}[.][a-z]{0,4}",
            "üñí",
        ]
    }

    proptest! {
        #[test]
        fn parse_format_roundtrip_property(
            lang in "[a-z][a-z0-9_]{0,10}",
            module in prop::collection::vec(segment(), 1..4),
            qn in prop::collection::vec(raw_segment(), 1..4),
            kind in prop::sample::select(SymbolKind::ALL.to_vec()),
            ordinal in prop::option::of(1u16..),
        ) {
            let qn: Vec<String> = qn;
            let p = SymbolIdParts::new(
                lang.clone(),
                ModulePath::from_checked(module.join("/")),
                qn.clone(),
                kind,
            );
            let p = match ordinal { Some(n) => p.with_ordinal(n), None => p };
            let id = p.format().unwrap();
            let back = SymbolId::parse(id.as_str()).unwrap().parts().unwrap();
            prop_assert_eq!(back.qualified_name.clone(), qn);
            prop_assert_eq!(back.kind, kind);
            prop_assert_eq!(back.ordinal, ordinal.unwrap_or(0));
            prop_assert_eq!(back.format().unwrap(), id.clone());
            prop_assert_eq!(SymbolId::from_parts(&back).unwrap(), id.clone());
            prop_assert_eq!(SymbolKey::of(&id), SymbolKey::of(&back.format().unwrap()));
            prop_assert_eq!(id.as_str().parse::<SymbolId>().unwrap(), id);
        }
    }

    proptest! {
        #[test]
        fn formatting_is_stable_across_repeated_calls(p in proptest::sample::select(vec![
            ("src/a", vec!["A", "b"], SymbolKind::Method, 0u16),
            ("src/a.b", vec!["a.b"], SymbolKind::Function, 3),
            ("src/x#y", vec!["p/q"], SymbolKind::Property, 1),
            ("ünï/cødé", vec!["üñí"], SymbolKind::Constant, 0),
        ])) {
            let (module, qn, kind, ordinal) = p;
            let parts = SymbolIdParts::new(
                "ts",
                ModulePath::from_checked(module.to_owned()),
                qn.iter().map(|s| (*s).to_owned()).collect(),
                kind,
            )
            .with_ordinal(ordinal);
            let first = parts.format().unwrap();
            let second = parts.format().unwrap();
            prop_assert_eq!(first.clone(), second);
            prop_assert_eq!(parts.format_lossy(), first);
        }
    }
}
