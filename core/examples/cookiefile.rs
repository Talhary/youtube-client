//! Write the app's signed-in YouTube cookies to a temp cookies.txt (for profiling yt-dlp).
fn main() {
    let real = streampipe_core::config::Paths::desktop().unwrap();
    let key = streampipe_core::secure::desktop_key(&real.data);
    let session = streampipe_core::secure::EncryptedFile::new(real.data.join("session.bin"), &key).read().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&session).unwrap();
    let header = json["auth_cookie"]["cookie"].as_str().unwrap();
    let mut txt = String::from("# Netscape HTTP Cookie File\n");
    for p in header.split(';') {
        if let Some((k, v)) = p.trim().split_once('=') {
            txt.push_str(&format!(".youtube.com\tTRUE\t/\tTRUE\t2000000000\t{k}\t{v}\n"));
        }
    }
    let path = std::env::temp_dir().join("sp-profile-cookies.txt");
    std::fs::write(&path, txt).unwrap();
    println!("{}", path.display());
}
