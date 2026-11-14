pub fn executable(command: &str) -> Option<String> {
    let text = command.trim();
    let path = if let Some(quoted) = text.strip_prefix('"') {
        quoted.split('"').next()?
    } else {
        let end = text.to_ascii_lowercase().find(".exe")? + 4;
        &text[..end]
    };
    path.to_ascii_lowercase()
        .ends_with(".exe")
        .then(|| path.to_string())
}
pub fn name(path: &str) -> &'static str {
    let path = path.to_ascii_lowercase();
    if path.contains("\\chromium\\") {
        return "Chromium";
    }
    match path.rsplit('\\').next().unwrap_or("") {
        "comet.exe" => "Comet",
        "msedge.exe" => "Microsoft Edge",
        "brave.exe" => "Brave",
        "vivaldi.exe" => "Vivaldi",
        "chromium.exe" => "Chromium",
        "arc.exe" => "Arc",
        "opera.exe" => "Opera",
        _ => "Google Chrome",
    }
}

pub fn browser_display_name(id: &str) -> &'static str {
    let lower = id.to_ascii_lowercase();
    if lower.contains("comet") {
        "Comet"
    } else if lower.contains("msedge") || lower.contains("edgemac") {
        "Microsoft Edge"
    } else if lower.contains("brave") {
        "Brave"
    } else if lower.contains("vivaldi") {
        "Vivaldi"
    } else if lower.contains("chromium") {
        "Chromium"
    } else if lower.contains("arc") || lower.contains("thebrowser") {
        "Arc"
    } else if lower.contains("operagx") {
        "Opera GX"
    } else if lower.contains("opera") {
        "Opera"
    } else {
        "Google Chrome"
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_commands_keep_spaces_and_drop_arguments() {
        assert_eq!(
            executable(r#""C:\Program Files\Comet\comet.exe" -- "%1""#).unwrap(),
            r"C:\Program Files\Comet\comet.exe"
        );
        assert_eq!(
            executable(r"C:\Program Files\Chrome\chrome.exe --new-window").unwrap(),
            r"C:\Program Files\Chrome\chrome.exe"
        );
        assert!(executable("not a command").is_none());
        assert_eq!(name(r"C:\Apps\Comet\comet.exe"), "Comet");
    }
}
