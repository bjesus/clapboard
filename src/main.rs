use clap::Parser;
use indexmap::IndexMap;
use std::path::Path;
use std::path::PathBuf;
use std::{
    fs,
    io::{self, Read, Write},
    process::{Command, Stdio},
};
use std::{
    fs::File,
    io::copy,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::task;
use toml::Value;
use wayland_clipboard_listener::WlClipboardPasteStream;
use wayland_clipboard_listener::WlListenType;
use wl_clipboard_rs::copy::{MimeSource, MimeType, Options, Source};
use wl_clipboard_rs::paste::{get_contents, ClipboardType, Seat};
use xdg::BaseDirectories;

/// Clapboard, a clipboard manager for Wayland
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Record mode, choose between "primary", "clipboard", or the default "both"
    #[arg(short, long, num_args(0..=1), default_missing_value = "both")]
    record: Option<String>,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let xdg_dirs = BaseDirectories::with_prefix("clapboard").unwrap();
    let config_path = xdg_dirs
        .place_config_file("config.toml")
        .expect("cannot create configuration directory");

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

    let cache_dir = xdg_dirs.get_cache_home();

    match args.record {
        Some(record) => {
            println!("Clapboard recording {record}...");
            let listeners = if record == "primary" {
                vec!["primary"]
            } else if record == "clipboard" {
                vec!["clipboard"]
            } else if record == "both" {
                vec!["primary", "clipboard"]
            } else {
                vec![]
            };

            // Spawn tasks for each listener
            let tasks: Vec<_> = listeners
                .iter()
                .map(|&paste_type| {
                    task::spawn(listen_to_clipboard(
                        paste_type,
                        cache_dir.clone(),
                        history_size,
                    ))
                })
                .collect();

            // Await each task individually
            for task in tasks {
                let _ = task.await;
            }
        }
        None => {
            let mut data: IndexMap<String, String> = IndexMap::new();

            let mut entries: Vec<_> = fs::read_dir(&cache_dir)
                .unwrap() // Handle the Result from read_dir
                .flatten() // Flatten the Result<Option<DirEntry>> to just DirEntry
                .collect(); // Collect into a vector of DirEntry

            // Sort entries by file name (ascending order)
            entries.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

            // Iterate over sorted entries
            for entry in entries {
                if entry.path().is_dir() {
                    let timestamp = entry.file_name().into_string().unwrap_or_default();
                    let text_files =
                        vec!["UTF8_STRING", "TEXT", "text.plain", "text.html", "STRING"];
                    let mut found_file = false;
                    let mut content = String::new();
                    for file_name in text_files {
                        let textual_representation = entry.path().join(file_name);

                        if textual_representation.exists() {
                            let mut file = File::open(&textual_representation).unwrap();
                            if file.read_to_string(&mut content).is_ok() {
                                found_file = true;
                                break;
                            }
                        }
                    }
                    if found_file {
                        data.insert(
                            content
                                .trim()
                                .to_string()
                                .replace("\n", " ")
                                .chars()
                                .take(50) // Avoid long text
                                .collect(),
                            timestamp.to_string(),
                        );
                    } else {
                        // If no file was found, proceed with the else logic
                        println!("No textfile found for: {}", timestamp.to_string());
                        data.entry(timestamp.to_string())
                            .or_insert_with(|| timestamp.to_string());
                    }
                }
            }
            for (key, value) in favorites {
                data.entry(key.parse().unwrap())
                    .or_insert_with(|| value.as_str().unwrap().to_string());
            }

            let input = data.keys().cloned().collect::<Vec<_>>().join("\n");
            let command_name = launcher.unwrap()[0].as_str().unwrap();
            let mut command = Command::new(command_name);
            for arg in &launcher.unwrap()[1..] {
                command.arg(arg.as_str().unwrap());
            }

            let output = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    child.stdin.as_mut().unwrap().write_all(input.as_bytes())?;
                    child.wait_with_output()
                })
            .unwrap_or_else(|_| panic!("Cannot start your launcher, please confirm you have {} installed or configure another one", command_name));

            let mut result = String::from_utf8_lossy(&output.stdout).into_owned();
            result.pop(); // Remove trailing new line
            if result.len() > 0 {
                let mut opts = Options::new();
                opts.foreground(true); // We need to keep the process alive for pasting to work
                if favorites.contains_key(&result) {
                    opts.copy(
                        Source::Bytes(
                            data.get(&result)
                                .unwrap()
                                .to_string()
                                .into_bytes()
                                .into_boxed_slice(),
                        ),
                        MimeType::Autodetect,
                    )
                    .expect("Failed to copy to clipboard");
                } else {
                    let prefix = data.get(&result).unwrap().as_str();
                    let sources: Vec<MimeSource> =
                        fs::read_dir(format!("{}{}", cache_dir.to_str().unwrap(), prefix))
                            .unwrap()
                            .flatten()
                            .filter_map(|entry| {
                                let path = entry.path();
                                let mime_type = path
                                    .file_name()?
                                    .to_string_lossy()
                                    .to_string()
                                    .replacen(".", "/", 1);
                                fs::read(&path).ok().map(|contents| MimeSource {
                                    source: Source::Bytes(contents.into()),
                                    mime_type: MimeType::Specific(mime_type),
                                })
                            })
                            .collect();

                    if !sources.is_empty() {
                        opts.copy_multi(sources)
                            .expect("Failed to copy to clipboard");
                    }
                }
            }
        }
    }
}

async fn listen_to_clipboard(paste_type: &str, cache_dir: PathBuf, history_size: usize) {
    let mut stream = WlClipboardPasteStream::init(match paste_type {
        "primary" => WlListenType::ListenOnSelect,
        _ => WlListenType::ListenOnCopy,
    })
    .unwrap();

    for context in stream.paste_stream().flatten().flatten() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        for mime in context.mime_types {
            match get_contents(
                match paste_type {
                    "primary" => ClipboardType::Primary,
                    _ => ClipboardType::Regular,
                },
                Seat::Unspecified,
                wl_clipboard_rs::paste::MimeType::Specific(&mime),
            ) {
                Ok((mut reader, _)) => {
                    let path = format!("{}{}", cache_dir.to_str().unwrap(), timestamp);
                    fs::create_dir_all(Path::new(&path)).unwrap();
                    let file_path = format!("{}/{}", &path, mime.replace("/", "."));
                    match File::create(&file_path) {
                        Ok(mut file) => {
                            if let Err(e) = copy(&mut reader, &mut file) {
                                eprintln!("Failed to copy content to {}: {}", file_path, e);
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to create file {}: {}", file_path, e);
                        }
                    }
                }
                Err(err) => eprintln!("Clipboard {paste_type:?} error: {}", err),
            }
        }
        clean_history(&cache_dir, history_size).unwrap();
    }
}

fn clean_history(directory: &Path, max: usize) -> io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(directory)?
        .filter_map(|entry| entry.ok()) // Ignore any errors in reading entries
        .collect();

    entries.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

    for (index, entry) in entries.into_iter().enumerate() {
        if index > max {
            let path = entry.path();
            if path.is_dir()
                && !path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .starts_with('.')
            {
                fs::remove_dir_all(&path)?;
            }
        }
    }
    Ok(())
}
