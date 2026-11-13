// Diagnostic: what does the streaming library send, and does the request get through when
// it is shaped like a plain cross-origin request (no headers that need a preflight)?
(async () => {
  const log = [];
  const D = window.__dl;
  try {
    const mp = document.querySelector('#movie_player');
    const pr = mp.getPlayerResponse();
    const sd = pr.streamingData;
    const config = pr.playerConfig.mediaCommonConfig.mediaUstreamerRequestConfig.videoPlaybackUstreamerConfig;
    const gv = performance.getEntriesByType('resource').map((e) => e.name).filter((n) => /googlevideo\.com\/videoplayback/.test(n));
    const liveN = gv.length ? new URL(gv[gv.length - 1]).searchParams.get('n') : null;
    const sabrUrl = new URL(sd.serverAbrStreamingUrl);
    if (liveN) sabrUrl.searchParams.set('n', liveN);
    const formats = sd.adaptiveFormats.map((f) => D.buildSabrFormat(f));
    const raw = sd.adaptiveFormats.filter((f) => /^audio\/mp4/.test(f.mimeType) && !f.isDrc).sort((a, b) => b.bitrate - a.bitrate)[0];
    const audio = formats.find((f) => f.itag === raw.itag);
    log.push('player stream requests seen: ' + gv.length + '; same host as ours: ' + (gv.length ? new URL(gv[gv.length - 1]).host === sabrUrl.host : 'n/a'));

    for (const mode of ['as the library sends it', 'without custom headers']) {
      const seen = [];
      const stream = new D.SabrStream({
        videoId: pr.videoDetails.videoId,
        fetchFunction: async (url, init) => {
          const headers = {};
          new Headers(init && init.headers).forEach((v, k) => { headers[k] = v; });
          const entry = { method: init && init.method, headers: Object.keys(headers).join(','), bodyBytes: init && init.body ? (init.body.byteLength || init.body.length || 0) : 0 };
          seen.push(entry);
          const use = mode === 'without custom headers' ? { method: init.method, body: init.body, signal: init.signal } : init;
          const t0 = performance.now();
          try {
            const r = await fetch(url, use);
            entry.status = r.status;
            entry.type = r.headers.get('content-type');
            entry.ms = Math.round(performance.now() - t0);
            return r;
          } catch (e) {
            entry.error = String(e && e.message || e).slice(0, 80);
            entry.ms = Math.round(performance.now() - t0);
            throw e;
          }
        },
        serverAbrStreamingUrl: sabrUrl.toString(),
        videoPlaybackUstreamerConfig: config,
        clientInfo: { clientName: Number(window.ytcfg.get('INNERTUBE_CONTEXT_CLIENT_NAME')), clientVersion: window.ytcfg.get('INNERTUBE_CONTEXT_CLIENT_VERSION') },
        formats,
      });
      let bytes = 0, outcome = 'complete';
      const timer = setTimeout(() => { stream.abort().catch(() => {}); }, 20000);
      try {
        const { audioStream } = await stream.start({ audioFormat: audio, enabledTrackTypes: D.EnabledTrackTypes.AUDIO_ONLY, isPostLiveDvr: false, maxRetries: 0 });
        const reader = audioStream.getReader();
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          bytes += value.byteLength;
          if (bytes >= 1024 * 1024) { outcome = 'stopped at 1 MB'; break; }
        }
      } catch (e) { outcome = 'FAILED: ' + String(e && e.message || e).slice(0, 120); }
      clearTimeout(timer);
      try { await stream.abort(); } catch (e) {}
      log.push(mode + ': ' + outcome + ', ' + bytes + ' bytes; requests: ' + JSON.stringify(seen.slice(0, 3)));
    }
  } catch (e) { log.push('FAILED: ' + (e && e.stack ? e.stack.split('\n').slice(0, 3).join(' | ') : e)); }
  return JSON.stringify(log, null, 1);
})()
