use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const SERVICE_WORKER_SOURCE: &str = "web/sw.js";

fn normalize_shell_path(path: &str) -> Result<Option<String>, String> {
    if path == "./" || path == "." {
        // The root URL resolves to index.html; require that file to be listed explicitly below.
        return Ok(None);
    }

    let path = path.strip_prefix("./").unwrap_or(path);
    if path.contains('#') {
        return Err(format!(
            "APP_SHELL entry contains a URL fragment instead of a file path: {path}"
        ));
    }
    if path.contains('\\') {
        return Err(format!(
            "APP_SHELL entry contains a backslash instead of a URL path separator: {path}"
        ));
    }
    if path.split('/').any(has_percent_encoded_dot_segment) {
        return Err(format!(
            "APP_SHELL entry contains a percent-encoded dot segment that aliases a canonical URL path: {path}"
        ));
    }
    if path.starts_with('/') || path.split('/').any(|part| part == ".." || part.is_empty()) {
        return Err(format!(
            "APP_SHELL entry is not a relative file path: {path}"
        ));
    }

    Ok(Some(path.to_owned()))
}

fn has_percent_encoded_dot_segment(segment: &str) -> bool {
    let decoded = segment.to_ascii_lowercase().replace("%2e", ".");
    decoded == "." || decoded == ".."
}

fn app_shell_assets(source: &str) -> Result<BTreeSet<String>, String> {
    const MARKER: &str = "const APP_SHELL = [";
    if source.matches(MARKER).count() != 1 {
        return Err("expected exactly one APP_SHELL declaration".to_owned());
    }

    let mut lines = source.lines().map(str::trim);
    let mut found = false;
    let mut cache_name_seen = false;
    for line in lines.by_ref() {
        if line == MARKER {
            found = true;
            break;
        }
        let cache_name = line
            .strip_prefix("const CACHE_NAME = \"")
            .and_then(|value| value.strip_suffix("\";"));
        let valid_cache_name = cache_name.is_some_and(|value| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
        });
        if valid_cache_name && std::mem::replace(&mut cache_name_seen, true) {
            return Err("multiple CACHE_NAME declarations".to_owned());
        }
        if !line.is_empty() && !valid_cache_name {
            return Err(format!("unsupported service worker preamble: {line}"));
        }
    }
    if !found {
        return Err("could not find const APP_SHELL in web/sw.js".to_owned());
    }

    let mut assets = BTreeSet::new();
    let mut has_root_url = false;
    let mut closed = false;
    for entry in lines {
        if entry == "];" {
            closed = true;
            break;
        }
        if entry.is_empty() {
            continue;
        }
        let path = entry
            .strip_suffix(',')
            .and_then(|entry| entry.trim().strip_prefix('"'))
            .and_then(|entry| entry.strip_suffix('"'))
            .ok_or_else(|| format!("unsupported APP_SHELL entry: {entry}"))?;
        match normalize_shell_path(path)? {
            Some(path) if !assets.insert(path.clone()) => {
                return Err(format!("APP_SHELL lists {path} more than once"));
            }
            Some(_) => {}
            None if has_root_url => {
                return Err("APP_SHELL lists the root URL more than once".to_owned());
            }
            None => has_root_url = true,
        }
    }
    if !closed {
        return Err("could not find the end of APP_SHELL in web/sw.js".to_owned());
    }
    if has_root_url && !assets.contains("index.html") {
        return Err("APP_SHELL includes a root URL but does not list \"./index.html\"".to_owned());
    }
    if assets.is_empty() {
        return Err("APP_SHELL contains no file paths".to_owned());
    }
    Ok(assets)
}

