# EyeSaver

A minimalist reminder to rest your eyes, for macOS and Windows.

Every 20 minutes EyeSaver asks you to look at something about 20 feet away for 20 seconds —
the [20‑20‑20 rule](https://www.aoa.org/healthy-eyes/caring-for-your-eyes/screen-fatigue)
optometrists recommend for screen work. It lives in the menu bar or the notification area,
shows a live countdown, and gets out of the way: no window, no dock icon, no account, no
network access.

It also tries not to nag you about a break you have already had. If your machine slept,
the screen locked, or you simply walked away from the desk, EyeSaver notices and starts a
fresh interval instead of reminding an empty chair.

```
┌─ macOS menu bar ────────────┐     ┌─ Windows notification area ──┐
│  ⋯   19:42   🔋  Wed 14:03  │     │  ⌃  19  🔊  ᴇɴɢ  14:03       │
└─────────────────────────────┘     └──────────────────────────────┘
   live countdown, as text            minutes, drawn into the icon
```

---

## Install

### macOS 11 or later (Apple silicon and Intel)

1. Download `EyeSaver-<version>-universal.dmg` from the
   [latest release](../../releases/latest).
2. Open it and drag **EyeSaver** to your Applications folder.
3. **The first launch needs one extra step.** The app is not signed with an Apple
   Developer ID, so opening it shows *"Apple could not verify 'EyeSaver' is free of
   malware…"* (worded *"…cannot be opened because the developer cannot be verified"* on
   older macOS). What clears it depends on your macOS version:
   - **macOS Sequoia (15) and later:** double‑click EyeSaver, click **Done** on the
     warning, then go to **System Settings → Privacy & Security**, scroll to the
     **Security** section, and click **Open Anyway** next to EyeSaver. Open it once more
     and a second dialog offers a real **Open Anyway** button.
   - **macOS Ventura/Sonoma and earlier:** right‑click (or Control‑click) **EyeSaver** in
     Applications → **Open** → **Open**.
   - **Fastest, if you're comfortable with Terminal, on any version:**
     `xattr -cr /Applications/EyeSaver.app` removes the quarantine flag outright, no
     dialogs at all.

   macOS remembers the choice either way; every launch after that is a normal double‑click.
4. The first reminder asks permission to show notifications. Allow it, or reminders will be
   silent.

An eye appears in the menu bar with the countdown beside it. There is no Dock icon — that
is deliberate.

### Windows 10 1809 or later (x64 and ARM64)

1. Download the installer for your machine from the
   [latest release](../../releases/latest):
   - `EyeSaver-<version>-x64.msi` — Intel and AMD processors
   - `EyeSaver-<version>-arm64.msi` — ARM processors (Snapdragon, Surface Pro X)
2. Run it. It installs for your user only, into
   `%LocalAppData%\Programs\EyeSaver`, so there is **no administrator prompt**.
3. **SmartScreen will warn you**, because the installer is not code‑signed: click **More
   info** → **Run anyway**.
4. EyeSaver starts by itself when the installer finishes, with its settings window open.
   After that it is in the Start Menu.

An eye appears in the notification area with the minutes remaining drawn into it. Hover for
the exact time; click for the menu.

> Windows attributes a toast notification to an identity registered on a shortcut, so
> reminders from a copy of `eye-saver.exe` that was never installed — one unpacked or
> built by hand — can appear with the wrong name.

### Uninstall

- **macOS** — drag EyeSaver to the Trash, then (if you turned on *Start at login*) delete
  `~/Library/LaunchAgents/com.eyesaver.app.plist`. Settings live in
  `~/Library/Application Support/EyeSaver`.
- **Windows** — Settings → Apps → EyeSaver → Uninstall. This also removes the login‑item
  registry value. Settings live in `%AppData%\EyeSaver`.

### Update

Your settings are kept either way.

