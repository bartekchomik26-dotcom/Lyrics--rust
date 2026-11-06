//! The system's own media session, which every player that shows in the OS
//! media controls reports to: Spotify, SoundCloud and YouTube Music in a
//! browser, Apple Music, Tidal, Deezer, foobar2000…
//!
//! * Windows: Global System Media Transport Controls, read by a PowerShell
//!   helper (the original mod's script, which also names the app).
//! * Linux: MPRIS through `playerctl`.
//! * macOS: Spotify and Music through `osascript`.

use crate::{Control, Engine, Origin, Sample, Source};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const SEP: char = '\u{1f}';

static CHILD: Mutex<Option<Child>> = Mutex::new(None);

/// Ends the helper process.
pub fn kill() {
    if let Some(mut c) = CHILD.lock().unwrap_or_else(|p| p.into_inner()).take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

fn wanted(engine: &Engine) -> bool {
    engine.with_config(|c, _| c.system_media && matches!(c.source, Source::Auto | Source::System))
}

/// A friendlier name for a media session's app id.
fn app_name(id: &str) -> String {
    let lower = id.to_lowercase();
    let known = [
        ("spotify", "Spotify"),
        ("soundcloud", "SoundCloud"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("opera", "Opera"),
        ("brave", "Brave"),
        ("applemusic", "Apple Music"),
        ("music", "Music"),
        ("tidal", "TIDAL"),
        ("deezer", "Deezer"),
        ("foobar", "foobar2000"),
        ("musicbee", "MusicBee"),
        ("vlc", "VLC"),
        ("youtube", "YouTube"),
    ];
    known
        .iter()
        .find(|(k, _)| lower.contains(k))
        .map(|(_, v)| (*v).to_owned())
        .unwrap_or_else(|| {
            let base = id.rsplit(['\\', '/', '!']).next().unwrap_or(id);
            base.trim_end_matches(".exe").to_owned()
        })
}

/// One `LP␟status␟pos␟dur␟artist␟title␟album␟app` line (art URL on Linux in place of app, then app).
fn parse(line: &str, at: Instant) -> Option<Option<Sample>> {
    let f: Vec<&str> = line.trim_end_matches(['\r', '\n']).split(SEP).collect();
    if f.first() != Some(&"LP") || f.len() < 7 {
        return None;
    }
    let status = f[1];
    if status == "NONE" || status == "Closed" || status == "Stopped" || f[5].trim().is_empty() {
        return Some(None);
    }
    let num = |s: &str| s.trim().parse::<f64>().unwrap_or(0.0).max(0.0) as u64;
    Some(Some(Sample {
        title: f[5].trim().to_owned(),
        artist: f[4].trim().to_owned(),
        album: f[6].trim().to_owned(),
        position_ms: num(f[2]),
        duration_ms: num(f[3]),
        playing: status.eq_ignore_ascii_case("playing"),
        art_url: f.get(8).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()),
        art_bytes: None,
        app: f.get(7).map(|a| app_name(a)).unwrap_or_default(),
        lyrics: None,
        at,
    }))
}

pub fn run(engine: &'static Engine) {
    let mut backoff = Duration::from_secs(1);
    while !engine.stopped() {
        if !wanted(engine) {
            kill();
            engine.set_system_running(false);
            engine.report(Origin::System, None);
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        let ok = if cfg!(windows) {
            windows(engine)
        } else if cfg!(target_os = "macos") {
            mac(engine)
        } else {
            linux(engine)
        };
        engine.set_system_running(ok);
        if !ok {
            engine.report(Origin::System, None);
            std::thread::sleep(backoff);
            backoff = (backoff * 2).min(Duration::from_secs(60));
        } else {
            backoff = Duration::from_secs(1);
        }
    }
    kill();
}

#[cfg(windows)]
pub(crate) fn hidden(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    // CREATE_NO_WINDOW: no console flashes up over the game.
    cmd.creation_flags(0x0800_0000);
}
#[cfg(not(windows))]
pub(crate) fn hidden(_: &mut Command) {}

/// Runs the PowerShell helper until it ends; false when it couldn't start
/// or ended at once.
fn windows(engine: &'static Engine) -> bool {
    let path = engine.data_dir().join("media-session.ps1");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(WINDOWS_SCRIPT) {
        let _ = std::fs::write(&path, WINDOWS_SCRIPT);
    }
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&path)
        .arg("-ParentPid")
        .arg(std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hidden(&mut cmd);
    let Ok(mut child) = cmd.spawn() else {
        return false;
    };
    let Some(out) = child.stdout.take() else {
        let _ = child.kill();
        return false;
    };
    *CHILD.lock().unwrap_or_else(|p| p.into_inner()) = Some(child);
    let started = Instant::now();
    let mut lines = 0u32;
    for line in BufReader::new(out).lines() {
        let Ok(line) = line else { break };
        if engine.stopped() || !wanted(engine) {
            break;
        }
        if let Some(sample) = parse(&line, Instant::now()) {
            lines += 1;
            engine.set_system_running(true);
            engine.report(Origin::System, sample);
        }
    }
    kill();
    lines > 0 || started.elapsed() > Duration::from_secs(10)
}

fn output(cmd: &mut Command) -> Option<String> {
    cmd.stdin(Stdio::null()).stderr(Stdio::null());
    hidden(cmd);
    let out = cmd.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Polls `playerctl` while it answers.
fn linux(engine: &'static Engine) -> bool {
    let format = ["{{status}}", "{{position}}", "{{mpris:length}}", "{{artist}}", "{{title}}", "{{album}}", "{{playerName}}", "{{mpris:artUrl}}"]
        .join(&SEP.to_string());
    let mut answered = false;
    while !engine.stopped() && wanted(engine) {
        let at = Instant::now();
        match output(Command::new("playerctl").args(["metadata", "--format", &format])) {
            Some(text) => {
                answered = true;
                // Microseconds to milliseconds.
                let mut f: Vec<String> = text.trim_end().split(SEP).map(str::to_owned).collect();
                if f.len() >= 8 {
                    let us = |s: &str| (s.trim().parse::<f64>().unwrap_or(0.0) / 1000.0).to_string();
                    f[1] = us(&f[1]);
                    f[2] = us(&f[2]);
                    let line = format!("LP{SEP}{}", f.join(&SEP.to_string()));
                    if let Some(s) = parse(&line, at) {
                        engine.report(Origin::System, s);
                    }
                } else {
                    engine.report(Origin::System, None);
                }
            }
            None => {
                // No player, or no playerctl at all.
                if Command::new("playerctl").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
                    return false;
                }
                answered = true;
                engine.report(Origin::System, None);
            }
        }
        std::thread::sleep(Duration::from_millis(750));
    }
    answered
}

/// Polls Spotify, then Music, through AppleScript (only apps already
/// running are asked, so nothing launches).
fn mac(engine: &'static Engine) -> bool {
    while !engine.stopped() && wanted(engine) {
        let at = Instant::now();
        let mut found = None;
        for (app, script) in [("Spotify", MAC_SPOTIFY), ("Music", MAC_MUSIC)] {
            let running = Command::new("pgrep")
                .args(["-x", app])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if !running {
                continue;
            }
            if let Some(text) = output(Command::new("osascript").args(["-e", script]))
                && let Some(Some(s)) = parse(text.trim(), at)
            {
                found = Some(s);
                break;
            }
        }
        engine.report(Origin::System, found);
        std::thread::sleep(Duration::from_millis(1000));
    }
    true
}

/// A media control for the system's session.
pub fn control(c: Control) {
    if cfg!(windows) {
        let action = match c {
            Control::PlayPause => "toggle",
            Control::Next => "next",
            Control::Previous => "prev",
        };
        // `-Command` joins what follows into the script, so the action goes in it.
        let script = format!("$Action = '{action}'\n{WINDOWS_CONTROL}");
        let mut cmd = Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command"])
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hidden(&mut cmd);
        let _ = cmd.status();
    } else if cfg!(target_os = "macos") {
        let verb = match c {
            Control::PlayPause => "playpause",
            Control::Next => "next track",
            Control::Previous => "previous track",
        };
        for app in ["Spotify", "Music"] {
            let running = Command::new("pgrep").args(["-x", app]).status().is_ok_and(|s| s.success());
            if running {
                let _ = Command::new("osascript")
                    .args(["-e", &format!("tell application \"{app}\" to {verb}")])
                    .status();
                break;
            }
        }
    } else {
        let verb = match c {
            Control::PlayPause => "play-pause",
            Control::Next => "next",
            Control::Previous => "previous",
        };
        let _ = Command::new("playerctl")
            .arg(verb)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

const MAC_SPOTIFY: &str = r#"set sep to (ASCII character 31)
tell application "Spotify"
	if player state is stopped then return ""
	set t to current track
	return "LP" & sep & (player state as string) & sep & ((player position * 1000) as integer as string) & sep & ((duration of t) as string) & sep & (artist of t) & sep & (name of t) & sep & (album of t) & sep & "Spotify" & sep & (artwork url of t)
end tell"#;

const MAC_MUSIC: &str = r#"set sep to (ASCII character 31)
tell application "Music"
	if player state is stopped then return ""
	set t to current track
	return "LP" & sep & (player state as string) & sep & ((player position * 1000) as integer as string) & sep & (((duration of t) * 1000) as integer as string) & sep & (artist of t) & sep & (name of t) & sep & (album of t) & sep & "Music"
end tell"#;

/// The original LyricsPlus GSMTC reader, with the app's id added, a field
/// separator no title holds, and an exit when the game is gone.
const WINDOWS_SCRIPT: &str = r#"param([int]$ParentPid = 0)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$asTaskGeneric = ([System.WindowsRuntimeSystemExtensions].GetMethods() | ? { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
function Await-WinRT($winRtAsync, $returnType) {
    if (-not $winRtAsync) { return $null }
    try {
        $asTask = $asTaskGeneric.MakeGenericMethod($returnType)
        $task = $asTask.Invoke($null, @($winRtAsync))
        if ($task.Wait(1200)) { return $task.Result }
    } catch {}
    return $null
}
[Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager, Windows.Media, ContentType = WindowsRuntime] | Out-Null
$manager = Await-WinRT ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager]::RequestAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager])
if (-not $manager) { exit 1 }
$sep = [char]31
$skip = 'TikTok|GDI\+|Default IME|MSCTFIME'
function Get-ActiveMediaSession($mgr) {
    try {
        $sessions = $mgr.GetSessions()
        foreach ($s in $sessions) {
            try {
                $pInfo = $s.GetPlaybackInfo()
                if ($pInfo -and $pInfo.PlaybackStatus -eq [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionPlaybackStatus]::Playing) {
                    $m = Await-WinRT ($s.TryGetMediaPropertiesAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties])
                    if ($m -and $m.Title -and $m.Title -notmatch $skip) { return @($s, $pInfo, $m) }
                }
            } catch {}
        }
        foreach ($s in $sessions) {
            try {
                $pInfo = $s.GetPlaybackInfo()
                if ($pInfo -and $pInfo.PlaybackStatus -ne [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionPlaybackStatus]::Closed) {
                    $m = Await-WinRT ($s.TryGetMediaPropertiesAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties])
                    if ($m -and $m.Title -and $m.Artist -and $m.Title -notmatch $skip) { return @($s, $pInfo, $m) }
                }
            } catch {}
        }
        $curr = $mgr.GetCurrentSession()
        if ($curr) {
            $pInfo = $curr.GetPlaybackInfo()
            $m = Await-WinRT ($curr.TryGetMediaPropertiesAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties])
            if ($m -and $m.Title -and $m.Title -notmatch $skip) { return @($curr, $pInfo, $m) }
        }
    } catch {}
    return $null
}
function Clean($v) { if ($v) { ($v -replace "[\r\n\x1f]+", ' ').Trim() } else { '' } }
$n = 0
$cached = $null
$lastEnd = -1
$lastStatus = ''
while ($true) {
    $n++
    if ($ParentPid -gt 0 -and ($n % 20) -eq 0 -and -not (Get-Process -Id $ParentPid -ErrorAction SilentlyContinue)) { exit 0 }
    try {
        # The full scan (every session's title, which is slow) about once a
        # second, or when the shown one changes state or length; otherwise
        # only the shown session's position.
        $active = $null
        if ($cached -and ($n % 6) -ne 0) {
            try {
                $s0 = $cached[0]
                $i0 = $s0.GetPlaybackInfo()
                $t0 = $s0.GetTimelineProperties()
                if ($i0 -and $t0 -and $i0.PlaybackStatus.ToString() -eq $lastStatus -and [long]$t0.EndTime.TotalMilliseconds -eq $lastEnd) {
                    $active = @($s0, $i0, $cached[2])
                }
            } catch { $active = $null }
        }
        if (-not $active) {
            $active = Get-ActiveMediaSession $manager
            $cached = $active
        }
        if ($active) {
            $session = $active[0]; $info = $active[1]; $media = $active[2]
            $tl = $session.GetTimelineProperties()
            $status = if ($info) { $info.PlaybackStatus.ToString() } else { 'Closed' }
            $pos = if ($tl) {
                if ($status -eq 'Playing' -and $tl.LastUpdatedTime) {
                    $elapsed = ([DateTimeOffset]::UtcNow - $tl.LastUpdatedTime).TotalMilliseconds
                    $rate = if ($info.PlaybackRate) { [double]$info.PlaybackRate } else { 1.0 }
                    [long]($tl.Position.TotalMilliseconds + ($elapsed * $rate))
                } else { [long]$tl.Position.TotalMilliseconds }
            } else { 0 }
            $dur = if ($tl) { [long]$tl.EndTime.TotalMilliseconds } else { 0 }
            $app = Clean $session.SourceAppUserModelId
            $line = 'LP' + $sep + $status + $sep + $pos + $sep + $dur + $sep + (Clean $media.Artist) + $sep + (Clean $media.Title) + $sep + (Clean $media.AlbumTitle) + $sep + $app
            [Console]::Out.WriteLine($line)
            $lastStatus = $status
            $lastEnd = $dur
        } else {
            [Console]::Out.WriteLine('LP' + $sep + 'NONE' + $sep + '0' + $sep + '0' + $sep + $sep + $sep + $sep)
        }
        [Console]::Out.Flush()
    } catch {}
    Start-Sleep -Milliseconds 200
}
"#;

