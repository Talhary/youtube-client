(async () => {
  const log = [];
  try {
    const B = window.__bg;
    const KEY = 'O43z0dpjhgX20SCx4KAo';
    const same = (u) => String(u).replace('https://www.youtube.com', location.origin);
    const f = (u, init) => fetch(same(u), init);
    // 1. A BotGuard challenge: from this site's home page, else from the Create endpoint.
    let program, globalName, interpreterUrl, interpreterJs;
    const page = await (await fetch(location.origin + '/', { credentials: 'include' })).text();
    const atN = /window\.ytAtN\((\{.*?\})\)/s.exec(page);
    if (atN) {
      const ch = B.parseLooseJSON(atN[1]).R.bgChallenge;
      program = ch.program; globalName = ch.globalName;
      interpreterUrl = ch.interpreterUrl.privateDoNotAccessOrElseTrustedResourceUrlWrappedValue;
      log.push('challenge from home page');
    } else {
      const ch = await B.BG.Challenge.create({ fetch: f, globalObj: globalThis, identifier: 'x', requestKey: KEY });
      program = ch.program; globalName = ch.globalName;
      interpreterJs = ch.interpreterJavascript && ch.interpreterJavascript.privateDoNotAccessOrElseSafeScriptWrappedValue;
      interpreterUrl = ch.interpreterJavascript && ch.interpreterJavascript.privateDoNotAccessOrElseTrustedResourceUrlWrappedValue;
      log.push('challenge from Create endpoint');
    }
    if (interpreterJs) {
      new Function(interpreterJs)();
    } else {
      // The interpreter is on another origin: load it as a script, which needs no CORS.
      await new Promise((resolve, reject) => {
        const s = document.createElement('script');
        s.src = new URL(interpreterUrl, 'https://www.google.com/').href;
        s.onload = resolve;
        s.onerror = () => reject(new Error('interpreter script failed to load'));
        document.head.appendChild(s);
      });
    }
    log.push('interpreter ran; global present: ' + (typeof globalThis[globalName]));
    const client = await B.BotGuardClient.create({ program, globalName, globalObject: globalThis });
    const webPoSignalOutput = [];
    const snapshot = await client.snapshot({ webPoSignalOutput });
    const r = await f(B.buildURL('GenerateIT', true), { method: 'POST', headers: B.getHeaders(), body: JSON.stringify([KEY, snapshot]) });
    log.push('GenerateIT HTTP ' + r.status);
    const [integrityToken, ttl] = await r.json();
    const minter = await B.WebPoMinter.create({ integrityToken, estimatedTtlSecs: ttl }, webPoSignalOutput);
    // 2. Tokens bound to the video and to the visitor/session; try each on a real stream.
    const mp = document.querySelector('#movie_player');
    const pr = mp.getPlayerResponse();
    const videoId = pr.videoDetails.videoId;
    const bindings = { video: videoId };
    try { bindings.visitor = window.ytcfg.get('VISITOR_DATA'); } catch (e) {}
    try { bindings.datasync = (window.ytcfg.get('DATASYNC_ID') || '').split('||')[0]; } catch (e) {}
    const gv = performance.getEntriesByType('resource').map((e) => e.name).filter((n) => /googlevideo\.com\/videoplayback/.test(n));
    const liveN = gv.length ? new URL(gv[gv.length - 1]).searchParams.get('n') : null;
    log.push('ttl ' + ttl + 's; live n: ' + !!liveN + '; video ' + videoId);
    const fmt = pr.streamingData.adaptiveFormats.find((x) => /audio\/mp4/.test(x.mimeType));
    for (const [name, id] of Object.entries(bindings)) {
      if (!id) continue;
      const pot = await minter.mintAsWebsafeString(id);
      for (const useN of liveN ? [true, false] : [false]) {
        const u = new URL(fmt.url);
        if (useN) u.searchParams.set('n', liveN);
        u.searchParams.set('pot', pot);
        u.searchParams.set('range', '0-1048575');
        const t0 = performance.now();
        try {
          const resp = await fetch(u.toString(), { credentials: 'omit' });
          const b = await resp.arrayBuffer();
          log.push('pot bound to ' + name + (useN ? ' + player n' : ' + raw n') + ': HTTP ' + resp.status + ', ' + b.byteLength + ' bytes, ' + Math.round(performance.now() - t0) + ' ms');
        } catch (e) { log.push('pot ' + name + ': fetch error ' + e); }
      }
    }
  } catch (e) { log.push('FAILED: ' + (e && e.stack ? e.stack.split('\n').slice(0, 3).join(' | ') : e)); }
  return JSON.stringify(log, null, 1);
})()
