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

fn normalize_source_path(path: &str) -> String {
    let is_absolute = path.starts_with('/');
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." if components.last().is_some_and(|last| *last != "..") => {
                let _ = components.pop();
            }
            ".." if !is_absolute => components.push(component),
            ".." => {}
            _ => components.push(component),
        }
    }

    let normalized = components.join("/");
    if is_absolute {
        format!("/{normalized}")
    } else {
        normalized
    }
}

fn strip_javascript_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut result = String::with_capacity(source.len());
    let mut index = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;

    while index < bytes.len() {
        let byte = bytes[index];
        let char_len = source[index..]
            .chars()
            .next()
            .expect("index remains on a character boundary")
            .len_utf8();

        if line_comment {
            if byte == b'\n' {
                result.push('\n');
                line_comment = false;
            } else {
                result.push(' ');
            }
            index += char_len;
            continue;
        }

        if block_comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                result.push_str("  ");
                index += 2;
                block_comment = false;
            } else {
                if byte == b'\n' {
                    result.push('\n');
                } else {
                    result.push(' ');
                }
                index += char_len;
            }
            continue;
        }

        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == delimiter {
                quote = None;
            }
            result.push_str(&source[index..index + char_len]);
            index += char_len;
            continue;
        }

        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            result.push_str("  ");
            index += 2;
            line_comment = true;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            result.push_str("  ");
            index += 2;
            block_comment = true;
            continue;
        }
        if matches!(byte, 0x27 | 0x22 | 0x60) {
            quote = Some(byte);
        }

        result.push_str(&source[index..index + char_len]);
        index += char_len;
    }

    result
}

fn javascript_code_marker_positions(source: &str, marker: &str) -> Vec<usize> {
    let bytes = source.as_bytes();
    let mut positions = Vec::new();
    let mut index = 0;
    let mut quote = None;
    let mut escaped = false;

    while index < bytes.len() {
        let byte = bytes[index];
        let char_len = source[index..]
            .chars()
            .next()
            .expect("index remains on a character boundary")
            .len_utf8();

        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == delimiter {
                quote = None;
            }
            index += char_len;
            continue;
        }

        if matches!(byte, 0x27 | 0x22 | 0x60) {
            quote = Some(byte);
            index += char_len;
            continue;
        }

        if source[index..].starts_with(marker) {
            positions.push(index);
            index += marker.len();
        } else {
            index += char_len;
        }
    }

    positions
}

fn app_shell_assets(source: &str) -> Result<BTreeSet<String>, String> {
    let source = strip_javascript_comments(source);
    let marker = "const APP_SHELL = [";
    let declarations = javascript_code_marker_positions(&source, marker);
    let start = match declarations.as_slice() {
        [] => return Err("could not find const APP_SHELL in web/sw.js".to_owned()),
        [start] => start + marker.len(),
        _ => {
            return Err(
                "found multiple active const APP_SHELL declarations in web/sw.js".to_owned(),
            );
        }
    };
    let contents = &source[start..];
    let end = javascript_code_marker_positions(contents, "];")
        .first()
        .copied()
        .ok_or_else(|| "could not find the end of APP_SHELL in web/sw.js".to_owned())?;

    let mut assets = BTreeSet::new();
    let mut has_root_url = false;
    for (line_number, line) in contents[..end].lines().enumerate() {
        let entry = line.trim();
        if entry.is_empty() {
            continue;
        }

        let entry = entry.strip_suffix(',').unwrap_or(entry).trim();
        let path = entry
            .strip_prefix('"')
            .and_then(|entry| entry.strip_suffix('"'))
            .ok_or_else(|| {
                format!(
                    "APP_SHELL entry on line {} is not a quoted path: {entry}",
                    line_number + 1
                )
            })?;

        match normalize_shell_path(path)? {
            Some(path) => {
                if !assets.insert(path.clone()) {
                    return Err(format!("APP_SHELL lists {path} more than once"));
                }
            }
            None => {
                if has_root_url {
                    return Err("APP_SHELL lists the root URL more than once".to_owned());
                }
                has_root_url = true;
            }
        }
    }

    if has_root_url && !assets.contains("index.html") {
        return Err("APP_SHELL includes a root URL but does not list \"./index.html\"".to_owned());
    }

    if assets.is_empty() {
        return Err("APP_SHELL contains no file paths".to_owned());
    }

    Ok(assets)
}
fn contains_shell_expansion(operand: &str) -> bool {
    operand
        .chars()
        .any(|character| matches!(character, '$' | '\x60' | '*' | '?' | '[' | ']'))
}

