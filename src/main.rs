use clap::Parser;
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    io::{self, Read, Write},
    path::Path,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
use wayland_clipboard_listener::{WlClipboardPasteStream, WlListenType};
use wl_clipboard_rs::copy::{MimeSource, MimeType as CopyMimeType, Options, Source};
use wl_clipboard_rs::paste::{get_contents, ClipboardType, MimeType as PasteMimeType, Seat};
use xdg::BaseDirectories;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// Clapboard, a clipboard manager for Wayland
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Record mode, choose between "primary", "clipboard", or the default "both"
    #[arg(short, long, num_args(0..=1), default_missing_value = "both")]
    record: Option<String>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, PartialEq, Eq)]
struct Config {
    #[serde(default = "Config::default_launcher")]
    launcher: Vec<String>,
    #[serde(default)]
    favorites: BTreeMap<String, String>,
    #[serde(default = "Config::default_hist_size")]
    history_size: usize,
}
impl Config {
    fn default_launcher() -> Vec<String> {
        let default_launcher = ["tofi", "--fuzzy-match=true", "--prompt-text=clapboard: "];
        default_launcher.iter().map(ToString::to_string).collect()
    }
    const fn default_hist_size() -> usize {
        50
    }
}

fn main() -> Res<()> {
    let args = Args::parse();

    let xdg_dirs = BaseDirectories::with_prefix("clapboard")?;
    let config_path = xdg_dirs
        .place_config_file("config.toml")
        .expect("cannot create configuration directory");

    let toml_string = fs::read_to_string(config_path).unwrap_or_default();
    let config: Config = basic_toml::from_str(&toml_string)?;

    let cache_dir = xdg_dirs.get_cache_home();

    if let Some(record) = args.record {
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
                std::thread::spawn({
                    let cache_dir = cache_dir.clone();
                    move || listen_to_clipboard(paste_type, cache_dir, config.history_size).unwrap()
                })
            })
            .collect();

        // Await each task individually
        for task in tasks {
            let _ = task.join().inspect_err(|e| eprintln!("error: {e:?}"));
        }
    } else {
        history(&config, cache_dir)?;
    }
    Ok(())
}

fn history(config: &Config, cache_dir: impl AsRef<Path>) -> Res<()> {
    let mut data = HashMap::new();

    let mut entries: Vec<_> = fs::read_dir(&cache_dir)?.flatten().collect();
    // Sort entries by file name (ascending order)
    entries.sort_by_key(|k| k.file_name());

    // Iterate over sorted entries
    for entry in entries {
        if entry.path().is_dir() {
            let timestamp = entry.file_name().into_string().unwrap_or_default();
            let text_files = vec!["UTF8_STRING", "TEXT", "text.plain", "text.html", "STRING"];
            let mut found_file = false;
            let mut content = String::new();
            for file_name in text_files {
                let textual_representation = entry.path().join(file_name);

                if textual_representation.exists() {
                    let mut file = File::open(&textual_representation)?;
                    if file.read_to_string(&mut content).is_ok() {
                        found_file = true;
                        break;
                    }
                }
            }
            if found_file {
                // trims long text
                let description = content.trim().replace('\n', " ").chars().take(50).collect();
                data.insert(description, timestamp);
            } else {
                println!("No textfile found for: {timestamp}");
                data.entry(timestamp.clone()).or_insert(timestamp);
            }
        }
    }
    for (key, value) in &config.favorites {
        data.entry(key.parse()?).or_insert(value.clone());
    }

    let input = data.keys().cloned().collect::<Vec<_>>().join("\n");
    let command_name = &config.launcher[0];
    let mut command = Command::new(command_name);
    for arg in &config.launcher[1..] {
        command.arg(arg);
    }

    let output = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.as_mut().unwrap().write_all(input.as_bytes())?;
            child.wait_with_output()
        })
        .expect("Cannot start your launcher, please confirm you have {command_name} installed or configure another one");

    let result = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if result.is_empty() {
        return Ok(());
    }
    let mut opts = Options::new();
    opts.foreground(true); // We need to keep the process alive for pasting to work
    if config.favorites.contains_key(&result) {
        opts.copy(
            Source::Bytes(
                data.get(&result)
                    .unwrap()
                    .to_string()
                    .into_bytes()
                    .into_boxed_slice(),
            ),
            CopyMimeType::Autodetect,
        )
        .expect("Failed to copy to clipboard");
    } else {
        let prefix = data.get(&result).unwrap();
        let sources = fs::read_dir(format!("{}{prefix}", cache_dir.as_ref().display()))?
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let mime_type = path
                    .file_name()?
                    .to_string_lossy()
                    .to_string()
                    .replacen('.', "/", 1);
                fs::read(&path).ok().map(|contents| MimeSource {
                    source: Source::Bytes(contents.into()),
                    mime_type: CopyMimeType::Specific(mime_type),
                })
            })
            .collect::<Vec<_>>();

        if !sources.is_empty() {
            opts.copy_multi(sources)
                .expect("Failed to copy to clipboard");
        }
    }
    Ok(())
}

fn listen_to_clipboard(paste_type: &str, cache_dir: impl AsRef<Path>, hist_size: usize) -> Res<()> {
    let listentype = match paste_type {
        "primary" => WlListenType::ListenOnSelect,
        _ => WlListenType::ListenOnCopy,
    };
    let mut stream = WlClipboardPasteStream::init(listentype)?;

    for context in stream.paste_stream().flatten().flatten() {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        for mime in context.mime_types {
            let clip_type = match paste_type {
                "primary" => ClipboardType::Primary,
                _ => ClipboardType::Regular,
            };
            match get_contents(clip_type, Seat::Unspecified, PasteMimeType::Specific(&mime)) {
                Ok((mut reader, _)) => {
                    let dir = cache_dir.as_ref().join(timestamp.to_string());
                    fs::create_dir_all(&dir)?;
                    let file_path = dir.join(mime.replace('/', "."));
                    let file_path_disp = file_path.display();
                    match File::create(&file_path) {
                        Ok(mut file) => {
                            let _ = io::copy(&mut reader, &mut file).inspect_err(|e| {
                                eprintln!("Failed to copy content to {file_path_disp}: {e}");
                            });
                        }
                        Err(e) => {
                            eprintln!("Failed to create file {file_path_disp}: {e}");
                        }
                    }
                }
                Err(err) => eprintln!("Clipboard {paste_type:?} error: {err}"),
            }
        }
        clean_history(&cache_dir, hist_size)?;
    }
    Ok(())
}

fn clean_history(directory: impl AsRef<Path>, max: usize) -> io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(directory)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|k| k.file_name());

    for entry in entries.into_iter().skip(max) {
        let path = entry.path();
        if path.is_dir()
            && path
                .file_stem()
                .is_some_and(|p| p.to_str().is_some_and(|s| s.starts_with('.')))
        {
            fs::remove_dir_all(&path)?;
        }
    }
    Ok(())
}
