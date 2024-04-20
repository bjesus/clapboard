use atty;
use base64::prelude::*;
use std::collections::VecDeque;
use std::env;
use std::fs;
use std::io::{stdin, BufRead, BufReader, Read, Write};
use std::process::exit;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use toml::Value;
use xdg::BaseDirectories;

fn main() {
    let xdg_dirs = BaseDirectories::with_prefix("clapboard").unwrap();

    let now = SystemTime::now();
    let since_the_epoch = now.duration_since(UNIX_EPOCH).unwrap();
    let timestamp = since_the_epoch.as_millis();

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

    let toml_string = fs::read_to_string(config_path).unwrap_or(String::from(""));
    let config: Value = toml::from_str(&toml_string).unwrap();

    let history_size = config
        .get("history_size")
        .and_then(|v| v.as_integer())
        .unwrap_or(50) as usize;

    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "store" {
        if atty::isnt(atty::Stream::Stdin) {
            let stdin = stdin();
            let mut handle = stdin.lock();
            let mut input = Vec::new();
            if handle.read_to_end(&mut input).unwrap() > 0 {
                let entry = match String::from_utf8(input) {
                    Ok(v) => v,
                    Err(e) => String::from(format!(
                        "file:{} {}",
                        timestamp,
                        BASE64_STANDARD.encode(e.into_bytes())
                    )),
                };
                let mut clipboard_history = fs::OpenOptions::new()
                    .append(true)
                    .open(&history_path)
                    .unwrap();
                writeln!(
                    clipboard_history,
                    "{}",
                    entry.replace("\r", "\\r").replace("\n", "\\n")
                )
                .unwrap();
                truncate_file(&history_path.to_str().unwrap(), history_size).unwrap();

                return;
            }
        }
        exit(0)
    }

    let default_launcher = vec!["tofi", "--fuzzy-match=true", "--prompt-text=clapboard: "];

    let default_launcher_values: Vec<Value> = default_launcher
        .iter()
        .map(|x| Value::String(x.to_string()))
        .collect();
    let default_launcher_value = Value::Array(default_launcher_values);
    let launcher = config
        .get("launcher")
        .unwrap_or_else(|| &default_launcher_value)
        .as_array();

    let default_favorites_value = Value::Table(toml::value::Table::new());
    let favorites = config
        .get("favorites")
        .unwrap_or_else(|| &default_favorites_value)
        .as_table()
        .unwrap();

    let full_history: Vec<String> = fs::read_to_string(&history_path)
        .unwrap()
        .lines()
        .map(|line| line.to_owned())
        .collect();

    let history: Vec<String> = full_history
        .clone()
        .into_iter()
        .map(|line| {
            if line.starts_with("file:") {
                line.split(' ').next().unwrap_or(line.as_str()).to_owned()
            } else {
                line.to_owned()
            }
        })
        .rev()
        .collect();

    let mut event_menu = history;

    for (name, _) in favorites {
        event_menu.push(name.to_owned().to_owned());
    }

    let input = event_menu.join("\n").to_string();
    let command_name = launcher.unwrap()[0].as_str().unwrap();
    let mut command = Command::new(command_name);
    for arg in &launcher.unwrap()[1..] {
        command.arg(arg.as_str().unwrap());
    }

    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to start command");

    let mut stdin = child
        .stdin
        .take()
        .expect(&format!("Failed to open stdin for {}", command_name));
    std::thread::spawn(move || {
        stdin
            .write_all(input.as_bytes())
            .expect("Failed to write to your launcher's stdin");
    });

    let output = child
        .wait_with_output()
        .expect(&format!("Failed to read stdout from {}", command_name));
    let result = String::from_utf8(output.stdout)
        .expect(&format!("Invalid UTF-8 returned from {}", command_name));
    if result.len() > 0 {
        for (name, value) in favorites {
            if result.trim().eq(name.trim()) {
                Command::new("wl-copy")
                    .arg(value.as_str().unwrap())
                    .spawn()
                    .expect("Cannot start wl-copy");
                return;
            }
        }

        if result.starts_with("file:") {
            let result_line = full_history
                .iter()
                .find(|line| line.starts_with(&result.replace("\n", "")))
                .unwrap();
            let data = result_line.splitn(2, ' ').nth(1).unwrap();
            let content = BASE64_STANDARD.decode(data).unwrap();
            let mut child = Command::new("wl-copy")
                .stdin(Stdio::piped())
                .spawn()
                .expect("Cannot spawn wl-copy");
            {
                let stdin = child.stdin.as_mut().expect("Failed to open stdin");
                stdin.write_all(&content).expect("Failed to write to stdin");
            }
        }

        Command::new("wl-copy")
            .arg(strip_trailing_newline(result.as_str()).replace("\\n", "\n"))
            .spawn()
            .expect("Cannot spawn wl-copy");
    }
}

fn truncate_file(path: &str, history_size: usize) -> std::io::Result<()> {
    let f = fs::File::open(path)?;
    let reader = BufReader::new(&f);

    let lines: Vec<String> = reader.lines().collect::<Result<_, _>>()?;

    let mut last_lines = VecDeque::new();
    for line in lines {
        if let Some(position) = last_lines.iter().position(|x| *x == line) {
            last_lines.remove(position);
        }
        last_lines.push_back(line);
        if last_lines.len() > history_size {
            last_lines.pop_front();
        }
    }

    let mut file = fs::File::create(path)?;
    for line in last_lines {
        writeln!(file, "{}", line)?;
    }

    Ok(())
}

fn strip_trailing_newline(input: &str) -> &str {
    input
        .strip_suffix("\r\n")
        .or(input.strip_suffix("\n"))
        .unwrap_or(input)
}
