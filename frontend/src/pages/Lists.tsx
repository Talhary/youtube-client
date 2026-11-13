import { useEffect, useRef, useState } from 'react';
import { call } from '../bridge';
import { ItemList } from '../components/ItemList';
import { Empty, ErrorBox, Icon, Spinner, TopBar } from '../components/ui';
import { count, img } from '../format';
import { useNav } from '../nav';
import type { ChannelPage, Page, PlaylistPage } from '../types';
import { usePage } from '../usePage';

function SignInPrompt({ what }: { what: string }) {
  return <Empty text={`Sign in from the Library tab to see ${what}.`} />;
}

export function HomeTab({ refresh }: { refresh: number }) {
  const nav = useNav();
  // Signing in or out changes what the home feed contains.
  const page = usePage(`home:${nav.signedIn}:${refresh}`, () => call<Page>({ cmd: 'home' }));
  return (
    <div className="scroll h-full">
      <ItemList page={page} empty="Nothing to show right now." />
    </div>
  );
}

export function SubsTab({ refresh }: { refresh: number }) {
  const nav = useNav();
  const page = usePage(`feed:${nav.signedIn}:${refresh}`, () =>
    nav.signedIn ? call<Page>({ cmd: 'feed' }) : Promise.resolve({ items: [], next: null }),
  );
  if (!nav.signedIn) return <SignInPrompt what="videos from your subscriptions" />;
  return (
    <div className="scroll h-full">
      <ItemList page={page} empty="No recent videos from your subscriptions." />
    </div>
  );
}

/** History, saved playlists or subscribed channels. */
export function AccountList({ kind }: { kind: 'history' | 'playlists' | 'channels' }) {
  const nav = useNav();
  const cmd = kind === 'history' ? 'history' : kind === 'playlists' ? 'saved_playlists' : 'subscriptions';
  const title = kind === 'history' ? 'History' : kind === 'playlists' ? 'Playlists' : 'Channels';
  const page = usePage(`${cmd}:${nav.signedIn}`, () => call<Page>({ cmd }));
  return (
    <div className="flex h-full flex-col">
      <TopBar title={title} onBack={nav.back} />
      <div className="scroll min-h-0 flex-1 pt-2">
        <ItemList page={page} compact empty={`No ${title.toLowerCase()} yet.`} />
      </div>
    </div>
  );
}

export function SearchPage() {
  const nav = useNav();
  const [text, setText] = useState('');
  const [query, setQuery] = useState('');
  const [suggestions, setSuggestions] = useState<string[]>([]);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    input.current?.focus();
  }, []);

  // Suggestions for what is typed, after a short pause.
  useEffect(() => {
    const q = text.trim();
    if (!q || q === query) {
      setSuggestions([]);
      return;
    }
    let live = true;
    const t = window.setTimeout(() => {
      call<{ items: string[] }>({ cmd: 'suggest', query: q })
        .then((r) => live && setSuggestions(r.items ?? []))
        .catch(() => {});
    }, 220);
    return () => {
      live = false;
      window.clearTimeout(t);
    };
  }, [text, query]);

  const run = (q: string) => {
    const clean = q.trim();
    if (!clean) return;
    setText(clean);
    setQuery(clean);
    setSuggestions([]);
    input.current?.blur();
  };

  return (
    <div className="flex h-full flex-col">
      <header className="flex h-14 flex-none items-center gap-1 border-b border-line px-1">
        <button onClick={nav.back} aria-label="Back" className="flex h-12 w-12 items-center justify-center active:opacity-60">
          <Icon name="back" />
        </button>
        <form
          className="flex min-w-0 flex-1 items-center rounded-full bg-panel px-4"
          onSubmit={(e) => {
            e.preventDefault();
            run(text);
          }}
        >
          <input
            ref={input}
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="Search YouTube"
            enterKeyHint="search"
            autoComplete="off"
            autoCorrect="off"
            className="h-10 min-w-0 flex-1 bg-transparent text-[15px] outline-none placeholder:text-mute"
          />
          {text && (
            <button type="button" aria-label="Clear" onClick={() => { setText(''); setQuery(''); input.current?.focus(); }} className="p-1 text-mute">
              <Icon name="close" size={18} />
            </button>
          )}
        </form>
        <div className="w-2" />
      </header>
      <div className="scroll min-h-0 flex-1">
        {suggestions.length > 0 ? (
          <ul>
            {suggestions.map((s) => (
              <li key={s}>
                <button onClick={() => run(s)} className="flex w-full items-center gap-3 px-4 py-3 text-left text-[15px] active:bg-panel">
                  <Icon name="search" size={18} className="flex-none text-mute" />
                  <span className="truncate">{s}</span>
                </button>
              </li>
            ))}
          </ul>
        ) : query ? (
          <SearchResults query={query} />
        ) : (
          <Empty text="Search for videos, channels and playlists." />
        )}
      </div>
    </div>
  );
}

function SearchResults({ query }: { query: string }) {
  const page = usePage(`search:${query}`, () => call<Page>({ cmd: 'search', query }));
  return <ItemList page={page} empty={`No results for "${query}".`} />;
}

export function ChannelView({ id, name }: { id: string; name?: string }) {
  const nav = useNav();
  const [info, setInfo] = useState<ChannelPage | null>(null);
  const page = usePage(`channel:${id}`, async () => {
    const c = await call<ChannelPage>({ cmd: 'channel', id });
    setInfo(c);
    return c.videos;
  });
  const c = info?.channel;
  return (
    <div className="flex h-full flex-col">
      <TopBar title={c?.name ?? name ?? 'Channel'} onBack={nav.back} />
      <div className="scroll min-h-0 flex-1">
        {info?.banner && <img src={img(info.banner)} alt="" className="h-20 w-full object-cover" />}
        {c && (
          <div className="flex items-center gap-3 px-4 py-4">
            <div className="h-14 w-14 flex-none overflow-hidden rounded-full bg-panel">
              {c.avatar && <img src={img(c.avatar)} alt="" className="h-full w-full object-cover" />}
            </div>
            <div className="min-w-0">
              <p className="truncate text-lg font-semibold">{c.name}</p>
              {c.subscribers != null && <p className="text-xs text-mute">{count(c.subscribers)} subscribers</p>}
            </div>
          </div>
        )}
        <ItemList page={page} empty="This channel has no videos." />
      </div>
    </div>
  );
}

export function PlaylistView({ id, name }: { id: string; name?: string }) {
  const nav = useNav();
  const [info, setInfo] = useState<PlaylistPage | null>(null);
  const page = usePage(`playlist:${id}`, async () => {
    const p = await call<PlaylistPage>({ cmd: 'playlist', id });
    setInfo(p);
    return p.videos;
  });
  const p = info?.playlist;
  return (
    <div className="flex h-full flex-col">
      <TopBar title={p?.name ?? name ?? 'Playlist'} onBack={nav.back} />
      <div className="scroll min-h-0 flex-1 pt-2">
        {p && (
          <p className="px-4 pb-2 text-xs text-mute">
            {[p.channel_name, p.video_count != null ? `${p.video_count} videos` : null].filter(Boolean).join(' · ')}
          </p>
        )}
        <ItemList page={page} compact empty="This playlist is empty." />
      </div>
    </div>
  );
}

export { ErrorBox, Spinner };
