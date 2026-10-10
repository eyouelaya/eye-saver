//! The exact contents the platform writes to register the app as a login item.
//!
//! macOS reads a LaunchAgent property list; Windows reads a registry string. Both are
//! generated here as pure functions so that the escaping and quoting - the only parts that
//! can actually be wrong - are unit tested, leaving the platform code with nothing to do
//! but write bytes to a known location.

use crate::APP_ID;

/// The LaunchAgent label, which is also the plist file's base name.
pub const LAUNCH_AGENT_LABEL: &str = APP_ID;

/// Registry key, relative to `HKEY_CURRENT_USER`, that Windows reads at sign-in.
pub const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Name of the registry value holding our command line.
pub const RUN_KEY_NAME: &str = "EyeSaver";

/// Passed when the app is started by the system rather than by the user, so that it knows
/// not to open the settings window.
pub const AUTOSTART_FLAG: &str = "--autostart";

/// File name for the LaunchAgent, to be written into `~/Library/LaunchAgents`.
#[must_use]
pub fn launch_agent_file_name() -> String {
    format!("{LAUNCH_AGENT_LABEL}.plist")
}

/// The LaunchAgent property list that starts `program` at login.
///
/// `ProcessType: Interactive` asks launchd not to throttle a UI process, and `KeepAlive`
/// is false because a user who quits the app means it.
#[must_use]
pub fn launch_agent_plist(program: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{program}</string>
		<string>{flag}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<false/>
	<key>ProcessType</key>
	<string>Interactive</string>
</dict>
</plist>
"#,
        label = xml_escape(LAUNCH_AGENT_LABEL),
        program = xml_escape(program),
        flag = AUTOSTART_FLAG,
    )
}

/// The registry value that starts `exe` at sign-in.
///
/// The path is quoted because `%LocalAppData%` contains the user's name, which routinely
/// contains spaces.
#[must_use]
pub fn run_key_command(exe: &str) -> String {
    format!("\"{exe}\" {AUTOSTART_FLAG}")
}

/// Escapes the five XML entities. Paths can legitimately contain `&` and `<`.
fn xml_escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "/Applications/EyeSaver.app/Contents/MacOS/eye-saver";

    #[test]
    fn the_plist_is_named_after_the_label() {
        assert_eq!(launch_agent_file_name(), "com.eyesaver.app.plist");
    }

    #[test]
    fn the_plist_declares_the_label_program_and_autostart_flag() {
        let plist = launch_agent_plist(APP);
        assert!(plist.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
        assert!(plist.contains("<string>com.eyesaver.app</string>"));
        assert!(plist.contains(&format!("<string>{APP}</string>")));
        assert!(plist.contains("<string>--autostart</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(plist.contains("<key>KeepAlive</key>\n\t<false/>"));
        assert!(plist.trim_end().ends_with("</plist>"));
    }

    #[test]
    fn paths_with_xml_significant_characters_are_escaped() {
        let plist = launch_agent_plist("/Users/a&b/<x>/\"q\"/'p'/EyeSaver");
        assert!(plist.contains("/Users/a&amp;b/&lt;x&gt;/&quot;q&quot;/&apos;p&apos;/EyeSaver"));
        assert!(
            !plist.contains("a&b"),
            "a raw ampersand would make the plist unparseable"
        );
    }

    #[test]
    fn the_registry_command_quotes_the_path() {
        assert_eq!(
            run_key_command(r"C:\Users\Ada Lovelace\AppData\Local\Programs\EyeSaver\eye-saver.exe"),
            "\"C:\\Users\\Ada Lovelace\\AppData\\Local\\Programs\\EyeSaver\\eye-saver.exe\" --autostart"
        );
    }

    #[test]
    fn the_registry_location_is_the_per_user_run_key() {
        assert_eq!(
            RUN_KEY_PATH,
            r"Software\Microsoft\Windows\CurrentVersion\Run"
        );
        assert_eq!(RUN_KEY_NAME, "EyeSaver");
    }

    #[test]
    fn escaping_leaves_ordinary_text_alone() {
        assert_eq!(
            xml_escape("/usr/local/bin/eye-saver"),
            "/usr/local/bin/eye-saver"
        );
    }
}
