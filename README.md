# Download Tool

Desktop downloader for large HTTP files. A website opens:

```text
dltool://add?url=<percent-encoded-http-url>&fileName=...&size=...
```

The inner download URL must be `http://`. `fileName`, `size`, `referrer`, and `userAgent` are optional. A cold start parses those arguments and enqueues the download. If the app is already running, the new process forwards the same arguments and exits.

## Protocol registration

Startup tries to register the `dltool` scheme. Failure is printed and the app still opens.

### Windows

The current user hive:

- `HKCU\Software\Classes\dltool` — default `URL:Download Tool Protocol`, and an empty `URL Protocol` value
- `HKCU\Software\Classes\dltool\shell\open\command` — `"<path-to-download-tool>" "%1"`

### Linux

Writes `dltool.desktop` to `$XDG_DATA_HOME/applications` (or `~/.local/share/applications`) with `MimeType=x-scheme-handler/dltool;`, then runs:

```bash
xdg-mime default dltool.desktop x-scheme-handler/dltool
```

### macOS

Launch Services associates a scheme only with an app bundle. Put this in the bundle `Info.plist`:

```xml
<key>CFBundleURLTypes</key>
<array>
    <dict>
        <key>CFBundleURLName</key>
        <string>Download Tool</string>
        <key>CFBundleURLSchemes</key>
        <array>
            <string>dltool</string>
        </array>
    </dict>
</array>
```

Refresh Launch Services after the bundle is in place (a bundled build does this itself when `Info.plist` already lists `dltool`):

```bash
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f /path/to/Download\ Tool.app
```

`cargo run` is a bare executable, not a `.app`. macOS will not open `dltool://` links from that binary. Pass the URL on the command line instead:

```bash
cargo run -p download-app -- 'dltool://add?url=http%3A%2F%2F127.0.0.1%2Ffile.bin&fileName=file.bin'
```