fn release_assets(build_script: &str) -> Result<BTreeSet<String>, String> {
    const WASM_BUILD: [&str; 5] = [
        r"wasm-pack build \",
        r"--release \",
        r"--target web \",
        r"--out-dir web/pkg \",
        "--out-name rustbound_idle",
    ];
    let lines: Vec<_> = build_script.lines().map(str::trim).collect();
    let mut assets = BTreeSet::new();
    let mut index = 0;
    let mut copied = false;

    while index < lines.len() {
        let line = lines[index];
        index += 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == WASM_BUILD[0] {
            if lines.get(index - 1..index + 4) != Some(WASM_BUILD.as_slice()) {
                return Err("unsupported wasm-pack build command".to_owned());
            }
            index += 4;
            continue;
        }
        if matches!(
            line,
            "set -euo pipefail"
                | r#"ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)""#
                | "cd \"$ROOT_DIR\""
                | r#"printf '\nBuilt static site in %s/dist\n' "$ROOT_DIR""#
                | r#"printf 'wasm size: %s bytes\n' "$(wc -c < dist/pkg/rustbound_idle_bg.wasm)""#
        ) {
            continue;
        }
        if matches!(line, "rm -rf dist" | "mkdir -p dist/pkg dist/icons") && !copied {
            continue;
        }
        let words: Vec<_> = line.split_whitespace().collect();
        if words.first() != Some(&"cp") || words.len() < 3 {
            return Err(format!("unsupported build command on line {index}: {line}"));
        }
        let destination = words[words.len() - 1];
        let directory = match destination {
            "dist/" => "",
            "dist/icons/" => "icons/",
            "dist/pkg/" => "pkg/",
            _ => {
                return Err(format!(
                    "unsupported copy destination on line {index}: {destination}"
                ));
            }
        };
        for source in &words[1..words.len() - 1] {
            let valid = source.starts_with("web/")
                && source
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_./-".contains(&byte))
                && !source
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..");
            if !valid {
                return Err(format!("unsupported copy source on line {index}: {source}"));
            }
            if *source != SERVICE_WORKER_SOURCE || !directory.is_empty() {
                let filename = source
                    .rsplit('/')
                    .next()
                    .expect("valid source has a filename");
                assets.insert(format!("{directory}{filename}"));
            }
        }
        copied = true;
    }
    if assets.is_empty() {
        return Err("found no files copied into dist by scripts/build.sh".to_owned());
    }
    Ok(assets)
}

fn check_app_shell(build_script: &str, service_worker: &str) -> Result<(), String> {
    let shipped = release_assets(build_script)?;
    let cached = app_shell_assets(service_worker)?;
    let missing: Vec<_> = shipped.difference(&cached).cloned().collect();
    let stale: Vec<_> = cached.difference(&shipped).cloned().collect();

    if missing.is_empty() && stale.is_empty() {
        return Ok(());
    }

    let mut details =
        vec!["web/sw.js APP_SHELL does not match scripts/build.sh output:".to_owned()];
    if !missing.is_empty() {
        details.push(format!(
            "not cached by the service worker: {}",
            missing.join(", ")
        ));
    }
    if !stale.is_empty() {
        details.push(format!(
            "listed in APP_SHELL but not shipped in dist: {}",
            stale.join(", ")
        ));
    }

    Err(details.join("\n"))
}

#[test]
fn service_worker_cache_matches_release_outputs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let build_script = fs::read_to_string(root.join("scripts/build.sh")).unwrap();
    let service_worker = fs::read_to_string(root.join(SERVICE_WORKER_SOURCE)).unwrap();

    let lines: Vec<_> = build_script.lines().map(str::trim).collect();
    assert!(
        lines
            .windows(2)
            .any(|pair| pair == ["rm -rf dist", "mkdir -p dist/pkg dist/icons"]),
        "the release build must clear dist and recreate its directories before copying"
    );

    check_app_shell(&build_script, &service_worker).unwrap();
}

#[test]
fn missing_or_unshipped_app_shell_files_have_a_useful_error() {
    let build_script = "cp web/index.html dist/\ncp web/styles.css dist/";
    let service_worker = r#"const APP_SHELL = [
  "./index.html",
  "./missing.css",
];"#;

    let error = check_app_shell(build_script, service_worker).unwrap_err();
    assert!(
        error.contains("not cached by the service worker: styles.css"),
        "{error}"
    );
    assert!(
        error.contains("listed in APP_SHELL but not shipped in dist: missing.css"),
        "{error}"
    );
}
#[test]
fn duplicate_root_urls_are_rejected() {
    for service_worker in [
        "const APP_SHELL = [\n  \"./index.html\",\n  \"./\",\n  \"./\",\n];",
        "const APP_SHELL = [\n  \"./index.html\",\n  \".\",\n  \"./\",\n];",
    ] {
        let error = app_shell_assets(service_worker).unwrap_err();
        assert!(error.contains("root URL more than once"), "{error}");
    }
}

#[test]
fn release_grammar_rejects_shell_forms_outside_the_build_script() {
    for script in [
        "command cp web/index.html dist/",
        "cd dist\ncp ../web/index.html ./",
        "wasm-pack build \\\ncp web/index.html dist/",
    ] {
        assert!(release_assets(script).is_err(), "accepted {script:?}");
    }
}

#[test]
fn commented_out_app_shell_declarations_are_rejected_as_ambiguous() {
    let build_script = "cp web/index.html dist/\ncp web/styles.css dist/";
    let service_worker = r#"/*
const APP_SHELL = [
  "./index.html",
  "./styles.css",
];
*/
const APP_SHELL = [
  "./index.html",
  "./missing.css",
];"#;

    let error = check_app_shell(build_script, service_worker).unwrap_err();
    assert!(
        error.contains("expected exactly one APP_SHELL declaration"),
        "{error}"
    );
}

