//! Node.js built-in module names (TSA-009). A bare specifier in this list, with or without the
//! `node:` prefix, is a built-in rather than an npm package.

const BUILTINS: &[&str] = &[
    "assert",
    "assert/strict",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "dns/promises",
    "domain",
    "events",
    "fs",
    "fs/promises",
    "http",
    "http2",
    "https",
    "inspector",
    "module",
    "net",
    "os",
    "path",
    "path/posix",
    "path/win32",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "readline/promises",
    "repl",
    "stream",
    "stream/promises",
    "stream/web",
    "string_decoder",
    "sys",
    "timers",
    "timers/promises",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "util/types",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

/// The built-in name of `specifier`, without a `node:` prefix, when it is one. Names that only
/// exist with the prefix (`node:test`, `node:sqlite`) are accepted when prefixed.
pub fn builtin_name(specifier: &str) -> Option<&str> {
    if let Some(rest) = specifier.strip_prefix("node:") {
        return Some(rest);
    }
    BUILTINS.contains(&specifier).then_some(specifier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixed_and_bare_names() {
        assert_eq!(builtin_name("node:fs"), Some("fs"));
        assert_eq!(builtin_name("fs"), Some("fs"));
        assert_eq!(builtin_name("fs/promises"), Some("fs/promises"));
        assert_eq!(builtin_name("node:test"), Some("test"));
        assert_eq!(builtin_name("test"), None);
        assert_eq!(builtin_name("lodash"), None);
    }
}
