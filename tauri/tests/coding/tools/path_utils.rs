use ai_toolbox_lib::coding::tools::{
    normalize_path, to_storage_path, NormalizedPath, PathType,
};

#[test]
fn test_tilde_prefix() {
    let result = normalize_path("~/.config/myapp");
    assert_eq!(result.path, ".config/myapp");
    assert_eq!(result.path_type, PathType::HomeRelative);
}

#[test]
fn test_appdata_prefix() {
    let result = normalize_path("%APPDATA%/Code/User");
    assert_eq!(result.path, "Code/User");
    assert_eq!(result.path_type, PathType::AppDataRelative);
}

#[test]
fn test_windows_backslash() {
    let result = normalize_path("%APPDATA%\\Code\\User");
    assert_eq!(result.path, "Code/User");
    assert_eq!(result.path_type, PathType::AppDataRelative);
}

#[test]
fn test_storage_path_home() {
    let normalized = NormalizedPath {
        path: ".config/myapp".to_string(),
        path_type: PathType::HomeRelative,
    };
    assert_eq!(to_storage_path(&normalized), "~/.config/myapp");
}

#[test]
fn test_storage_path_appdata() {
    let normalized = NormalizedPath {
        path: "Code/User".to_string(),
        path_type: PathType::AppDataRelative,
    };
    assert_eq!(to_storage_path(&normalized), "%APPDATA%/Code/User");
}

#[test]
fn test_storage_path_empty_home() {
    let normalized = NormalizedPath {
        path: String::new(),
        path_type: PathType::HomeRelative,
    };
    assert_eq!(to_storage_path(&normalized), "~");
}
