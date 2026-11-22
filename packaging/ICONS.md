# The WhatsNative icon

The old mark came from upstream's OxideZap organization avatar. It looked like
a WhatsApp bubble with the corners filed off, which is exactly the kind of
lazy identity this fork is not here for. We threw it out.

The new mark is ours: an angular W cut from cyan and violet shards on an
indigo tile. No speech bubble, no phone handset, no WhatsApp green. The source
of truth is [`whatsnative-logo.svg`](whatsnative-logo.svg), hand-written vector
code, committed here so it can be reviewed and changed like any other file.

`linux/org.whatsnative.client.local.png` is the raster master. It is 460 by 460
pixels with a transparent corner outside the rounded tile. The Linux window
embeds it, the desktop installer puts it in the user's icon search path, and
AppImage uses the 256 pixel hicolor copy as its `.DirIcon`.

Web assets come from the same mark, with the transparent margins cropped so
16 pixel favicons stay readable:

- `web/icons/favicon.ico` with 16, 32 and 48 pixel frames
- `web/icons/favicon-32.png`
- `web/icons/apple-touch-icon.png`, opaque on the page background so the OS
  mask does not show black corners

To regenerate, use Python 3.11 or newer, ImageMagick, `rsvg-convert`, and the
pinned tools:

```sh
python3 -m venv .venv-icons
. .venv-icons/bin/activate
python -m pip install -r packaging/requirements.txt
cargo install --locked --version 9.1.5 oxipng
rsvg-convert -w 460 -h 460 packaging/whatsnative-logo.svg -o /tmp/wn-icon.png
python packaging/generate-icons.py /tmp/wn-icon.png
packaging/linux/icons.sh
```

The generated PNGs are checked for decoded pixel equality after optimization.
Trunk hashes the favicon ICO and copies the PNGs beside `index.html`, so the
relative links work in Pages previews and relocated web archives.

Credit where it is due: the icon artwork before this change came from upstream
and is still credited in `docs/our-changes.md`. This one is ours.
