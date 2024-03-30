# Clapboard - clipboard manager that makes you clap 👏

Clapboard is a simple clipboard manager for Wayland, built in Rust. It saves a history of your clipboard usage, and lets you paste things you've copied earlier. It also lets you configure "favorite" pastes for strings you need often. For example, you can set favorites with your address, phone number, email address etc - and they'll all become just a few clicks away.

[video.webm](https://user-images.githubusercontent.com/55081/211161880-63bb628c-e43d-4e46-9e77-85b5cabb8318.webm)

## Requirements

- [tofi](https://github.com/philj56/tofi) or any other dmenu-like program ([wofi](https://hg.sr.ht/~scoopta/wofi), [rofi](https://github.com/lbonn/rofi), [dmenu](https://github.com/nyyManni/dmenu-wayland))
- [wl-clipboard](https://github.com/bugaevc/wl-clipboard)

## Installation

### From source

- `git clone` the repository
- Run `cargo build --release`
- copy the `clapboard` executable to your PATH

### Arch Linux

Clapboard is available on AUR as [clapboard-git](https://aur.archlinux.org/packages/clapboard-git).

## Usage

- `echo "something" | clapboard store` should run whenever something is copied. If you're using Sway, just add this to your `~/.config/sway/config`:

```
exec wl-paste --watch clapboard store
exec wl-paste -p --watch clapboard store
```

- Optionally, bind some key to run `clapboard`. I'm binding the Favorites key in Sway like this:
```
bindsym XF86Favorites exec clapboard
```

## Configuration

Below is the default Clapboard configuration. If you want to change it, create a similar file at `~/.config/clapboard/config.toml`

```toml
launcher = [ "tofi", "--fuzzy-match=true", "--prompt-text=clap: " ]
history_size = 50

[favorites]
# You can add your favorite clipboard pastes here like this:
# "some key" = "some value"
```

## Fun fact

Clapboard was originally built in Python and later converted to Rust with the help of ChatGPT.