#[test]
fn declarations_inside_comments_and_strings_are_rejected() {
    for source in [
        "/*\nconst APP_SHELL = [\n\"./index.html\",\n];\n*/",
        "const example = `\nconst APP_SHELL = [\n\"./index.html\",\n];\n`;",
    ] {
        assert!(app_shell_assets(source).is_err(), "accepted {source:?}");
    }
}

#[test]
fn app_shell_paths_with_url_fragments_are_rejected() {
    let error =
        app_shell_assets("const APP_SHELL = [\n  \"./index.html\",\n  \"./extra#icon.png\",\n];")
            .unwrap_err();

    assert!(error.contains("URL fragment"), "{error}");
}

#[test]
fn percent_encoded_dot_segments_are_rejected() {
    for path in [
        "./%2e",
        "./%2E",
        "./.%2e",
        "./%2e.",
        "./%2e%2e",
        "./%2E%2E",
        "./icons/%2e/icon-192.png",
    ] {
        let service_worker = format!("const APP_SHELL = [\n  \"./index.html\",\n  \"{path}\",\n];");
        let error = app_shell_assets(&service_worker).unwrap_err();
        assert!(
            error.contains("percent-encoded dot segment"),
            "{path}: {error}"
        );
    }
}

#[test]
fn raw_backslashes_in_app_shell_entries_are_rejected() {
    let service_worker = r#"const APP_SHELL = [
  "./index.html",
  "./icons\icon-192.png",
];"#;

    let error = app_shell_assets(service_worker).unwrap_err();
    assert!(error.contains("contains a backslash"), "{error}");
}

#[test]
fn root_url_entry_is_covered_by_an_explicit_index_file_entry() {
    let build_script = "cp web/index.html dist/";
    let service_worker = "const APP_SHELL = [\n  \"./\",\n  \"./index.html\",\n];";

    assert!(check_app_shell(build_script, service_worker).is_ok());
    let error = app_shell_assets("const APP_SHELL = [\n  \"./\",\n];").unwrap_err();
    assert!(error.contains("does not list \"./index.html\""), "{error}");
}

#[test]
fn unsupported_shell_syntax_fails_closed() {
    for command in [
        "cp web/icons/icon-192.png ~+/dist/extra-icon.png",
        "cp ~/icon.png dist/",
        "mkdir ~+/dist/icons",
        "rm ~+/dist/styles.css",
        "cd ~+",
        "cd -",
        "if false; then",
        "command rm dist/styles.css",
        "rm dist/{index.html,styles.css}",
        "cp web/index.html dist",
        "cp -t dist/ web/index.html",
        "cp web/index.html ./dist/",
        "cp ./web/sw.js dist/",
        "cp web/index.html dist/ # comment",
        "cp web/index.html \"dist/\"",
        "cp web/index.html 'dist/icons\\icon.png'",
        "cp web/index.html dist/ && echo done",
        "cp web/index.html \"$ROOT_DIR/dist/\"",
        "mkdir -p dist/icons",
        "cp web/index.html dist/\nrm -rf dist",
    ] {
        assert!(release_assets(command).is_err(), "accepted {command:?}");
    }
}

#[test]
fn static_copies_report_added_and_removed_cache_entries() {
    let build = "cp web/index.html web/extra.js dist/";
    let worker = "const APP_SHELL = [\n  \"./index.html\",\n  \"./stale.js\",\n];";
    let error = check_app_shell(build, worker).unwrap_err();
    assert!(
        error.contains("not cached by the service worker: extra.js"),
        "{error}"
    );
    assert!(
        error.contains("listed in APP_SHELL but not shipped in dist: stale.js"),
        "{error}"
    );
}

#[test]
fn service_worker_is_exempt_only_at_the_root_destination() {
    let assets = release_assets("cp web/sw.js dist/\ncp web/sw.js dist/icons/").unwrap();
    assert_eq!(assets, BTreeSet::from(["icons/sw.js".to_owned()]));
}

#[test]
fn unsupported_commands_appended_to_real_build_are_rejected() {
    let build = include_str!("../scripts/build.sh");
    for extra in [
        "cd dist\ncd -\nrm dist/styles.css",
        "if false; then\ncp web/styles.css dist/\nfi",
        "cp web/icons/icon-192.png ~+/dist/extra-icon.png",
    ] {
        let changed = format!("{build}\n{extra}\n");
        assert!(release_assets(&changed).is_err(), "accepted {extra:?}");
    }
}
