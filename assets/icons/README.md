# App icon

`sniplet.svg` is the source. Export it as a transparent 1024 by 1024 PNG to
`sniplet-1024.png`, then run `python3 scripts/generate-icons.py` from the repository
root. The script needs Pillow and creates the PNG sizes, Windows ICO, and macOS
ICNS files. These files are checked in, so normal builds need no image tools.

The app icon uses the same Lucide scissors shape as the menu bar and system tray.
The tray uses the plain symbol. The app icon adds capture corners and a dark tile.
See `LICENSE-LUCIDE` for the scissors license.
