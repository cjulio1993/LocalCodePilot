use localcodepilot_core::runtimes::RuntimeKind;
use std::path::Path;

pub fn detect(path: &Path) -> Vec<RuntimeKind> {
    let candidates = [
        ("Cargo.toml", RuntimeKind::Rust),
        ("package.json", RuntimeKind::Node),
        ("composer.json", RuntimeKind::Php),
        ("pyproject.toml", RuntimeKind::Python),
        ("requirements.txt", RuntimeKind::Python),
    ];
    let mut found = Vec::new();
    for (marker, runtime) in candidates {
        if path.join(marker).is_file() && !found.contains(&runtime) {
            found.push(runtime);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::SystemTime};

    #[test]
    fn detects_multiple_runtimes() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("localcodepilot-runtime-{nonce}"));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("Cargo.toml"), "").unwrap();
        fs::write(path.join("package.json"), "{}").unwrap();
        assert_eq!(detect(&path), vec![RuntimeKind::Rust, RuntimeKind::Node]);
        fs::remove_dir_all(path).unwrap();
    }
}
