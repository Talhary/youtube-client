//! Read a video's comments from YouTube's page API (signed out).
//!
//!     cargo run -p streampipe-core --example comments -- <video id>
#[tokio::main]
async fn main() {
    let id = std::env::args().nth(1).unwrap_or_else(|| "jNQXAC9IVRw".into());
    let http = streampipe_core::service::http_client();
    match streampipe_core::webfeed::comments(&http, None, &id).await {
        Ok((items, more)) => {
            println!("{} comments, more: {}", items.len(), more.is_some());
            for c in items.iter().take(3) {
                println!("  {:?} | {} | likes {:?} | replies {} | {:.50}", c.author, c.published, c.likes, c.replies, c.text.replace('\n', " "));
            }
            if let Some(m) = more {
                match streampipe_core::webfeed::comments_more(&http, None, &m).await {
                    Ok((next, more2)) => println!("next page: {} comments, more: {}", next.len(), more2.is_some()),
                    Err(e) => println!("next page FAILED: {e}"),
                }
            }
        }
        Err(e) => println!("FAILED: {e}"),
    }
}
