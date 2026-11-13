//! Print the first raw entries yt-dlp returns for the account feed pages (session copy).
use streampipe_core::config::Paths;
use streampipe_core::secure::EncryptedFile;

fn main() {
    let real = Paths::desktop().unwrap();
    let key = streampipe_core::secure::desktop_key(&real.data);
    let session = EncryptedFile::new(real.data.join("session.bin"), &key).read().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&session).unwrap();
    let header = json["auth_cookie"]["cookie"].as_str().unwrap();
    let mut txt = String::from("# Netscape HTTP Cookie File\n");
    for p in header.split(';') {
        if let Some((k, v)) = p.trim().split_once('=') {
            txt.push_str(&format!(".youtube.com\tTRUE\t/\tTRUE\t2000000000\t{k}\t{v}\n"));
        }
    }
    let ck = std::env::temp_dir().join("sp-flatraw-cookies.txt");
    std::fs::write(&ck, txt).unwrap();
    for url in ["https://www.youtube.com/feed/subscriptions", "https://www.youtube.com/feed/channels"] {
        let out = std::process::Command::new("D:/yt-client/target/debug/yt-dlp.exe")
            .args(["-J", "--flat-playlist", "--playlist-end", "3", "--no-warnings", "--ignore-config", "--cookies"])
            .arg(&ck).arg(url).output().unwrap();
        println!("== {url} exit={}", out.status);
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
        println!("top-level keys: {:?}", v.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()));
        for e in v["entries"].as_array().into_iter().flatten().take(3) {
            let mut e = e.clone();
            if let Some(o) = e.as_object_mut() { o.remove("thumbnails"); o.remove("description"); }
            println!("{}", serde_json::to_string(&e).unwrap().chars().take(600).collect::<String>());
        }
        if !out.status.success() { println!("{}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("")); }
    }
    let _ = std::fs::remove_file(&ck);
}