/// Play/pause, next or previous on the current media session; the media
/// keys when there is none.
const WINDOWS_CONTROL: &str = r#"Add-Type -AssemblyName System.Runtime.WindowsRuntime
$g = ([System.WindowsRuntimeSystemExtensions].GetMethods() | ? { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
function Await($op, $type) { try { $t = $g.MakeGenericMethod($type).Invoke($null, @($op)); if ($t.Wait(2000)) { return $t.Result } } catch {}; return $null }
[Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager, Windows.Media, ContentType = WindowsRuntime] | Out-Null
$m = Await ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager]::RequestAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager])
$s = if ($m) { $m.GetCurrentSession() } else { $null }
if ($s) {
    switch ($Action) {
        'next' { Await ($s.TrySkipNextAsync()) ([bool]) | Out-Null }
        'prev' { Await ($s.TrySkipPreviousAsync()) ([bool]) | Out-Null }
        default { Await ($s.TryTogglePlayPauseAsync()) ([bool]) | Out-Null }
    }
} else {
    $k = switch ($Action) { 'next' { 176 } 'prev' { 177 } default { 179 } }
    (New-Object -ComObject WScript.Shell).SendKeys([string][char]$k)
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_helper_lines() {
        let line = "LP\u{1f}Playing\u{1f}61000\u{1f}200000\u{1f}Band\u{1f}Song\u{1f}Album\u{1f}Spotify.exe";
        let s = parse(line, Instant::now()).unwrap().unwrap();
        assert_eq!((s.title.as_str(), s.artist.as_str(), s.app.as_str()), ("Song", "Band", "Spotify"));
        assert!(s.playing);
        assert_eq!(s.position_ms, 61000);
        assert!(parse("LP\u{1f}NONE\u{1f}0\u{1f}0\u{1f}\u{1f}\u{1f}\u{1f}", Instant::now()).unwrap().is_none());
        assert!(parse("garbage", Instant::now()).is_none());
        assert_eq!(app_name("308046B0AF4A39CB"), "308046B0AF4A39CB");
        assert_eq!(app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "Music");
    }
}
