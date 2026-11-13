//! StreamPipe core: YouTube API, auth, downloads, thumbnails and storage, with no UI.
//! Shells talk to it only through [`service::Core`]'s command and event channels.

pub mod adblock;
pub mod api;
pub mod auth;
pub mod config;
pub mod diagnostics;
pub mod download;
pub mod model;
pub mod music;
pub mod mux;
pub mod proxy;
pub mod remote;
pub mod secure;
pub mod service;
pub mod streams;
pub mod thumbs;
pub mod update;
pub mod webfeed;
pub mod ytdlp;
pub mod ytp;
pub mod ytworker;

pub use service::{Command, Core, CoreHandle, CoreOptions, Event, PlaybackInfo, Status};
