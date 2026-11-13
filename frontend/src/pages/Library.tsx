import { send } from '../bridge';
import { Icon } from '../components/ui';
import { useNav } from '../nav';
import type { Settings, Status } from '../types';

function Row({ icon, label, onClick, hint }: { icon: Parameters<typeof Icon>[0]['name']; label: string; onClick: () => void; hint?: string }) {
  return (
    <button onClick={onClick} className="flex w-full items-center gap-4 px-4 py-3.5 text-left active:bg-panel">
      <Icon name={icon} className="flex-none text-mute" />
      <span className="min-w-0 flex-1">
        <span className="block text-[15px]">{label}</span>
        {hint && <span className="block text-xs text-mute">{hint}</span>}
      </span>
      <Icon name="chevron" size={18} className="text-mute" />
    </button>
  );
}

export function LibraryTab({ authKnown, signingIn }: { authKnown: boolean; signingIn: boolean }) {
  const nav = useNav();
  if (!nav.signedIn) {
    return (
      <div className="flex h-full flex-col items-center justify-center px-8 text-center">
        <span className="flex h-16 w-16 items-center justify-center rounded-full bg-panel text-accent">
          <Icon name="user" size={30} />
        </span>
        <h2 className="mt-5 text-lg font-semibold">Sign in to YouTube</h2>
        <p className="mt-2 text-sm text-mute">
          See your subscriptions, history and playlists. You sign in on Google's own page; the session stays on this device.
        </p>
        <button
          disabled={signingIn || !authKnown}
          onClick={() => send({ cmd: 'sign_in' })}
          className="mt-6 rounded-full bg-accent px-7 py-3 text-sm font-semibold text-black active:opacity-80 disabled:opacity-50"
        >
          {signingIn ? 'Waiting for sign-in…' : 'Sign in with Google'}
        </button>
      </div>
    );
  }
  return (
    <div className="scroll h-full py-2">
      <Row icon="history" label="History" onClick={() => nav.push({ kind: 'history' })} />
      <Row icon="playlist" label="Playlists" onClick={() => nav.push({ kind: 'playlists' })} />
      <Row icon="user" label="Channels" hint="Channels you are subscribed to" onClick={() => nav.push({ kind: 'channels' })} />
    </div>
  );
}

function Toggle({ on, onChange, label, hint }: { on: boolean; onChange: (v: boolean) => void; label: string; hint: string }) {
  return (
    <button role="switch" aria-checked={on} onClick={() => onChange(!on)} className="flex w-full items-center gap-4 px-4 py-3.5 text-left active:bg-panel">
      <span className="min-w-0 flex-1">
        <span className="block text-[15px]">{label}</span>
        <span className="block text-xs text-mute">{hint}</span>
      </span>
      <span className={`relative h-6 w-11 flex-none rounded-full transition-colors ${on ? 'bg-accent' : 'bg-line'}`}>
        <span className={`absolute top-0.5 h-5 w-5 rounded-full bg-white transition-all ${on ? 'left-[22px]' : 'left-0.5'}`} />
      </span>
    </button>
  );
}

export function SettingsTab({ settings, status, adRules }: { settings: Settings | null; status: Status | null; adRules: string | null }) {
  const nav = useNav();
  const update = (patch: Partial<Settings>) => {
    if (settings) send({ cmd: 'update_settings', settings: { ...settings, ...patch } });
  };
  return (
    <div className="scroll h-full py-2">
      <h2 className="px-4 pb-1 pt-3 text-xs font-semibold uppercase tracking-wide text-mute">Playback</h2>
      <Toggle
        on={settings?.autoplay ?? true}
        onChange={(v) => update({ autoplay: v })}
        label="Play next video automatically"
        hint="When a video ends, start the first one under Up next"
      />
      <h2 className="px-4 pb-1 pt-5 text-xs font-semibold uppercase tracking-wide text-mute">Account</h2>
      {nav.signedIn ? (
        <button onClick={() => send({ cmd: 'sign_out' })} className="block w-full px-4 py-3.5 text-left text-[15px] text-red-400 active:bg-panel">
          Sign out
        </button>
      ) : (
        <button onClick={() => send({ cmd: 'sign_in' })} className="block w-full px-4 py-3.5 text-left text-[15px] text-accent active:bg-panel">
          Sign in with Google
        </button>
      )}
      <h2 className="px-4 pb-1 pt-5 text-xs font-semibold uppercase tracking-wide text-mute">About</h2>
      <div className="space-y-1 px-4 py-2 text-sm text-mute">
        <p>StreamPipe {status?.version ?? ''}</p>
        <p>Ad rules: {adRules ?? 'loading…'}</p>
      </div>
    </div>
  );
}
