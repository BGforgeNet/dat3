/*!
# Per-directory configuration

Optional `.bgforge.yml` in the current directory. Only one key is read:

```yaml
dat3:
  default_format: arcanum
```

It sets the format used when `a` creates a new archive and no `--format`
was given. An unusable file or value produces a warning and falls back to
the built-in default, so a foreign or broken config never blocks the tool.
*/

use std::path::Path;

use crate::ArchiveFormat;

/// Config file name, looked up in the process working directory
pub const CONFIG_FILE: &str = ".bgforge.yml";

/// Read `dat3.default_format` from `dir`'s config file.
/// Missing file, missing key, or an unusable value all yield `None`;
/// the unusable cases print a warning first.
pub fn default_format(dir: &Path) -> Option<ArchiveFormat> {
    let path = dir.join(CONFIG_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            eprintln!("Warning: {}: {e}; using the default format", path.display());
            return None;
        }
    };
    match parse_default_format(&text) {
        Ok(format) => format,
        Err(problem) => {
            eprintln!(
                "Warning: {}: {problem}; using the default format",
                path.display()
            );
            None
        }
    }
}

/// Extract and validate the key. `Err` carries the warning text.
fn parse_default_format(text: &str) -> Result<Option<ArchiveFormat>, String> {
    let docs =
        yaml_rust2::YamlLoader::load_from_str(text).map_err(|e| format!("not valid YAML ({e})"))?;
    let Some(doc) = docs.first() else {
        return Ok(None);
    };

    let value = &doc["dat3"]["default_format"];
    match value {
        yaml_rust2::Yaml::BadValue => Ok(None),
        yaml_rust2::Yaml::String(s) => <ArchiveFormat as clap::ValueEnum>::from_str(s, false)
            .map(Some)
            .map_err(|_| {
                format!("unsupported dat3.default_format {s:?} (expected dat1, dat2, or arcanum)")
            }),
        other => Err(format!(
            "dat3.default_format must be a string, got {other:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_each_supported_format() {
        for (name, expected) in [
            ("dat1", ArchiveFormat::Dat1),
            ("dat2", ArchiveFormat::Dat2),
            ("arcanum", ArchiveFormat::Arcanum),
        ] {
            let text = format!("dat3:\n  default_format: {name}\n");
            assert_eq!(parse_default_format(&text), Ok(Some(expected)));
        }
    }

    #[test]
    fn ignores_missing_key_and_unrelated_content() {
        assert_eq!(parse_default_format(""), Ok(None));
        assert_eq!(parse_default_format("other_tool:\n  key: 1\n"), Ok(None));
        assert_eq!(parse_default_format("dat3:\n  other: x\n"), Ok(None));
    }

    #[test]
    fn warns_on_unsupported_value() {
        let err = parse_default_format("dat3:\n  default_format: zip\n").unwrap_err();
        assert!(err.contains("unsupported"), "got: {err}");
        assert!(err.contains("zip"), "got: {err}");
    }

    #[test]
    fn warns_on_non_string_value() {
        let err = parse_default_format("dat3:\n  default_format: 2\n").unwrap_err();
        assert!(err.contains("must be a string"), "got: {err}");
    }

    #[test]
    fn warns_on_invalid_yaml() {
        let err = parse_default_format("dat3: [unclosed\n").unwrap_err();
        assert!(err.contains("not valid YAML"), "got: {err}");
    }

    #[test]
    fn missing_file_is_silent_none() {
        let dir = crate::test_support::ScratchPath::new("cfg");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(default_format(&dir), None);

        std::fs::write(dir.join(CONFIG_FILE), "dat3:\n  default_format: arcanum\n").unwrap();
        assert_eq!(default_format(&dir), Some(ArchiveFormat::Arcanum));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
