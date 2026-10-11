//! Now playing and transport control through the Windows media session API (SMTC).
//!
//! Spotify, browsers, and most players publish their current track there, and accept
//! play, pause, and skip commands from it. This needs no account or API key. On other
//! systems this module does nothing.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum PlayState {
    #[default]
    Unknown,
    Playing,
    Paused,
    Stopped,
}

#[derive(Clone, Default, Debug)]
pub struct NowPlaying {
    pub app: String,
    pub artist: String,
    pub title: String,
    pub state: PlayState,
}

#[derive(Clone, Copy, Debug)]
pub enum Command {
    Prev,
    Play,
    PauseToggle,
    Stop,
    Next,
}

impl Command {
    pub fn label(self) -> &'static str {
        match self {
            Command::Prev => "PREVIOUS",
            Command::Play => "PLAY",
            Command::PauseToggle => "PAUSE",
            Command::Stop => "STOP",
            Command::Next => "NEXT",
        }
    }
}

pub struct Media {
    now: Arc<Mutex<Option<NowPlaying>>>,
    tx: Option<Sender<Command>>,
}

impl Media {
    pub fn start() -> Self {
        let now = Arc::new(Mutex::new(None));
        let tx = platform::spawn(now.clone());
        Media { now, tx }
    }

    pub fn now(&self) -> Option<NowPlaying> {
        self.now.lock().ok().and_then(|n| n.clone())
    }

    /// Returns false when no media session API is available on this system.
    pub fn send(&self, cmd: Command) -> bool {
        self.tx.as_ref().is_some_and(|tx| tx.send(cmd).is_ok())
    }
}

/// Turns an app id such as `Spotify.exe` or `Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic`
/// into a short name.
pub fn short_app_name(id: &str) -> String {
    // Firefox registers under a hash of its install folder. This is the hash for the
    // default folder; other install folders show their raw hash.
    if id == "308046B0AF4A39CB" {
        return "Firefox".into();
    }
    let base = id.rsplit(['\\', '/']).next().unwrap_or(id);
    let base = base.split('!').next().unwrap_or(base);
    let base = base.split('_').next().unwrap_or(base);
    let base = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
    let base = base.rsplit('.').next().unwrap_or(base);
    base.to_string()
}

#[cfg(windows)]
mod platform {
    use super::{short_app_name, Command, NowPlaying, PlayState};
    use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };

    pub fn spawn(now: Arc<Mutex<Option<NowPlaying>>>) -> Option<Sender<Command>> {
        let (tx, rx) = channel::<Command>();
        std::thread::Builder::new()
            .name("media-session".into())
            .spawn(move || {
                let Ok(manager) = Manager::RequestAsync().and_then(|op| op.join()) else {
                    return;
                };
                loop {
                    match rx.recv_timeout(Duration::from_millis(700)) {
                        Ok(cmd) => {
                            if let Ok(session) = manager.GetCurrentSession() {
                                run(&session, cmd);
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                    let info = manager.GetCurrentSession().ok().and_then(|s| read(&s));
                    if let Ok(mut slot) = now.lock() {
                        *slot = info;
                    }
                }
            })
            .ok()?;
        Some(tx)
    }

    fn read(session: &Session) -> Option<NowPlaying> {
        let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
        let state = match session.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()) {
            Ok(Status::Playing) => PlayState::Playing,
            Ok(Status::Paused) => PlayState::Paused,
            Ok(Status::Stopped) | Ok(Status::Closed) => PlayState::Stopped,
            _ => PlayState::Unknown,
        };
        Some(NowPlaying {
            app: session.SourceAppUserModelId().map(|h| short_app_name(&h.to_string())).unwrap_or_default(),
            artist: props.Artist().map(|h| h.to_string()).unwrap_or_default(),
            title: props.Title().map(|h| h.to_string()).unwrap_or_default(),
            state,
        })
    }

    fn run(session: &Session, cmd: Command) {
        // Each Try*Async call resolves to true when the app accepted the command.
        macro_rules! accepted {
            ($op:expr) => {
                $op.and_then(|op| op.join()).unwrap_or(false)
            };
        }
        let _ = match cmd {
            Command::Prev => accepted!(session.TrySkipPreviousAsync()),
            Command::Play => accepted!(session.TryPlayAsync()),
            Command::PauseToggle => accepted!(session.TryTogglePlayPauseAsync()),
            // Some apps do not accept Stop, so fall back to Pause.
            Command::Stop => accepted!(session.TryStopAsync()) || accepted!(session.TryPauseAsync()),
            Command::Next => accepted!(session.TrySkipNextAsync()),
        };
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{Command, NowPlaying};
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex};

    pub fn spawn(_now: Arc<Mutex<Option<NowPlaying>>>) -> Option<Sender<Command>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::short_app_name;

    #[test]
    fn app_names_are_short() {
        assert_eq!(short_app_name("Spotify.exe"), "Spotify");
        assert_eq!(short_app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "ZuneMusic");
        assert_eq!(short_app_name("Chrome"), "Chrome");
        assert_eq!(short_app_name(r"C:\Program Files\foobar2000\foobar2000.exe"), "foobar2000");
        assert_eq!(short_app_name("308046B0AF4A39CB"), "Firefox");
    }
}
