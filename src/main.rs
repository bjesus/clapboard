use std::env;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use toml::Value;
use xdg::BaseDirectories;

fn main() {
    let xdg_dirs = BaseDirectories::with_prefix("clapboard").unwrap();

    let history_path = xdg_dirs
        .place_cache_file("history.txt")
        .expect("cannot create cache file");

    let file_exists: bool = std::path::Path::new(&history_path).exists();

    if !file_exists {
        fs::File::create(&history_path).expect("Cannot create a history file");
    }

    let config_path = xdg_dirs
        .place_config_file("config.toml")
        .expect("cannot create configuration directory");

    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "store" {
        if let Some(arg) = args.get(2) {
            let mut clipboard_history = fs::OpenOptions::new()
                .append(true)
                .open(&history_path)
                .unwrap();
            write!(
                clipboard_history,
                "{}\n",
                arg.replace("\r", "\\r").replace("\n", "\\n")
            )
            .unwrap();
            return;
        }
    }

    let toml_string = fs::read_to_string(config_path).unwrap_or(String::from(""));
    let value: Value = toml::from_str(&toml_string).unwrap();

    let default_launcher = vec!["tofi", "--fuzzy-match=true", "--prompt-text=clapboard: "];

    let default_launcher_values: Vec<Value> = default_launcher
        .iter()
        .map(|x| Value::String(x.to_string()))
        .collect();
    let default_launcher_value = Value::Array(default_launcher_values);
    let launcher = value
        .get("launcher")
        .unwrap_or_else(|| &default_launcher_value)
        .as_array();

    let history_size = value
        .get("history_size")
        .and_then(|v| v.as_integer())
        .unwrap_or(50) as usize;

    let default_favorites_value = Value::Table(toml::value::Table::new());
    let favorites = value
        .get("favorites")
        .unwrap_or_else(|| &default_favorites_value)
        .as_table()
        .unwrap();

    let history: Vec<String> = fs::read_to_string(&history_path)
        .unwrap()
        .lines()
        .map(|line| line.to_owned())
        .collect();

    let history: Vec<String> = history
        .iter()
        .skip(
            history
                .len()
                .saturating_sub(usize::try_from(history_size).unwrap()),
        )
        .cloned()
        .collect();

    let history_clone = history.clone();
    let mut event_menu = history;

    for (name, _) in favorites {
        event_menu.push(name.to_owned().to_owned());
    }

    let input = event_menu.join("\n").to_string();

    let mut command = Command::new(launcher.unwrap()[0].as_str().unwrap());
    for arg in &launcher.unwrap()[1..] {
        command.arg(arg.as_str().unwrap());
    }

    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to start command");

    let mut stdin = child.stdin.take().expect("Failed to open stdin for tofi");
    std::thread::spawn(move || {
        stdin
            .write_all(input.as_bytes())
            .expect("Failed to write to tofi's stdin");
    });

    let output = child
        .wait_with_output()
        .expect("Failed to read stdout from tofi");
    let result = String::from_utf8(output.stdout).expect("Invalid UTF-8 returned from tofi");

    for (name, value) in favorites {
        if result.trim().eq(name.trim()) {
            Command::new("wl-copy")
                .arg(value.as_str().unwrap())
                .spawn()
                .expect("Cannot start wl-copy");
            return;
        }
    }

    Command::new("wl-copy")
        .arg(result.replace("\\n", "\n"))
        .spawn()
        .expect("Cannot spawn wl-copy");

    let mut clipboard_history = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&history_path)
        .unwrap();

    for line in history_clone.iter().rev() {
        writeln!(clipboard_history, "{}", line).unwrap();
    }
}
