use chrono::Local;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

pub struct ScreenRecorder {
    child: Arc<Mutex<Option<Child>>>,
    output_dir: PathBuf,
}

impl ScreenRecorder {
    pub fn new() -> Self {
        // Default to saving in .steer/recordings
        let mut output_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        output_dir.push(".steer");
        output_dir.push("recordings");

        // Ensure dir exists
        let _ = std::fs::create_dir_all(&output_dir);

        Self {
            child: Arc::new(Mutex::new(None)),
            output_dir,
        }
    }

    pub fn start(&self) -> String {
        let mut child_guard = self.child.lock().unwrap();

        // If already running, do nothing
        if child_guard.is_some() {
            return "Recording already in progress".to_string();
        }

        let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        let filename = format!("recording_{}.mp4", timestamp);
        let file_path = self.output_dir.join(&filename);
        let path_str = file_path.to_string_lossy().to_string();

        println!("🎥 [Blackbox] Starting recording to: {}", path_str);

        // FFmpeg Command for efficiency:
        // -f avfoundation: macOS capture
        // -r 5: 5 FPS (Ultra low)
        // -i "1": Screen 1 (default)
        // -vf scale=1280:-1: Downscale to 720p width, auto height
        // -c:v libx264: H.264 Codec
        // -preset ultrafast: Low CPU usage
        // -y: Overwrite if exists
        // Log level quiet to avoid spam
        let child = Command::new("ffmpeg")
            .args(&[
                "-f",
                "avfoundation",
                "-r",
                "5",
                "-i",
                "1",
                "-vf",
                "scale=1280:-1",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p",
                "-y",
                &path_str,
            ])
            .stdin(Stdio::piped()) // Needed to send 'q' later if we wanted, but kill is easier
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();

        match child {
            Ok(c) => {
                *child_guard = Some(c);
                format!("Started recording: {}", filename)
            }
            Err(e) => {
                format!("Failed to start recording: {}", e)
            }
        }
    }

    pub fn stop(&self) -> String {
        let mut child_guard = self.child.lock().unwrap();

        if let Some(mut child) = child_guard.take() {
            // Signal to stop.
            // Sending 'q' to stdin is ideal for ffmpeg to finish file properly.
            // But kill() is more reliable if stdin piping is flaky.
            // Let's try kill first for simplicity as -movflags +faststart usually handles abrupt stops or we can use SIGTERM.
            // Actually, simply killing ffmpeg might corrupt the mp4 header.
            // Better to use SIGTERM (Unix) which ffmpeg catches and closes gracefully.

            #[cfg(unix)]
            {
                // Rust std doesn't expose signal sending easily on Child.
                // We'll use the 'kill' command line for now as it's easiest cross-platform-ish on macos.
                let _ = Command::new("kill")
                    .arg("-SIGTERM")
                    .arg(child.id().to_string())
                    .output();
            }

            // Wait for it to exit
            let _ = child.wait();
            println!("🛑 [Blackbox] Recording stopped.");
            "Recording saved.".to_string()
        } else {
            "No recording active.".to_string()
        }
    }

    pub fn cleanup_old_recordings(&self) {
        // Implementation for retention policy (24h)
        let now = std::time::SystemTime::now();
        let one_day = std::time::Duration::from_secs(24 * 60 * 60);

        if let Ok(entries) = std::fs::read_dir(&self.output_dir) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if let Ok(modified) = metadata.modified() {
                        if let Ok(age) = now.duration_since(modified) {
                            if age > one_day {
                                let _ = std::fs::remove_file(entry.path());
                                println!("🗑️ [Blackbox] Deleted old recording: {:?}", entry.path());
                            }
                        }
                    }
                }
            }
        }
    }
}
