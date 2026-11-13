// Comparison: capture one of the player's own stream requests and one of ours, decode
// both, and report how they differ. Secrets are reported by length only.
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
      if (typeof body === 'string') return new TextEncoder().encode(body);
      return null;
    };

    // 1. The player's request: watch fetch and XHR until one stream POST goes out.
    let captured = null;
    const prevFetch = window.fetch;
    window.fetch = function (input, init) {
      try {
        const url = typeof input === 'string' ? input : input && input.url;
        const method = (init && init.method) || (input && input.method) || 'GET';
        if (!captured && isStream(url) && method === 'POST') {
          const headers = {};
          try { new Headers((init && init.headers) || (input && input.headers) || {}).forEach((v, k) => { headers[k] = v; }); } catch (e) {}
          const entry = { via: 'fetch', url, headers: Object.keys(headers), body: null };
          captured = entry;
          const src = init && init.body ? Promise.resolve(init.body) : (input && input.clone ? input.clone().arrayBuffer() : Promise.resolve(null));
          src.then(toBytes).then((b) => { entry.body = b; }).catch(() => {});
        }
      } catch (e) {}
      return prevFetch.apply(this, arguments);
    };
    const X = XMLHttpRequest.prototype, open = X.open, send = X.send;
    X.open = function (m, u) { this.__m = m; this.__u = u; return open.apply(this, arguments); };
    X.send = function (body) {
      if (!captured && this.__m === 'POST' && isStream(this.__u)) {
        const entry = { via: 'xhr', url: String(this.__u), headers: [], body: null };
        captured = entry;
        toBytes(body).then((b) => { entry.body = b; }).catch(() => {});
      }
      return send.apply(this, arguments);
    };
    const v = document.querySelector('video');
    const mp = document.querySelector('#movie_player');
    // Jump ahead so the player has to ask for new data.
    try { mp.seekTo(Math.min((v.duration || 600) - 30, v.currentTime + 120), true); mp.playVideo(); } catch (e) {}
    for (let i = 0; i < 40 && !(captured && captured.body); i++) await new Promise((r) => setTimeout(r, 300));
    window.fetch = prevFetch; X.open = open; X.send = send;
    if (!captured || !captured.body) { out.error = 'the player sent no stream request in 12 s'; return JSON.stringify(out, null, 1); }

    // 2. Our request: let the library build one and keep its body.
    const pr = mp.getPlayerResponse();
    const sd = pr.streamingData;
    const config = pr.playerConfig.mediaCommonConfig.mediaUstreamerRequestConfig.videoPlaybackUstreamerConfig;
    const formats = sd.adaptiveFormats.map((f) => D.buildSabrFormat(f));
    const raw = sd.adaptiveFormats.filter((f) => /^audio\/mp4/.test(f.mimeType) && !f.isDrc).sort((a, b) => b.bitrate - a.bitrate)[0];
    let ours = null;
    const stream = new D.SabrStream({
      videoId: pr.videoDetails.videoId,
      fetchFunction: async (url, init) => {
        if (!ours) ours = { url: String(url), body: await toBytes(init.body) };
        throw new Error('captured');
      },
      serverAbrStreamingUrl: sd.serverAbrStreamingUrl,
      videoPlaybackUstreamerConfig: config,
      clientInfo: { clientName: Number(window.ytcfg.get('INNERTUBE_CONTEXT_CLIENT_NAME')), clientVersion: window.ytcfg.get('INNERTUBE_CONTEXT_CLIENT_VERSION') },
      formats,
      callbacks: { onMintPoToken: async () => new Uint8Array(100) },
    });
    // The library keeps its stream open after a failed request, so only wait until the
    // first request has been built (or a few seconds at most).
    (async () => {
      try {
        const { audioStream } = await stream.start({ audioFormat: formats.find((f) => f.itag === raw.itag), enabledTrackTypes: D.EnabledTrackTypes.AUDIO_ONLY, isPostLiveDvr: false, maxRetries: 0 });
        await audioStream.getReader().read();
      } catch (e) {}
    })();
    for (let i = 0; i < 30 && !ours; i++) await new Promise((r) => setTimeout(r, 200));
    stream.abort().catch(() => {});

    // 3. Decode and summarise.
    const shape = (x, depth) => {
      if (x === null || x === undefined) return x;
      if (x instanceof Uint8Array) return 'bytes(' + x.length + ')';
      if (typeof x === 'string') return x.length > 48 ? 'text(' + x.length + ')' : x;
      if (typeof x !== 'object') return x;
      if (depth > 5) return '…';
      if (Array.isArray(x)) return x.length > 4 ? ['list of ' + x.length, shape(x[0], depth + 1)] : x.map((y) => shape(y, depth + 1));
      const o = {};
      for (const k of Object.keys(x)) {
        const val = shape(x[k], depth + 1);
        if (val !== undefined && !(Array.isArray(val) && val.length === 0)) o[k] = val;
      }
      return o;
    };
    const decode = (bytes) => {
      try { return shape(D.Protos.VideoPlaybackAbrRequest.decode(bytes), 0); } catch (e) { return 'could not decode: ' + e; }
    };
    const params = (u) => { const p = new URL(u); return { host: p.host.replace(/^[^.]+/, 'rr*'), keys: [...p.searchParams.keys()].sort() }; };
    const pp = params(captured.url), op = params(ours ? ours.url : sd.serverAbrStreamingUrl);
    out.player = { via: captured.via, headers: captured.headers, bodyBytes: captured.body.length, host: pp.host,
      paramsOnlyInPlayer: pp.keys.filter((k) => !op.keys.includes(k)), message: decode(captured.body) };
    out.ours = ours ? { bodyBytes: ours.body.length, host: op.host, paramsOnlyInOurs: op.keys.filter((k) => !pp.keys.includes(k)), message: decode(ours.body) }
      : { error: 'the library built no request' };
    const pu = new URL(captured.url), ou = new URL(ours ? ours.url : sd.serverAbrStreamingUrl);
    out.sameValue = {};
    for (const k of ['id', 'expire', 'ei', 'ip', 'sig', 'lsig', 'n', 'mn', 'ms', 'mv', 'fvip', 'c', 'cver', 'sabr', 'svpuc', 'ns']) {
      out.sameValue[k] = pu.searchParams.get(k) === ou.searchParams.get(k);
    }
  } catch (e) { out.error = String(e && e.stack ? e.stack.split('\n').slice(0, 3).join(' | ') : e); }
  return JSON.stringify(out, null, 1);
})()
