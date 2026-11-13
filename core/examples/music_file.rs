//! Read saved YouTube Music answers (a JSON object of name -> answer) with the app's reader:
//!     cargo run -p streampipe-core --example music_file -- saved.json
fn main() {
    let path = std::env::args().nth(1).expect("file");
    let all: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    for (name, data) in all.as_object().unwrap() {
        if name == "next" {
            let (items, more) = streampipe_core::music::read_queue(data);
            println!("{name}: queue {} items, more {}", items.len(), more.is_some());
            for i in items.iter().take(2) {
                println!("   {} {:?} {} | {} | {:?}", i.kind, i.id, i.title, i.subtitle, i.duration);
            }
            continue;
        }
        let (page, more) = streampipe_core::music::read_page(data);
        println!("{name}: header {:?}, more {}", page.header.as_ref().map(|h| (&h.title, &h.subscribers, &h.radio)), more.is_some());
        for s in &page.shelves {
            println!("   [{}] {:?} {} items", s.style, s.title, s.items.len());
            for i in s.items.iter().take(1) {
                println!("      {} {} | {} | {} | thumb {}", i.kind, i.id, i.title, i.subtitle, i.thumbnail.is_some());
            }
        }
    }
}
