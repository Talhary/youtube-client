import { createContext, useContext } from 'react';
import type { VideoCard } from './types';

export type Route =
  | { kind: 'search' }
  | { kind: 'channel'; id: string; name?: string }
  | { kind: 'playlist'; id: string; name?: string }
  | { kind: 'history' }
  | { kind: 'playlists' }
  | { kind: 'channels' };

export interface Nav {
  push(route: Route): void;
  back(): void;
  play(video: VideoCard): void;
  signedIn: boolean;
}

export const NavContext = createContext<Nav>({
  push() {},
  back() {},
  play() {},
  signedIn: false,
});

export const useNav = () => useContext(NavContext);
