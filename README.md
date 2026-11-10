# GUC Station

A native Windows app for GUC students, written in Rust with [Iced](https://iced.rs). It signs in to the
student portal, CMS and university mail with one GUC account and talks only to `*.guc.edu.eg`, plus GitHub when you check for updates.

## Run

```bash
cargo run
```

Needs the Rust toolchain and the Visual Studio C++ build tools. `cargo run --release` builds the small,
fast exe, but its last step takes a few minutes because the whole program is optimised at once.

## How it talks to GUC

- `apps.guc.edu.eg` and `cms.guc.edu.eg` are IIS sites that only accept NTLM. `src/ntlm.rs` implements
  NTLMv2 and `src/client.rs` keeps each host on one keep-alive connection, because NTLM authenticates
  the connection rather than the request.
- The servers only offer TLS 1.2 CBC ciphers, which rustls refuses, so reqwest uses the OS TLS stack.
- Mail goes through Exchange Web Services (`/EWS/Exchange.asmx`), which takes the same NTLM sign-in.
  OWA itself uses a forms login. Sending saves a draft, adds any attachments, then sends it, because
  EWS only adds attachments to a saved item.
- Portal pages are ASP.NET WebForms. `src/page.rs` pulls out tables, links, dropdowns and form
  state, and replays postbacks (`__VIEWSTATE`, `__EVENTTARGET`) when you pick an option.
- Section URLs come from links on the portal home page, with fallbacks in `src/portal.rs`.

## Writing parsers against real pages

Page layouts are only guessed until someone looks at real markup. To save every section's raw HTML to
`fixtures/` (git-ignored, because it holds personal data):

```powershell
$env:GUC_USER = "firstname.lastname"; $env:GUC_PASS = "..."; cargo run --bin probe
```

Without those variables, the probe uses the sign-in the app saved with "Keep me signed in".
`cargo run --bin probe -- drafts` checks mail sending by building drafts addressed to yourself and
deleting them again; it never sends. In the app, Settings → Troubleshooting saves the pages too.

## Layout

| Path | What |
|------|------|
| `src/ntlm.rs`, `src/client.rs` | NTLM, cookies, redirects |
| `src/page.rs` | WebForms page parsing and postbacks |
| `src/portal.rs` | Section discovery, GPA and letter grades |
| `src/academic.rs` | Schedule, exams, grades, attendance, transcript and CMS parsers |
| `src/mail.rs` | EWS reading, searching, attachments and sending |
| `src/creds.rs` | Saved sign-in in Windows Credential Manager |
| `src/main.rs`, `src/views.rs`, `src/screens.rs` | App state and screens |
| `src/theme.rs`, `src/settings.rs`, `src/tools.rs`, `src/equal_row.rs` | Themes, preferences, downloads, calculators, layout |
| `src/pdf.rs` | In-app PDF viewing with Windows' built-in PDF engine |
| `src/update.rs` | Checking GitHub releases and updating in place |
| `src/bin/probe.rs` | Saves real pages for writing parsers |
| `assets/` | IBM Plex Sans (with its licence) and the app's icons and pixel art |
| `packaging/windows/README.txt` | The read-me that goes in the download zip |
