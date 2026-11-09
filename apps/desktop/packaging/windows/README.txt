Orbit for Windows

Orbit shows your team's Orbit in its own window, with a tray icon, notifications for new
messages and mentions, and an unread badge on the taskbar button.

Install: run Orbit-<version>-windows-x64-setup.exe. It installs for you only (no admin
rights) in %LOCALAPPDATA%\Programs\Orbit, adds Orbit to the Start menu (and, if you tick it,
to the desktop), and starts Orbit. Remove it in Settings > Apps. Your settings and sign-in
stay in %APPDATA%\orbit-desktop and %LOCALAPPDATA%\orbit-desktop; delete those folders to
remove them too. To update, run the newer installer.

Zip: unpack it anywhere and start Orbit.exe. Without the installer's Start-menu shortcut,
Windows names "Windows PowerShell" as the sender of Orbit's notifications.

The app is not signed yet: on the first start Windows SmartScreen says "Windows protected
your PC". Click "More info", then "Run anyway".

The first time, enter your Orbit server's address (for example orbit.wellrox.com), then sign in
as in the browser. Closing the window keeps Orbit in the tray (next to the clock); right-click
the tray icon for "Start with Windows" (off until you turn it on), "Change server..." and
"Quit Orbit". Downloads go to your Downloads folder. Orbit runs once: starting it again brings its window
to the front.

Needs the Microsoft Edge WebView2 runtime (Windows 10 and 11 have it).

Risk-4b check (a person signs in to the gateway and to Orbit; prints no secret and signs out
at the end). In a Command Prompt in this folder:

    start /wait Orbit.exe --check-gateway https://orbit.wellrox.com
