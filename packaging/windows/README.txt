GUC Station 0.4.0
A desktop app for the GUC student portal, CMS and university mail.
This is an unofficial app, not made or endorsed by the German University in Cairo.

TO RUN
1. Extract this whole zip to a folder (don't run it from inside the zip).
2. Double-click "GUC Station.exe". There's nothing to install.
3. Sign in with your GUC username and password.

Needs Windows 10 or 11 (64-bit) and an internet connection.

"WINDOWS PROTECTED YOUR PC"
The app isn't digitally signed, so Windows may show this the first time.
Click "More info", then "Run anyway". Only do this if you got the zip from
someone you trust.

RUNNING IN THE BACKGROUND
- GUC Station starts with Windows and waits in the notification area (by the
  clock). Closing the window keeps it running there; right-click the tray
  icon and choose "Quit" to close it fully.
- While it runs and you are signed in, it checks GUC every 5 minutes and shows
  a Windows notification for new mail or portal notifications.
- About once a day it also asks GitHub whether a newer version is out, and
  shows a notification if one is. Nothing is installed until you say yes.
- Turn any of these off in Settings: "Start with Windows", "Notify me about
  new mail and portal notifications" and "Check for updates automatically".

YOUR DATA
- The app only talks to apps.guc.edu.eg, cms.guc.edu.eg and mail.guc.edu.eg.
  Checking for updates also contacts GitHub: about once a day while automatic
  checks are on, and whenever you press "Check for updates" in Settings.
- If you tick "Keep me signed in", your password is saved in Windows Credential
  Manager on this PC. Signing out in Settings removes it.
- Settings, the list of mail and notifications already announced, and any
  pages you save for troubleshooting are in
  %LOCALAPPDATA%\GUC Station
- After you first sign in, the app asks where to put its GUC Station downloads
  folder (Downloads\GUC Station by default). You can change it in Settings.

The app is built around how GUC's pages work today, so a change on GUC's side
can break a screen. Check your institution's rules on third-party apps before
using it with your account.

CREDITS
IBM Plex Sans font: see LICENSE-IBM-Plex.txt