- **Windows** — download the newer installer and run it. It replaces the version that is
  already installed, closing EyeSaver first if it is running and starting the new one
  when it is done. There is nothing to uninstall beforehand.
- **macOS** — quit EyeSaver from its menu, drag the new **EyeSaver** from the disk image to
  Applications, choose **Replace**, and open it again.

---

## Using it

Click the icon for the menu:

| Item | What it does |
|---|---|
| *Next break in 19m* | How long until the next reminder (not clickable) |
| **Take a break now** | Start the look‑away break immediately |
| **Reset the timer** | "I've just looked away" — start the interval again |
| **Pause** / **Resume** | Stop counting. A paused EyeSaver uses no CPU at all |
| **Settings…** | The one and only window |
| **Quit EyeSaver** | Stop |

Launching EyeSaver opens its settings window, so that starting it visibly does something.
If it is already running, that is all a second launch does — there is never a second timer.
Only a start at login (`--autostart`) is silent.

### Settings

| Setting | Default | Range |
|---|---|---|
| Remind me every | 20 minutes | 1–180 |
| Look away for | 20 seconds | 5–300 |
| Count time away as a break after | 60 seconds | 15–1800 |
| Menu bar / tray shows | live countdown | countdown · minutes · icon only |
| Start when I log in | off | |
| Play a sound with the reminder | on | |

Settings are a TOML file you can also edit by hand while the app is closed
(`settings.toml`, in the directories listed above). Unknown keys are ignored and
out‑of‑range values are clamped, so an upgrade or a typo cannot stop the app from starting.

### When a break counts as already taken

This is the part worth knowing, because it is why EyeSaver stays quiet when it should.
Counting stops, and a fresh interval begins, whenever your eyes have had at least the break
length away from the screen. Four independent signals feed that decision:

1. **Sleep and wake.** The lid closes, the machine suspends. On waking, counting does not
   restart until the screen is genuinely back: EyeSaver tracks *every* reason it is waiting
   — machine asleep, display off, session locked — and resumes only when the last of them
   clears. That matters because waking a laptop raises those events in a different order on
   each platform, and on each wake.
2. **Clock divergence.** If the wall clock has run further than the monotonic clock, the
   machine was suspended even if no event said so. A backstop for a missed notification.
3. **Idle.** No keyboard or mouse for longer than the threshold and the countdown freezes at
   full, so you get a whole interval from the moment you come back, and nothing fires while
   you are in a meeting.
4. **A tick that arrives impossibly late.** If the process was not scheduled for longer than
   a whole break, the break's window has passed; EyeSaver starts over silently rather than
   reminding you about it.

Short interruptions do *not* reset anything: a five‑second screen blank or a thirty‑second
coffee leaves the countdown exactly where it was.

---

## Battery

EyeSaver is an event loop that sleeps. It asks the operating system to wake it at the next
moment a visible number actually changes, does a few microseconds of work, and goes back to
sleep. Measured on an M4 Pro with the default live countdown: **0.0 % CPU**.

| State | Wake-ups |
|---|---|
| Live `19:59` countdown (default) | once a second |
| "Minutes" display | once a minute, once a second for the last minute |
| "Icon only" display | once a minute |
| During a 20‑second break | once a second |
| Checking whether you are at the keyboard | once every 15 seconds |
| **Paused** | **none — the process blocks indefinitely** |
| **Machine asleep** | **none** |
| Screen locked or display off | once every 2 seconds, to notice it coming back |

The settings window is *created when you open it and destroyed when you close it*, so
nothing is laid out, rasterised or presented while it is shut — and egui is reactive, so an
open‑but‑idle window does not repaint either. Choose "minutes" or "icon only" to drop from
60 wake‑ups a minute to one.

---

## Architecture

Ports and adapters, with a hard line between decisions and the operating system.

