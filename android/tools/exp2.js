// Experiment: download one audio track of the playing video through YouTube's streaming
// protocol, inside the player page, and report how many bytes arrive. Nothing is saved.
(async () => {
  const log = [];
  const D = window.__dl;
  try {
    const KEY = 'O43z0dpjhgX20SCx4KAo';
    const same = (u) => String(u).replace('https://www.youtube.com', location.origin);
    // 1. Token minter (same recipe as the first experiment).
    const page = await (await fetch(location.origin + '/', { credentials: 'include' })).text();
    const atN = /window\.ytAtN\((\{.*?\})\)/s.exec(page);
    if (!atN) throw new Error('the home page carries no challenge');
    const ch = D.parseLooseJSON(atN[1]).R.bgChallenge;
    if (typeof globalThis[ch.globalName] === 'undefined') {
      await new Promise((resolve, reject) => {
        const s = document.createElement('script');
        s.src = new URL(ch.interpreterUrl.privateDoNotAccessOrElseTrustedResourceUrlWrappedValue, 'https://www.google.com/').href;
        s.onload = resolve;
        s.onerror = () => reject(new Error('interpreter script failed to load'));
        document.head.appendChild(s);
      });
    }
    const client = await D.BotGuardClient.create({ program: ch.program, globalName: ch.globalName, globalObject: globalThis });
    const webPoSignalOutput = [];
    const snapshot = await client.snapshot({ webPoSignalOutput });
    const it = await fetch(same(D.buildURL('GenerateIT', true)), { method: 'POST', headers: D.getHeaders(), body: JSON.stringify([KEY, snapshot]) });
    const [integrityToken, ttl] = await it.json();
    const minter = await D.WebPoMinter.create({ integrityToken, estimatedTtlSecs: ttl }, webPoSignalOutput);
    log.push('minter ready (GenerateIT HTTP ' + it.status + ')');

    // 2. What the page knows about the video.
    const mp = document.querySelector('#movie_player');
    const pr = mp.getPlayerResponse();
    const videoId = pr.videoDetails.videoId;
    const sd = pr.streamingData;
    const config = pr.playerConfig && pr.playerConfig.mediaCommonConfig && pr.playerConfig.mediaCommonConfig.mediaUstreamerRequestConfig
      && pr.playerConfig.mediaCommonConfig.mediaUstreamerRequestConfig.videoPlaybackUstreamerConfig;
    const gv = performance.getEntriesByType('resource').map((e) => e.name).filter((n) => /googlevideo\.com\/videoplayback/.test(n));
    const liveN = gv.length ? new URL(gv[gv.length - 1]).searchParams.get('n') : null;
    const sabrUrl = new URL(sd.serverAbrStreamingUrl);
    if (liveN) sabrUrl.searchParams.set('n', liveN);
    const formats = sd.adaptiveFormats.map((f) => D.buildSabrFormat(f));
    const raw = sd.adaptiveFormats.filter((f) => /^audio\/mp4/.test(f.mimeType) && !f.isDrc).sort((a, b) => b.bitrate - a.bitrate)[0];
    const audio = formats.find((f) => f.itag === raw.itag);
    log.push('video ' + videoId + '; config: ' + !!config + '; player n: ' + !!liveN + '; audio itag ' + raw.itag + ' (' + raw.contentLength + ' bytes expected)');

    const bindings = [['video', videoId]];
    try { const d = (window.ytcfg.get('DATASYNC_ID') || '').split('||')[0]; if (d) bindings.push(['account', d]); } catch (e) {}
    try { const v = window.ytcfg.get('VISITOR_DATA'); if (v) bindings.push(['visitor', v]); } catch (e) {}

    const tokenBytes = (t) => Uint8Array.from(atob(t.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));
    const LIMIT = 3 * 1024 * 1024;
    for (const [name, id] of bindings) {
      const statuses = [];
      const stream = new D.SabrStream({
        videoId,
        // The library's extra headers make the browser send a preflight that the stream
        // server does not answer; the body alone is enough.
        fetchFunction: async (url, init) => {
          const r = await fetch(url, { method: init.method, body: init.body, signal: init.signal });
          statuses.push(r.status);
          return r;
        },
        serverAbrStreamingUrl: sabrUrl.toString(),
        videoPlaybackUstreamerConfig: config,
        clientInfo: { clientName: Number(window.ytcfg.get('INNERTUBE_CONTEXT_CLIENT_NAME')), clientVersion: window.ytcfg.get('INNERTUBE_CONTEXT_CLIENT_VERSION') },
        formats,
        callbacks: { onMintPoToken: async () => tokenBytes(await minter.mintAsWebsafeString(id)) },
      });
      const t0 = performance.now();
      let bytes = 0, outcome = 'complete';
      try {
        const { audioStream } = await stream.start({
          audioFormat: audio, enabledTrackTypes: D.EnabledTrackTypes.AUDIO_ONLY, isPostLiveDvr: false, maxRetries: 2,
        });
        const reader = audioStream.getReader();
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          bytes += value.byteLength;
          if (bytes >= LIMIT) { outcome = 'stopped at 3 MB'; break; }
        }
      } catch (e) { outcome = 'FAILED: ' + String(e && e.message || e).slice(0, 160); }
      try { await stream.abort(); } catch (e) {}
      log.push('token bound to ' + name + ': ' + outcome + ', ' + bytes + ' bytes in ' + Math.round(performance.now() - t0) + ' ms; HTTP ' + [...new Set(statuses)].join(','));
      if (bytes > 100000) break;
    }
  } catch (e) { log.push('FAILED: ' + (e && e.stack ? e.stack.split('\n').slice(0, 3).join(' | ') : e)); }
  return JSON.stringify(log, null, 1);
})()
