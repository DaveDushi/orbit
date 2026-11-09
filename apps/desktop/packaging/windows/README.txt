Orbit for Windows

Start Orbit.exe. The app is not signed yet: on the first start Windows SmartScreen says
"Windows protected your PC". Click "More info", then "Run anyway".

Needs the Microsoft Edge WebView2 runtime for the server's sign-in window (Windows 10 and 11
have it).

Risk-4b check (a person signs in to the gateway and to Orbit; prints no secret and signs out
at the end). In a Command Prompt in this folder:

    start /wait Orbit.exe --check-gateway https://orbit.wellrox.com