```
crates/core/        eye-saver-core      no OS, no I/O, no clock reading, no GUI
  engine.rs           the timer state machine: every rule lives here
  settings.rs         defaults, limits, TOML
  format.rs           the strings, and when each one next changes
  screen.rs           display/lock readings -> suspend and resume events
  sleep_detect.rs     the two clocks disagreeing -> "it was asleep"
  icon.rs             the artwork, as arithmetic (no image files)
  runner.rs           carrying out effects, through the ports
  ports.rs            the traits the outside world implements
  testing.rs          recording fakes for every port

crates/platform/    eye-saver-platform  adapters
  store.rs            the settings file (atomic writes)
  instance.rs         the lock that keeps one EyeSaver running
  logger.rs           the log file
  autostart.rs        the macOS login item
  paths.rs, clock.rs
  os/                 the irreducible part: tray, notifications, AppKit, Win32

crates/app/         eye-saver           the winit event loop and the settings window
```

The core never acts. It is handed an `Event` and answers with a list of `Effect`s —
including `WakeAt`, which is how the battery behaviour above becomes something a test can
assert rather than something you observe with a power meter. The binary's whole job is to
read the clock, pump events in, and carry effects out.

A deliberate platform difference lives in `TrayCaps`: macOS can show text beside a menu bar
icon and Windows cannot, so on Windows the countdown is drawn *into* the icon as digits and
the refresh schedule is coarser to match. Both behaviours are tested on any host.

### Dependencies

No Electron, no webview, no plugin framework. Beyond `serde`/`toml` and `log`: `tray-icon`
and `muda` for the menu bar item, `notify-rust` for notifications, `winit` for the event
loop, and `egui`/`egui_glow`/`glutin` for the one settings window. The macOS adapters call
CoreGraphics directly rather than pulling in an Objective‑C bridge; the Windows adapters use
the `windows` crate with nine features enabled.

---

## Building from source

