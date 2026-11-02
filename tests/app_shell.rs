use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const SERVICE_WORKER_SOURCE: &str = "web/sw.js";

fn normalize_shell_path(path: &str) -> Result<Option<String>, String> {
    if path == "./" || path == "." {
        // The root URL resolves to index.html, which is listed explicitly.
        return Ok(None);
    }

    let path = path.strip_prefix("./").unwrap_or(path);
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
        let line = line.trim();
        let Some(copy_command) = line.strip_prefix("cp ") else {
            continue;
        };

        let arguments: Vec<_> = copy_command.split_whitespace().collect();
        if arguments.len() < 2 {
            return Err(format!(
                "copy command on scripts/build.sh line {} has no destination",
                line_number + 1
            ));
        }

        let destination = arguments[arguments.len() - 1];
        if destination != "dist" && !destination.starts_with("dist/") {
            continue;
        }

        let destination_is_directory = destination == "dist" || destination.ends_with('/');
        for source in &arguments[..arguments.len() - 1] {
            if *source == SERVICE_WORKER_SOURCE {
                // A service worker is installed by the browser and does not precache itself.
                continue;
            }

            let output = if destination_is_directory {
                let filename = Path::new(source)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| format!("could not read filename from build source {source}"))?;
                format!("{destination}{filename}")
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
