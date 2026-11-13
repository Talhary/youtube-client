use std::path::Path;

use streampipe_ui::{FileCallback, FileDialog, FormFactor, Platform};

pub struct Desktop {
    /// Where the sign-in window keeps its throwaway browser profile.
    pub signin_profile: std::path::PathBuf,
}

impl Platform for Desktop {
    fn form_factor(&self) -> FormFactor {
        FormFactor::Desktop
    }

    fn open_url(&self, url: &str) {
        let _ = open::that_detached(url);
    }

    fn open_path(&self, path: &Path) {
        let _ = open::that_detached(path);
    }

    fn file_dialog(&self, kind: FileDialog, done: FileCallback) {
        // Native dialogs block, so they run on their own thread.
        std::thread::spawn(move || {
            let picked = match kind {
                FileDialog::OpenCookies => rfd::FileDialog::new()
                    .set_title("Import cookies.txt")
                    .add_filter("Netscape cookies", &["txt"])
                    .pick_file(),
                FileDialog::PickFolder => rfd::FileDialog::new()
                    .set_title("Choose the downloads folder")
                    .pick_folder(),
                FileDialog::SaveDiagnostics => rfd::FileDialog::new()
                    .set_title("Save diagnostics")
                    .set_file_name("streampipe-diagnostics.txt")
                    .save_file(),
            };
            done(picked);
        });
    }

    fn browser_sign_in(&self, done: Box<dyn FnOnce(Result<String, String>) + Send>) {
        let dir = self.signin_profile.clone();
        std::thread::spawn(move || done(crate::signin::sign_in(dir)));
    }

    fn supports_browser_sign_in(&self) -> bool {
        true
    }

    fn os_reduced_motion(&self) -> bool {
        streampipe_ui::os_reduced_motion()
    }
}
