// Test: take one of the player's own stream requests, change only what is being asked for
// (the audio track from the start instead of the next piece of what is playing), send it,
// and see whether media comes back. Nothing is saved.
(async () => {
  const out = {};
  const D = window.__dl;
  try {
    const isStream = (u) => /googlevideo\.com\/videoplayback/.test(String(u));
    const toBytes = async (body) => {
      if (!body) return null;
      if (body instanceof Uint8Array) return body.slice();
      if (body instanceof ArrayBuffer) return new Uint8Array(body.slice(0));
      if (ArrayBuffer.isView(body)) return new Uint8Array(body.buffer.slice(body.byteOffset, body.byteOffset + body.byteLength));
      if (body instanceof Blob) return new Uint8Array(await body.arrayBuffer());
      return null;
    };
    let captured = null;
    const prevFetch = window.fetch;
    window.fetch = function (input, init) {
      try {
        const url = typeof input === 'string' ? input : input && input.url;
        const method = (init && init.method) || (input && input.method) || 'GET';
        if (!captured && isStream(url) && method === 'POST') {
          const entry = { url, body: null };
          captured = entry;
          const src = init && init.body ? Promise.resolve(init.body) : (input && input.clone ? input.clone().arrayBuffer() : Promise.resolve(null));
          src.then(toBytes).then((b) => { entry.body = b; }).catch(() => {});
        }
      } catch (e) {}
      return prevFetch.apply(this, arguments);
    };
    const v = document.querySelector('video');
    const mp = document.querySelector('#movie_player');
    try { mp.seekTo(Math.min((v.duration || 600) - 30, v.currentTime + 60), true); mp.playVideo(); } catch (e) {}
    for (let i = 0; i < 40 && !(captured && captured.body); i++) await new Promise((r) => setTimeout(r, 300));
    window.fetch = prevFetch;
    if (!captured || !captured.body) { out.error = 'the player sent no stream request in 12 s'; return JSON.stringify(out); }

    const pr = mp.getPlayerResponse();
    const raw = pr.streamingData.adaptiveFormats.filter((f) => /^audio\/mp4/.test(f.mimeType) && !f.isDrc).sort((a, b) => b.bitrate - a.bitrate)[0];
    const P = D.Protos.VideoPlaybackAbrRequest;
    const msg = P.decode(captured.body);
    msg.bufferedRanges = [];
    msg.initializationFormatIds = [];
    msg.selectedVideoFormatIds = [];
    msg.selectedAudioFormatIds = [{ itag: raw.itag, lastModified: raw.lastModified, xtags: raw.xtags }];
    if (msg.clientAbrState) {
      msg.clientAbrState.playerTimeMs = '0';
      msg.clientAbrState.enabledTrackTypesBitfield = 1; // audio only
    }
    const body = P.encode(msg).finish();
    const url = new URL(captured.url);
    url.searchParams.set('rn', String(Number(url.searchParams.get('rn') || '0') + 500));
    out.asking = 'audio itag ' + raw.itag + ' (' + raw.contentLength + ' bytes in total), from the start';

    const t0 = performance.now();
    const resp = await prevFetch(url.toString(), { method: 'POST', body });
    out.status = resp.status;
    out.contentType = resp.headers.get('content-type');
    const reader = resp.body.getReader();
    let bytes = 0;
    const head = [];
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      bytes += value.byteLength;
      if (head.length < 4) head.push(value);
      if (bytes > 2 * 1024 * 1024) { try { await reader.cancel(); } catch (e) {} break; }
    }
    out.bytes = bytes;
    out.ms = Math.round(performance.now() - t0);
    // MP4 audio starts with an "ftyp" box; its presence means real media came back.
    const text = head.map((c) => String.fromCharCode.apply(null, Array.from(c.subarray(0, Math.min(c.length, 60000))))).join('');
    out.looksLikeMedia = { ftyp: text.includes('ftyp'), moof: text.includes('moof'), mdat: text.includes('mdat') };
  } catch (e) { out.error = String(e && e.stack ? e.stack.split('\n').slice(0, 3).join(' | ') : e); }
  return JSON.stringify(out, null, 1);
})()
