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
    if path.contains('\\') {
        return Err(format!(
            "APP_SHELL entry contains a backslash instead of a URL path separator: {path}"
        ));
    }
    if path.starts_with('/') || path.split('/').any(|part| part == ".." || part.is_empty()) {
        return Err(format!(
            "APP_SHELL entry is not a relative file path: {path}"
        ));
    }

    Ok(Some(path.replace('\\', "/")))
}

fn app_shell_assets(source: &str) -> Result<BTreeSet<String>, String> {
    let marker = "const APP_SHELL = [";
    let start = source
        .find(marker)
        .ok_or_else(|| "could not find const APP_SHELL in web/sw.js".to_owned())?
        + marker.len();
    let contents = &source[start..];
    let end = contents
        .find("];")
        .ok_or_else(|| "could not find the end of APP_SHELL in web/sw.js".to_owned())?;

    let mut assets = BTreeSet::new();
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

        if let Some(path) = normalize_shell_path(path)?
            && !assets.insert(path.clone())
        {
            return Err(format!("APP_SHELL lists {path} more than once"));
        }
    }

    if assets.is_empty() {
        return Err("APP_SHELL contains no file paths".to_owned());
    }

    Ok(assets)
}

fn release_assets(build_script: &str) -> Result<BTreeSet<String>, String> {
    let mut assets = BTreeSet::new();

    for (line_number, line) in build_script.lines().enumerate() {
        let line = strip_shell_comment(line).trim();
        if !line
            .strip_prefix("cp")
            .is_some_and(|rest| rest.chars().next().is_some_and(char::is_whitespace))
        {
            continue;
        }

        let arguments = shell_words(line).map_err(|error| {
            format!(
                "could not parse copy command on scripts/build.sh line {}: {error}",
                line_number + 1
            )
        })?;
        if arguments.len() < 2 {
            return Err(format!(
                "copy command on scripts/build.sh line {} has no destination",
                line_number + 1
            ));
        }

        let destination = arguments.last().expect("at least two arguments");
        if destination != "dist" && !destination.starts_with("dist/") {
            continue;
        }

        let sources = &arguments[1..arguments.len() - 1];
        if sources.is_empty() || sources.iter().any(|source| source.starts_with('-')) {
            return Err(format!(
                "copy command on scripts/build.sh line {} uses unsupported cp options or has no source",
                line_number + 1
            ));
        }
        if std::iter::once(destination)
            .chain(sources.iter())
            .any(|operand| {
                operand
                    .chars()
                    .any(|character| matches!(character, '$' | '`' | '*' | '?' | '[' | ']'))
            })
        {
            return Err(format!(
                "copy command on scripts/build.sh line {} uses shell expansion that cannot be checked safely",
                line_number + 1
            ));
        }

        let destination_is_directory = destination == "dist" || destination.ends_with('/');
        for source in sources {
            if *source == SERVICE_WORKER_SOURCE {
                // A service worker is installed by the browser and does not precache itself.
                continue;
            }

            let output = if destination_is_directory {
                let filename = Path::new(source)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| format!("could not read filename from build source {source}"))?;
                format!("{}/{filename}", destination.trim_end_matches('/'))
            } else {
                destination.to_owned()
            };

            let relative = output
                .strip_prefix("dist/")
                .ok_or_else(|| format!("build output {output} is not inside the dist directory"))?;
            if relative.is_empty() || relative.split('/').any(|part| part == "..") {
                return Err(format!("invalid release output path: {output}"));
            }
            assets.insert(relative.replace('\\', "/"));
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let build_script = fs::read_to_string(root.join("scripts/build.sh")).unwrap();
    let service_worker = fs::read_to_string(root.join(SERVICE_WORKER_SOURCE)).unwrap();
    let service_worker = service_worker.replace("\"./styles.css\",", "").replace(
        "\"./icons/icon-512.png\",",
        "\"./icons/icon-512.png\",\n  \"./missing.css\",",
    );

    let error = check_app_shell(&build_script, &service_worker).unwrap_err();
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
    assert!(check_app_shell(build_script, "const APP_SHELL = [\n  \"./\",\n];").is_err());
}
