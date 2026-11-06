//! Shared by build.rs and the unit tests: turns the package version into the
//! fields of the Windows version resource.

/// The four numbers of FILEVERSION/PRODUCTVERSION for a semver version:
/// `MAJOR.MINOR.PATCH-alpha.8` becomes `MAJOR,MINOR,PATCH,8` (first number of
/// the pre-release) and a release without pre-release ends in 0.
pub fn version_numbers(version: &str) -> Result<[u16; 4], String> {
    let invalid = || format!("unsupported package version for the Windows resource: {version}");
    let without_build = version.split('+').next().unwrap_or_default();
    let (core, pre) = match without_build.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (without_build, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return Err(invalid());
    }
    let mut numbers = [0u16; 4];
    for (slot, part) in numbers.iter_mut().zip(&parts) {
        *slot = part.parse().map_err(|_| invalid())?;
    }
    if let Some(pre) = pre {
        let first_number = pre.split('.').find_map(|id| id.parse::<u16>().ok());
        numbers[3] = first_number.ok_or_else(invalid)?;
    }
    Ok(numbers)
}

/// The resource script: `template` with the version markers and the icon path filled.
pub fn render_resource(template: &str, version: &str, icon: &str) -> Result<String, String> {
    let numbers = version_numbers(version)?.map(|n| n.to_string()).join(",");
    Ok(template
        .replace("@VERSION_NUMBERS@", &numbers)
        .replace("@VERSION@", version)
        .replace("@ICON@", icon))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_release_number_becomes_the_fourth_field() {
        assert_eq!(version_numbers("0.4.0-alpha.8"), Ok([0, 4, 0, 8]));
        assert_eq!(version_numbers("0.4.0-alpha.5.1"), Ok([0, 4, 0, 5]));
        assert_eq!(version_numbers("1.2.3-rc.2+build.7"), Ok([1, 2, 3, 2]));
    }

    #[test]
    fn a_release_without_pre_release_ends_in_zero() {
        assert_eq!(version_numbers("1.0.0"), Ok([1, 0, 0, 0]));
    }

    #[test]
    fn rejects_versions_the_resource_cannot_represent() {
        for version in ["0.4", "0.4.0-alpha", "70000.0.0", "0.4.x"] {
            assert!(version_numbers(version).is_err(), "{version}");
        }
    }

    #[test]
    fn the_template_gets_the_package_version_and_no_marker_is_left() {
        let template = include_str!("app.rc.in");
        let resource = render_resource(template, "0.4.0-alpha.8", "localcodepilot.ico").unwrap();
        assert!(resource.contains("FILEVERSION 0,4,0,8"));
        assert!(resource.contains("PRODUCTVERSION 0,4,0,8"));
        assert!(resource.contains(r#"VALUE "FileVersion", "0.4.0-alpha.8\0""#));
        assert!(resource.contains(r#"VALUE "ProductVersion", "0.4.0-alpha.8\0""#));
        assert!(resource.contains(r#"1 ICON "localcodepilot.ico""#));
        for marker in ["@VERSION@", "@VERSION_NUMBERS@", "@ICON@"] {
            assert!(!resource.contains(marker), "{marker} left in:\n{resource}");
        }
        assert!(!resource.contains("alpha.5"), "{resource}");
    }
}