fn record_created_directory(directories: &mut BTreeSet<String>, path: &str, create_parents: bool) {
    let path = normalize_source_path(path);
    if path != "dist" && !path.starts_with("dist/") {
        return;
    }

    if create_parents {
        let mut prefix = String::new();
        for component in path.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            if prefix == "dist" || prefix.starts_with("dist/") {
                directories.insert(prefix.clone());
            }
        }
    } else {
        directories.insert(path);
    }
}

fn release_assets(build_script: &str) -> Result<BTreeSet<String>, String> {
    let mut assets: BTreeSet<String> = BTreeSet::new();
    let mut created_directories = BTreeSet::new();

    for (line_number, line) in build_script.lines().enumerate() {
        let line = strip_shell_comment(line).trim();
        let line_number = line_number + 1;

        if line
            .strip_prefix("mkdir")
            .is_some_and(|rest| rest.chars().next().is_some_and(char::is_whitespace))
        {
            let arguments = shell_words(line).map_err(|error| {
                format!(
                    "could not parse mkdir command on scripts/build.sh line {line_number}: {error}"
                )
            })?;
            let mut create_parents = false;
            let mut parse_options = true;
            let mut paths = Vec::new();

            for operand in arguments.iter().skip(1) {
                if parse_options && operand == "--" {
                    parse_options = false;
                } else if parse_options && matches!(operand.as_str(), "-p" | "--parents") {
                    create_parents = true;
                } else if parse_options && operand.starts_with('-') {
                    return Err(format!(
                        "mkdir command on scripts/build.sh line {line_number} uses unsupported option {operand}"
                    ));
                } else {
                    if contains_shell_expansion(operand) {
                        return Err(format!(
                            "mkdir command on scripts/build.sh line {line_number} uses shell expansion that cannot be checked safely"
                        ));
                    }
                    paths.push(operand.as_str());
                }
            }

            if paths.is_empty() {
                return Err(format!(
                    "mkdir command on scripts/build.sh line {line_number} has no directory"
                ));
            }
            for path in paths {
                record_created_directory(&mut created_directories, path, create_parents);
            }
            continue;
        }

        if line
            .strip_prefix("rm")
            .is_some_and(|rest| rest.chars().next().is_some_and(char::is_whitespace))
        {
            let arguments = shell_words(line).map_err(|error| {
                format!(
                    "could not parse rm command on scripts/build.sh line {line_number}: {error}"
                )
            })?;
            let mut parse_options = true;
            let mut recursive = false;
            let mut paths = Vec::new();

            for operand in arguments.iter().skip(1) {
                if parse_options && operand == "--" {
                    parse_options = false;
                } else if parse_options
                    && matches!(
                        operand.as_str(),
                        "-r" | "-f" | "-rf" | "-fr" | "--recursive" | "--force"
                    )
                {
                    recursive |= matches!(operand.as_str(), "-r" | "-rf" | "-fr" | "--recursive");
                } else if parse_options && operand.starts_with('-') {
                    return Err(format!(
                        "rm command on scripts/build.sh line {line_number} uses unsupported option {operand}"
                    ));
                } else {
                    if contains_shell_expansion(operand) {
                        return Err(format!(
                            "rm command on scripts/build.sh line {line_number} uses shell expansion that cannot be checked safely"
                        ));
                    }
                    paths.push(normalize_source_path(operand));
                }
            }

            if paths.is_empty() {
                return Err(format!(
                    "rm command on scripts/build.sh line {line_number} has no path"
                ));
            }
            for path in paths {
                if path == "dist" {
                    if recursive {
                        assets.clear();
                        created_directories.clear();
                    }
                } else if let Some(relative) = path.strip_prefix("dist/") {
                    if recursive {
                        let output_prefix = format!("{path}/");
                        assets.retain(|asset| {
                            asset != relative && !asset.starts_with(&format!("{relative}/"))
                        });
                        created_directories.retain(|directory| {
                            directory != &path && !directory.starts_with(&output_prefix)
                        });
                    } else {
                        assets.remove(relative);
                        if !assets
                            .iter()
                            .any(|asset| asset.starts_with(&format!("{relative}/")))
                        {
                            created_directories.remove(&path);
                        }
                    }
                }
            }
            continue;
        }

        let copy_line = line
            .strip_prefix("command")
            .filter(|rest| rest.chars().next().is_some_and(char::is_whitespace))
            .map(str::trim_start)
            .unwrap_or(line);

        if !copy_line
            .strip_prefix("cp")
            .is_some_and(|rest| rest.chars().next().is_some_and(char::is_whitespace))
        {
            continue;
        }

        let arguments = shell_words(copy_line).map_err(|error| {
            format!("could not parse copy command on scripts/build.sh line {line_number}: {error}")
        })?;
        if arguments.len() < 2 {
            return Err(format!(
                "copy command on scripts/build.sh line {line_number} has no destination"
            ));
        }

        let operands = &arguments[1..];
        if operands.is_empty() || operands.iter().any(|operand| operand.starts_with('-')) {
            return Err(format!(
                "copy command on scripts/build.sh line {line_number} uses unsupported cp options or has no source"
            ));
        }
        if operands
            .iter()
            .any(|operand| contains_shell_expansion(operand))
        {
            return Err(format!(
                "copy command on scripts/build.sh line {line_number} uses shell expansion that cannot be checked safely"
            ));
        }

        let raw_destination = arguments.last().expect("at least two arguments");
        let destination_has_trailing_slash = raw_destination.ends_with('/');
        let destination = normalize_source_path(raw_destination);
        if destination != "dist" && !destination.starts_with("dist/") {
            continue;
        }

        let sources = &arguments[1..arguments.len() - 1];
        if sources.is_empty() {
            return Err(format!(
                "copy command on scripts/build.sh line {line_number} uses unsupported cp options or has no source"
            ));
        }

        let destination_is_directory = sources.len() > 1
            || destination == "dist"
            || destination_has_trailing_slash
            || created_directories.contains(&destination);
        for source in sources {
            if normalize_source_path(source) == SERVICE_WORKER_SOURCE {
                // A service worker is installed by the browser and does not precache itself.
                continue;
            }

            let output = if destination_is_directory {
                let filename = Path::new(source)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| format!("could not read filename from build source {source}"))?;
                format!("{destination}/{filename}")
            } else {
                destination.clone()
            };

            if output.contains('\\') {
                return Err(format!(
                    "release output path contains a backslash: {output}"
                ));
            }

            let relative = output
                .strip_prefix("dist/")
                .ok_or_else(|| format!("build output {output} is not inside the dist directory"))?;
            if relative.is_empty() || relative.split('/').any(|part| part == "..") {
                return Err(format!("invalid release output path: {output}"));
            }
            assets.insert(relative.to_owned());
        }
    }

    if assets.is_empty() {
        return Err("found no files copied into dist by scripts/build.sh".to_owned());
    }

    Ok(assets)
}