Needs [Rust](https://rustup.rs) 1.90 or later. `make help` lists everything.

```sh
make build          # debug build
make test           # 217 tests
make check          # formatting, clippy, tests and the coverage gate, as CI runs them
make run            # run from the source tree
```

### Installers

```sh
# macOS: universal (aarch64 + x86_64) .app and a disk image, in dist/
make app
make dmg

# Windows: run on Windows, once per architecture. Needs `dotnet tool install --global wix --version 4.0.6`.
./packaging/windows/build-msi.ps1 -Arch x64
./packaging/windows/build-msi.ps1 -Arch arm64
```

Both scripts produce unsigned output by default and switch to real signing from environment
variables, with no code change:

| Variable | Effect |
|---|---|
| `APPLE_SIGNING_IDENTITY` | Sign the `.app` with a Developer ID instead of ad‑hoc |
| `AC_APPLE_ID`, `AC_PASSWORD`, `AC_TEAM_ID` | Also notarise and staple the disk image |
| `WINDOWS_CERT`, `WINDOWS_CERT_PASSWORD` | Authenticode‑sign the executable and the MSI |

### Processor support

| Platform | Target | Built by |
|---|---|---|
| macOS, Apple silicon | `aarch64-apple-darwin` | one universal binary |
| macOS, Intel | `x86_64-apple-darwin` | ″ |
| Windows, Intel/AMD | `x86_64-pc-windows-msvc` | `-Arch x64` |
| Windows, ARM | `aarch64-pc-windows-msvc` | `-Arch arm64` |

CI builds and tests on macOS arm64, macOS x86_64, Windows x86_64 and Windows arm64, and
type‑checks each platform's adapters from the other.

### Icons

There are no image files in this repository. `make icons` renders the `.icns`, `.ico` and
`.png` the installers embed from `eye_saver_core::icon` — the same code that draws the tray
at runtime — writing the container formats directly so they can be regenerated on any host.

---

## Releasing

`main` is protected: every change, including this repo's own config, goes through a pull
request — there is no direct push, not even for the owner. Version numbers are never
hand-edited; they are computed automatically from commit messages by
[release-plz](https://release-plz.dev), configured in `release-plz.toml`.

### Commit messages

Every commit message (or, if you squash-merge, the squash commit message — only what
actually lands on `main` is read) should follow
[Conventional Commits](https://www.conventionalcommits.org/):

| Prefix | Meaning |
|---|---|
| `fix: …` | A bug fix |
| `feat: …` | A new feature |
| `feat!: …`, `fix!: …`, or a `BREAKING CHANGE:` footer | A breaking change |
| `chore:`, `docs:`, `style:`, `refactor:`, `test:`, `build:`, `ci:` | Housekeeping — doesn't trigger a release on its own |

**While the version stays below `1.0.0`** (EyeSaver's current state), `feat`, `fix`, and
even breaking changes all just increment the patch number (`0.1.0` → `0.1.1`) — this is
the standard Rust-ecosystem reading of semver's "0.x.y: anything may change" clause, and
it is deliberate: crossing into `1.0.0` is always a decision a person makes on purpose
(`release-plz set-version 1.0.0`), never something inferred from a commit message.
Once past `1.0.0`, the usual mapping applies: `fix` → patch, `feat` → minor, breaking → major.

### What actually happens

```
you merge a PR with a feat:/fix: commit
                │
                ▼
release-plz opens or updates one PR: "chore: release vX.Y.Z"
   (bumps the version + CHANGELOG.md — no code in this PR)
                │
                ▼
          you merge that PR
                │
                ▼
   the vX.Y.Z tag is created automatically
                │
                ▼
   release.yml builds the DMG + both MSIs and attaches
   them to a new GitHub Release
```

So the one deliberate action that ships a release is merging the bot's version-bump PR —
everything before and after that is automatic. Ordinary PRs that don't warrant a release
yet (or that only contain `chore:`/`docs:` commits) just leave that PR as it is.

---

## Testing

```sh
make test            # every test
make coverage        # the gate: >= 95% of lines
make coverage-html   # a browsable report
```

**217 tests. 99.7 % line coverage, 100 % of the core.** The suite is in four parts:

- **Unit tests** over every module: each event against each state, the reason‑set
  semantics, break‑credit boundaries, clock divergence either side of its tolerance, TOML
  with missing and unknown keys, the wake‑up schedule per display mode, the icon's pixels.
- **Property tests** (`crates/core/tests/invariants.rs`) over arbitrary event sequences:
  the countdown never exceeds its phase, counting and the reasons not to count can never
  disagree, exactly one wake‑up is armed whenever something is being counted and none when
  it is not, and nothing is ever notified while the screen is away.
- **End‑to‑end timelines** (`crates/core/tests/timeline.rs`) that drive the whole
  application against fake hardware, in a loop that is a faithful copy of the real one:
  an undisturbed morning, sleeping through a reminder, a wake where the display returns
  before the lock, an hour‑long meeting, an NTP correction, a tray that refuses to update,
  and a full day of mixed interruptions.
- **Real file and lock behaviour** in `eye-saver-platform`: atomic saves, corrupt configs,
  rotation, and one process genuinely failing to take a lock another holds.

### What the gate excludes, and why

`crates/app/`, `crates/platform/src/os/` and `xtask/` are outside the coverage denominator.
They are the code that cannot run without a logged‑in desktop session — creating a menu bar
item, posting to Notification Center, asking AppKit whether the display is asleep, reading
the Windows registry, running an event loop — so "covering" them would mean mocking the
operating system and measuring the mock.

Rather than inflate the number, the architecture keeps that layer as thin as it can be:
every adapter does one call and converts the result, and all four of them together are
smaller than the state machine they feed. Everything those adapters *inform* is tested. The
figure above is honest about what it measures.

Verified by hand on macOS 26 (Apple silicon), since no test can: the menu bar countdown
with no Dock icon, a second launch opening the settings window instead of a second timer,
notifications from the bundled app, and 0.0 % CPU while counting.

---

## License

MIT. See [LICENSE](LICENSE).
