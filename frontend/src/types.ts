// Shapes of the core's data as it arrives in JSON.

export interface VideoCard {
  id: string;
  title: string;
  channel_id: string | null;
  channel_name: string | null;
  duration_s: number | null;
  thumbnail: string | null;
  views: number | null;
  published: string | null;
  is_live: boolean;
}

export interface ChannelCard {
  id: string;
  name: string;
  avatar: string | null;
  subscribers: number | null;
}

export interface PlaylistCard {
  id: string;
  name: string;
  thumbnail: string | null;
  channel_name: string | null;
  video_count: number | null;
}

export type Item = { Video: VideoCard } | { Channel: ChannelCard } | { Playlist: PlaylistCard };

export interface Page {
  items: Item[];
  /** Handle for the next page; usable once. */
  next: number | null;
}

export interface ChannelPage {
  channel: ChannelCard;
  banner: string | null;
  description: string;
  videos: Page;
}

export interface PlaylistPage {
  playlist: PlaylistCard;
  videos: Page;
}

export interface VideoPage {
  video: VideoCard;
  description: string;
  likes: number | null;
  related: Page;
}

export interface Settings {
  autoplay: boolean;
  [key: string]: unknown;
}

export interface Status {
  version: string;
  ytdlp: string | null;
}

export type AuthState = 'unknown' | 'in' | 'out';

export interface PlayerState {
  id: string | null;
  ad: boolean;
  pos: number | null;
  dur: number | null;
  paused: boolean | null;
  height: number | null;
  signedIn: boolean | null;
}

export type AppEvent =
  | { ev: 'ready'; status: Status }
  | { ev: 'settings'; settings: Settings }
  | { ev: 'auth'; auth: unknown }
  | { ev: 'toast'; text: string }
  | { ev: 'back' }
  | { ev: 'signin'; state: 'open' | 'done' | 'cancelled' | 'failed'; text?: string }
  | { ev: 'player_state'; state: PlayerState }
  | { ev: 'player_video'; id: string }
  | { ev: 'player_ended'; id: string }
  | { ev: 'player_fullscreen'; on: boolean }
  | { ev: string; [key: string]: unknown };