fn strip_shell_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;

    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }

        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            }
            continue;
        }

        match character {
            '\'' | '"' => quote = Some(character),
            '#' if index == 0
                || line[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace) =>
            {
                return &line[..index];
            }
            _ => {}
        }
    }

    line
}

fn shell_words(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut escaped_in_double_quotes = false;
    let mut started = false;

    for character in line.chars() {
        if escaped {
            if escaped_in_double_quotes && !matches!(character, '$' | '`' | '"' | '\\') {
                word.push('\\');
            }
            word.push(character);
            escaped = false;
            escaped_in_double_quotes = false;
            started = true;
            continue;
        }

        match quote {
            Some('\'') => {
                if character == '\'' {
                    quote = None;
                } else {
                    word.push(character);
                }
            }
            Some('"') => match character {
                '"' => quote = None,
                '\\' => {
                    escaped = true;
                    escaped_in_double_quotes = true;
                }
                _ => word.push(character),
            },
            _ => match character {
                '\'' | '"' => {
                    quote = Some(character);
                    started = true;
                }
                '\\' => {
                    escaped = true;
                    escaped_in_double_quotes = false;
                    started = true;
                }
                character if character.is_whitespace() => {
                    if started {
                        words.push(std::mem::take(&mut word));
                        started = false;
                    }
                }
                ';' | '&' | '|' | '<' | '>' => {
                    return Err(format!(
                        "unquoted shell control operator {character:?} cannot be checked safely"
                    ));
                }
                _ => {
                    word.push(character);
                    started = true;
                }
            },
        }
    }

    if escaped {
        return Err("line ends with an incomplete escape".to_owned());
    }
    if quote.is_some() {
        return Err("line contains an unterminated quoted operand".to_owned());
    }
    if started {
        words.push(word);
    }

    Ok(words)
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
fn removed_single_release_files_are_not_reported_as_shipped() {
    let build_script = "cp web/index.html dist/\ncp web/styles.css dist/\nrm dist/styles.css";
    let service_worker = "const APP_SHELL = [\n  \"./index.html\",\n  \"./styles.css\",\n];";

    let error = check_app_shell(build_script, service_worker).unwrap_err();
    assert!(
        error.contains("listed in APP_SHELL but not shipped in dist: styles.css"),
        "{error}"
    );
}

#[test]
fn recursively_removed_release_files_are_not_reported_as_shipped() {
    let build_script = "cp web/index.html dist/\nmkdir -p dist/icons\ncp web/icons/icon-192.png dist/icons/\nrm -rf dist/icons";
    let service_worker =
        "const APP_SHELL = [\n  \"./index.html\",\n  \"./icons/icon-192.png\",\n];";

    let error = check_app_shell(build_script, service_worker).unwrap_err();
    assert!(
        error.contains("listed in APP_SHELL but not shipped in dist: icons/icon-192.png"),
        "{error}"
    );
}

#[test]
fn commented_out_app_shell_declarations_do_not_hide_the_active_array() {
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
        error.contains("not cached by the service worker: styles.css"),
        "{error}"
    );
    assert!(
        error.contains("listed in APP_SHELL but not shipped in dist: missing.css"),
        "{error}"
    );
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
fn dot_segments_are_normalized_before_exempting_the_service_worker() {
    let service_worker = "const APP_SHELL = [\n  \"./index.html\",\n];";
    for source in ["./web/sw.js", "././web/sw.js", "web/./sw.js"] {
        let build_script = format!("cp web/index.html dist/\ncp {source} dist/");
        assert!(
            check_app_shell(&build_script, service_worker).is_ok(),
            "service-worker source {source:?} should not be added to the cache list"
        );
    }
}

#[test]
fn trailing_shell_comments_do_not_hide_copied_files() {
    let build_script = "cp web/icons/icon-192.png dist/extra-icon.png # publish extra icon";
    let service_worker = "const APP_SHELL = [\n  \"./index.html\",\n];";

    let error = check_app_shell(build_script, service_worker).unwrap_err();
    assert!(
        error.contains("not cached by the service worker: extra-icon.png"),
        "{error}"
    );
}

#[test]
fn bare_dist_destination_is_joined_with_the_source_filename() {
    let build_script = "cp web/index.html dist";
    let service_worker = "const APP_SHELL = [\n  \"./index.html\",\n];";

    assert!(check_app_shell(build_script, service_worker).is_ok());
}

#[test]
fn command_builtin_invocation_tracks_release_copies() {
    let assets = release_assets("command cp web/icons/icon-192.png dist/extra-icon.png").unwrap();

    assert_eq!(assets, BTreeSet::from(["extra-icon.png".to_owned()]));
}

#[test]
fn quoted_release_destinations_are_unquoted_before_checking() {
    let build_script = r#"cp web/icons/icon-192.png "dist/extra-icon.png""#;
    let service_worker = "const APP_SHELL = [\n  \"./index.html\",\n];";

    let error = check_app_shell(build_script, service_worker).unwrap_err();
    assert!(
        error.contains("not cached by the service worker: extra-icon.png"),
        "{error}"
    );
}

#[test]
fn dot_prefixed_dist_destinations_are_normalized_before_filtering() {
    let build_script = "cp web/index.html dist/\ncp web/icons/icon-192.png ./dist/extra-icon.png";
    let service_worker = "const APP_SHELL = [\n  \"./index.html\",\n];";

    let error = check_app_shell(build_script, service_worker).unwrap_err();
    assert!(
        error.contains("not cached by the service worker: extra-icon.png"),
        "{error}"
    );
}

#[test]
fn cp_target_directory_options_are_rejected_before_destination_filtering() {
    let error = release_assets("cp -t dist web/icons/icon-192.png").unwrap_err();
    assert!(error.contains("unsupported cp options"), "{error}");
}

#[test]
fn release_output_paths_with_backslashes_are_rejected() {
    let error =
        release_assets(r#"cp web/icons/icon-192.png 'dist/icons\icon-192.png'"#).unwrap_err();
    assert!(
        error.contains("release output path contains a backslash"),
        "{error}"
    );
}

#[test]
fn unquoted_shell_control_operators_are_rejected() {
    for operator in [";", "&&", "||", "|", ">", "<", "&"] {
        let build_script =
            format!("cp web/icons/icon-192.png dist/extra-icon.png {operator} echo done");
        let error = release_assets(&build_script).unwrap_err();
        assert!(error.contains("unquoted shell control operator"), "{error}");
    }
}

#[test]
fn dynamic_copy_destinations_are_rejected_before_release_filtering() {
    let error =
        release_assets(r#"cp web/icons/icon-192.png "$ROOT_DIR/dist/extra-icon.png""#).unwrap_err();
    assert!(error.contains("uses shell expansion"), "{error}");
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
fn single_source_copies_to_a_created_directory_keep_their_filenames() {
    let build_script = "mkdir -p dist/icons\ncp web/icons/icon-192.png dist/icons\ncp web/icons/icon-512.png dist/icons";
    let service_worker =
        "const APP_SHELL = [\n  \"./icons/icon-192.png\",\n  \"./icons/icon-512.png\",\n];";

    assert!(check_app_shell(build_script, service_worker).is_ok());

    let worker_missing_second_icon = "const APP_SHELL = [\n  \"./icons/icon-192.png\",\n];";
    let error = check_app_shell(build_script, worker_missing_second_icon).unwrap_err();
    assert!(
        error.contains("not cached by the service worker: icons/icon-512.png"),
        "{error}"
    );
}

#[test]
fn removed_directories_are_not_used_to_classify_later_copy_destinations() {
    let build_script =
        "mkdir -p dist/icons\nrm -rf dist/icons\ncp web/icons/icon-192.png dist/icons";
    let service_worker = "const APP_SHELL = [\n  \"./index.html\",\n];";

    let error = check_app_shell(build_script, service_worker).unwrap_err();
    assert!(
        error.contains("not cached by the service worker: icons"),
        "{error}"
    );
}

#[test]
fn multiple_sources_make_a_nested_destination_a_directory() {
    let assets = release_assets("cp web/icon-a.png web/icon-b.png dist/icons").unwrap();

    assert_eq!(
        assets,
        BTreeSet::from(["icons/icon-a.png".to_owned(), "icons/icon-b.png".to_owned()])
    );
}
