Orbit for Windows

Orbit shows your team's Orbit in its own window, with a tray icon, notifications for new
messages and mentions, and an unread badge on the taskbar button.

Start Orbit.exe. The app is not signed yet: on the first start Windows SmartScreen says
"Windows protected your PC". Click "More info", then "Run anyway".

The first time, enter your Orbit server's address (for example orbit.wellrox.com), then sign in
as in the browser. Closing the window keeps Orbit in the tray (next to the clock); right-click
the tray icon for "Start with Windows" (off until you turn it on), "Change server..." and
"Quit Orbit". Downloads go to your Downloads folder.

Needs the Microsoft Edge WebView2 runtime (Windows 10 and 11 have it).

Risk-4b check (a person signs in to the gateway and to Orbit; prints no secret and signs out
at the end). In a Command Prompt in this folder:

    start /wait Orbit.exe --check-gateway https://orbit.wellrox.com
