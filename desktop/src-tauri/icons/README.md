`logo-source.png` (1024x1024) is a generated placeholder — swap it for a real logo whenever you
want. `32x32.png`, `128x128.png`, `128x128@2x.png` and `icon.ico` were generated from it and are
already wired up in `tauri.conf.json`, good enough for a Windows build as-is.

`icon.icns` (macOS) isn't included — it needs Apple's icon format, which isn't easily produced
outside macOS/Tauri tooling. If you ever build for macOS, regenerate the full set properly with:

```bash
cargo install tauri-cli --version "^2.0.0"
cd desktop/src-tauri
cargo tauri icon icons/logo-source.png
```

That also re-adds `icon.icns` and you can add it back to `tauri.conf.json`'s `bundle.icon` list.
