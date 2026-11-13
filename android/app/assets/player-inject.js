// StreamPipe player page script. Runs in YouTube's watch page before any page script.
//
// 1. Applies uBlock Origin's YouTube script rules (downloaded by the core, passed in as
//    RULES) with our own implementations of the rule types they use.
// 2. If an ad starts anyway, jumps to its end and presses Skip.
// 3. Shows only the video player, and reports playback state to the app.
(function (RULES, OPTS) {
  'use strict';
  if (window.__spInjected) return;
  window.__spInjected = true;
  var host = location.hostname;
  if (!/(^|\.)youtube\.com$/.test(host)) return;
  var top = window.top === window;
  var post = function (o) {
    try { if (top && window.SPP) window.SPP.post(JSON.stringify(o)); } catch (e) {}
  };
  var stats = { applied: 0, unsupported: [], failed: [], pruned: 0, replaced: 0, skipped: 0 };

  // ------------------------------------------------------------------ helpers
  var escapeRe = function (s) { return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); };
  // uBlock pattern: "/regex/flags" or a plain substring; empty matches everything.
  var toRe = function (s) {
    if (s === undefined || s === null || s === '') return /^/;
    var m = /^\/(.+)\/([gimsuy]*)$/.exec(s);
    if (m) { try { return new RegExp(m[1], m[2]); } catch (e) {} }
    return new RegExp(escapeRe(s));
  };
  // "propsToMatch" value: space-separated "prop:pattern" pairs; a bare pattern means the URL.
  var requestMatcher = function (spec) {
    var tests = [];
    String(spec || '').split(/\s+/).forEach(function (tok) {
      if (!tok) return;
      var m = /^(url|method):(.*)$/.exec(tok);
      tests.push(m ? [m[1], toRe(m[2])] : ['url', toRe(tok)]);
    });
    return function (url, method) {
      for (var i = 0; i < tests.length; i++) {
        var v = tests[i][0] === 'url' ? String(url) : String(method || 'GET');
        tests[i][1].lastIndex = 0;
        if (!tests[i][1].test(v)) return false;
      }
      return true;
    };
  };
  // Named extra arguments: (..., 'propsToMatch', value, ...).
  var extra = function (args, from) {
    var o = {};
    for (var i = from; i + 1 < args.length; i += 2) o[args[i]] = args[i + 1];
    return o;
  };

  // uBlock prune paths: dot-separated; "[]" any array item, "*" any property,
  // "[-]"/"{-}" remove the containing item when the rest of the path exists in it.
  var walk = function (owner, segs, i, prune) {
    if (owner === null || typeof owner !== 'object') return false;
    var seg = segs[i], last = i === segs.length - 1, found = false, keys, k, j;
    if (seg === '[-]' || seg === '{-}') {
      keys = Object.keys(owner);
      for (j = keys.length - 1; j >= 0; j--) {
        k = keys[j];
        if (last || walk(owner[k], segs, i + 1, false)) {
          found = true;
          if (prune) { if (Array.isArray(owner)) owner.splice(Number(k), 1); else delete owner[k]; }
        }
      }
      return found;
    }
    if (seg === '[]' || seg === '*') {
      if (seg === '[]' && !Array.isArray(owner)) return false;
      keys = Object.keys(owner);
      for (j = 0; j < keys.length; j++) {
        k = keys[j];
        if (last) { found = true; if (prune) delete owner[k]; }
        else if (walk(owner[k], segs, i + 1, prune)) found = true;
      }
      return found;
    }
    if (!Object.prototype.hasOwnProperty.call(owner, seg)) return false;
    if (last) { if (prune) delete owner[seg]; return true; }
    return walk(owner[seg], segs, i + 1, prune);
  };
  var pruner = function (rawPaths, rawNeedles) {
    var paths = String(rawPaths || '').split(/\s+/).filter(Boolean).map(function (p) { return p.split('.'); });
    var needles = String(rawNeedles || '').split(/\s+/).filter(Boolean).map(function (p) { return p.split('.'); });
    return function (obj) {
      if (obj === null || typeof obj !== 'object') return false;
      for (var n = 0; n < needles.length; n++) if (!walk(obj, needles[n], 0, false)) return false;
      var hit = false;
      for (var p = 0; p < paths.length; p++) {
        try { if (walk(obj, paths[p], 0, true)) hit = true; } catch (e) {}
      }
      if (hit) stats.pruned++;
      return hit;
    };
  };

  // ------------------------------------------------------------- fetch / XHR
  // Each hook gets the response text and returns the new text, or undefined to keep it.
  var fetchHooks = [];
  var realFetch = window.fetch;
  if (typeof realFetch === 'function') {
    window.fetch = function (input, init) {
      var url = typeof input === 'string' ? input : (input && input.url) || String(input);
      var method = (init && init.method) || (input && input.method) || 'GET';
      var promise = realFetch.apply(this, arguments);
      var mine = fetchHooks.filter(function (h) { return h.match(url, method); });
      if (!mine.length) return promise;
      return promise.then(function (resp) {
        return resp.clone().text().then(function (text) {
          var out = text;
          mine.forEach(function (h) {
            try { var t = h.edit(out); if (typeof t === 'string') out = t; } catch (e) {}
          });
          if (out === text) return resp;
          var r = new Response(out, { status: resp.status, statusText: resp.statusText, headers: resp.headers });
          try { Object.defineProperty(r, 'url', { value: resp.url }); } catch (e) {}
          return r;
        }).catch(function () { return resp; });
      });
    };
    try { window.fetch.toString = function () { return realFetch.toString(); }; } catch (e) {}
  }

  var xhrHooks = [];
  (function () {
    var X = window.XMLHttpRequest && XMLHttpRequest.prototype;
    if (!X) return;
    var open = X.open;
    X.open = function (method, url) {
      var mine = xhrHooks.filter(function (h) { return h.match(String(url), method); });
      if (mine.length) this.__spHooks = mine;
      return open.apply(this, arguments);
    };
    var edited = function (xhr, raw) {
      if (xhr.__spRaw === raw) return xhr.__spOut;
      var out = raw;
      xhr.__spHooks.forEach(function (h) {
        try { var t = h.edit(out); if (typeof t === 'string') out = t; } catch (e) {}
      });
      xhr.__spRaw = raw; xhr.__spOut = out;
      return out;
    };
    var dText = Object.getOwnPropertyDescriptor(X, 'responseText');
    var dResp = Object.getOwnPropertyDescriptor(X, 'response');
    if (!dText || !dResp) return;
    Object.defineProperty(X, 'responseText', {
      configurable: true, enumerable: dText.enumerable,
      get: function () {
        var raw = dText.get.call(this);
        if (!this.__spHooks || this.readyState !== 4 || typeof raw !== 'string') return raw;
        return edited(this, raw);
      }
    });
    Object.defineProperty(X, 'response', {
      configurable: true, enumerable: dResp.enumerable,
      get: function () {
        var raw = dResp.get.call(this);
        if (!this.__spHooks || this.readyState !== 4) return raw;
        if (typeof raw === 'string') return edited(this, raw);
        if (this.responseType === 'json' && raw && typeof raw === 'object') {
          if (this.__spObj === undefined) {
            var text = JSON.stringify(raw), out = edited(this, text);
            try { this.__spObj = out === text ? raw : JSON.parse(out); } catch (e) { this.__spObj = raw; }
          }
          return this.__spObj;
        }
        return raw;
      }
    });
  })();

  var textReplacer = function (pattern, replacement) {
    var re = toRe(pattern);
    return function (text) {
      re.lastIndex = 0;
      if (!re.test(text)) return undefined;
      re.lastIndex = 0;
      stats.replaced++;
      return text.replace(re, function () { return replacement; });
    };
  };
  var jsonEditor = function (prune) {
    return function (text) {
      var obj;
      try { obj = JSON.parse(text); } catch (e) { return undefined; }
      return prune(obj) ? JSON.stringify(obj) : undefined;
    };
  };

  // ------------------------------------------------------------ set-constant
  // Several rules can target the same root (ytInitialPlayerResponse.adSlots, .playerAds…),
  // so each watched property keeps the list of everything to apply below it.
  var watched = new WeakMap();
  var constValue = function (raw) {
    switch (raw) {
      case 'undefined': return undefined;
      case 'false': return false;
      case 'true': return true;
      case 'null': return null;
      case "''": case '': return '';
      case '[]': case 'emptyArr': return [];
      case '{}': case 'emptyObj': return {};
      case 'noopFunc': return function () {};
      case 'trueFunc': return function () { return true; };
      case 'falseFunc': return function () { return false; };
      default:
        if (/^-?\d+(\.\d+)?$/.test(raw)) return Number(raw);
        return raw;
    }
  };
  var trap = function (owner, parts, value) {
    if (owner === null || (typeof owner !== 'object' && typeof owner !== 'function')) return;
    var prop = parts[0];
    if (parts.length === 1) {
      try {
        Object.defineProperty(owner, prop, { configurable: true, get: function () { return value; }, set: function () {} });
      } catch (e) {}
      return;
    }
    var props = watched.get(owner);
    if (!props) { props = {}; watched.set(owner, props); }
    var w = props[prop];
    if (!w) {
      w = props[prop] = { inner: undefined, subs: [] };
      try { w.inner = owner[prop]; } catch (e) {}
      try {
        Object.defineProperty(owner, prop, {
          configurable: true,
          get: function () { return w.inner; },
          set: function (v) {
            w.inner = v;
            w.subs.forEach(function (s) { trap(v, s.parts, s.value); });
          }
        });
      } catch (e) { return; }
    }
    w.subs.push({ parts: parts.slice(1), value: value });
    trap(w.inner, parts.slice(1), value);
  };

  // ---------------------------------------------------------------- node text
  var nodeTextRules = [];
  var checkTextNode = function (node) {
    var parent = node.parentNode;
    if (!parent) return;
    var name = parent.nodeName.toLowerCase();
    for (var i = 0; i < nodeTextRules.length; i++) {
      var r = nodeTextRules[i];
      if (r.name !== name) continue;
      var text = node.textContent;
      r.condition.lastIndex = 0;
      if (!r.condition.test(text)) continue;
      if (r.remove) { node.textContent = ''; stats.replaced++; continue; }
      r.pattern.lastIndex = 0;
      var out = text.replace(r.pattern, function () { return r.replacement; });
      if (out !== text) { node.textContent = out; stats.replaced++; }
    }
  };
  var startNodeObserver = function () {
    var visit = function (node) {
      if (node.nodeType === 3) return checkTextNode(node);
      if (node.nodeType !== 1) return;
      for (var c = node.firstChild; c; c = c.nextSibling) if (c.nodeType === 3) checkTextNode(c);
    };
    new MutationObserver(function (records) {
      for (var i = 0; i < records.length; i++) {
        var added = records[i].addedNodes;
        for (var j = 0; j < added.length; j++) visit(added[j]);
      }
    }).observe(document, { childList: true, subtree: true });
  };

  // ---------------------------------------------------------------- scriptlets
  var resolvePath = function (path) {
    var parts = path.split('.'), owner = window;
    for (var i = 0; i < parts.length - 1; i++) { owner = owner && owner[parts[i]]; }
    return owner ? [owner, parts[parts.length - 1]] : null;
  };
  var scriptlets = {
    'json-prune': function (a) {
      var prune = pruner(a[0], a[1]);
      var parse = JSON.parse;
      JSON.parse = function () { var r = parse.apply(this, arguments); prune(r); return r; };
      var json = Response.prototype.json;
      Response.prototype.json = function () {
        return json.apply(this, arguments).then(function (r) { prune(r); return r; });
      };
    },
    'json-prune-fetch-response': function (a) {
      fetchHooks.push({ match: requestMatcher(extra(a, 2).propsToMatch), edit: jsonEditor(pruner(a[0], a[1])) });
    },
    'json-prune-xhr-response': function (a) {
      xhrHooks.push({ match: requestMatcher(extra(a, 2).propsToMatch), edit: jsonEditor(pruner(a[0], a[1])) });
    },
    'trusted-replace-fetch-response': function (a) {
      fetchHooks.push({ match: requestMatcher(a[2]), edit: textReplacer(a[0], a[1] || '') });
    },
    'trusted-replace-xhr-response': function (a) {
      xhrHooks.push({ match: requestMatcher(a[2]), edit: textReplacer(a[0], a[1] || '') });
    },
    'set-constant': function (a) {
      if (!a[0]) return;
      trap(window, a[0].split('.'), constValue(a[1]));
    },
    'adjust-setTimeout': function (a) {
      var needle = toRe(a[0]);
      var delay = a[1] === undefined || a[1] === '' ? 1000 : (a[1] === '*' ? null : Number(a[1]));
      var boost = Math.min(Math.max(Number(a[2]) || 0.05, 0.001), 50);
      var st = window.setTimeout;
      window.setTimeout = function (fn, d) {
        try {
          needle.lastIndex = 0;
          if ((delay === null || Number(d) === delay) && needle.test(String(fn))) arguments[1] = d * boost;
        } catch (e) {}
        return st.apply(this, arguments);
      };
    },
    'remove-node-text': function (a) {
      nodeTextRules.push({ name: String(a[0] || '').toLowerCase(), condition: toRe(a[1]), remove: true });
    },
    'replace-node-text': function (a) {
      var x = extra(a, 3);
      nodeTextRules.push({
        name: String(a[0] || '').toLowerCase(), pattern: toRe(a[1]), replacement: a[2] || '',
        condition: toRe(x.condition !== undefined ? x.condition : a[1]), remove: false
      });
    },
    'trusted-prevent-dom-bypass': function (a) {
      var t = resolvePath(a[0] || ''), target = a[1];
      if (!t || typeof t[0][t[1]] !== 'function') return;
      var owner = t[0], name = t[1], orig = owner[name];
      owner[name] = function () {
        var r = orig.apply(this, arguments);
        try {
          for (var i = 0; i < arguments.length; i++) {
            var w = arguments[i] && arguments[i].contentWindow;
            if (w && target) w[target] = window[target];
          }
        } catch (e) {}
        return r;
      };
    },
    'no-window-open-if': function (a) {
      var needle = toRe(a[0]), open = window.open;
      window.open = function (url) {
        needle.lastIndex = 0;
        if (needle.test(String(url))) return null;
        return open.apply(this, arguments);
      };
    },
    'addEventListener-defuser': function (a) {
      var type = toRe(a[0]), needle = toRe(a[1]), add = EventTarget.prototype.addEventListener;
      EventTarget.prototype.addEventListener = function (t, fn) {
        try {
          type.lastIndex = 0; needle.lastIndex = 0;
          if (type.test(String(t)) && needle.test(String(fn))) return;
        } catch (e) {}
        return add.apply(this, arguments);
      };
    }
  };
  var aliases = {
    'set': 'set-constant',
    'trusted-rpfr': 'trusted-replace-fetch-response',
    'nano-stb': 'adjust-setTimeout',
    'nano-setTimeout-booster': 'adjust-setTimeout',
    'rmnt': 'remove-node-text',
    'rpnt': 'replace-node-text',
    'trusted-rpnt': 'replace-node-text',
    'trusted-replace-node-text': 'replace-node-text',
    'nowoif': 'no-window-open-if',
    'window.open-defuser': 'no-window-open-if',
    'aeld': 'addEventListener-defuser',
    'prevent-addEventListener': 'addEventListener-defuser'
  };

  (RULES || []).forEach(function (rule) {
    if (rule.hosts && rule.hosts.indexOf(host) === -1) return;
    var name = aliases[rule.name] || rule.name;
    var fn = scriptlets[name];
    if (!fn) {
      if (stats.unsupported.indexOf(rule.name) === -1) stats.unsupported.push(rule.name);
      return;
    }
    try { fn(rule.args || []); stats.applied++; }
    catch (e) { stats.failed.push(rule.name + ': ' + e); }
  });
  if (nodeTextRules.length) startNodeObserver();
  if (!top) return;

  // YouTube's player compares the window's size with the screen's (and with the largest
  // window it has seen). A window well under the screen size counts as picture-in-picture,
  // which it pauses on every tick for accounts without Premium. Here the page only ever
  // fills a part of the screen (the player box, the mini bar, Android's floating window),
  // so the page is told its window is the whole screen.
  try {
    Object.defineProperty(window, 'outerWidth', { configurable: true, get: function () { return screen.width; } });
    Object.defineProperty(window, 'outerHeight', { configurable: true, get: function () { return screen.height; } });
  } catch (e) {}

  // Background play: YouTube pauses when the page says it is hidden. The app keeps the
  // page running with the screen off, so the page is told it is always visible.
  if (OPTS.background) {
    try {
      Object.defineProperty(document, 'hidden', { configurable: true, get: function () { return false; } });
      Object.defineProperty(document, 'visibilityState', { configurable: true, get: function () { return 'visible'; } });
      document.hasFocus = function () { return true; };
      var swallow = function (e) { e.stopImmediatePropagation(); };
      document.addEventListener('visibilitychange', swallow, true);
      document.addEventListener('webkitvisibilitychange', swallow, true);
      window.addEventListener('pagehide', swallow, true);
    } catch (e) {}
  }

  // A service worker could serve pages that never pass through the app, without this script.
  try {
    if (navigator.serviceWorker) {
      navigator.serviceWorker.register = function () { return Promise.reject(new Error('disabled')); };
      navigator.serviceWorker.getRegistrations().then(function (rs) {
        rs.forEach(function (r) { r.unregister(); });
      }).catch(function () {});
    }
  } catch (e) {}

  // ----------------------------------------------------------- Shorts page
  // The Shorts feed keeps YouTube's own layout; only the site's bars and app prompts go,
  // and the page stays on Shorts.
  if (OPTS.shorts) {
    var shortsCss = [
      'html, body { background: #000 !important; }',
      'ytm-mobile-topbar-renderer, ytm-pivot-bar-renderer, ytm-app-promo, ytm-mealbar-promo-renderer, .mealbar-promo-renderer,',
      'ytm-open-app-promo, .open-app-button, ytm-consent-bump-v2-lightbox, ytm-companion-slot { display: none !important; }',
      // Without the bars the feed can use the whole view.
      'ytm-app, ytm-app.sticky-player, #app { padding-top: 0 !important; padding-bottom: 0 !important; }',
      'ytm-shorts, #player-shorts-container, .shorts-player-container, ytm-reel-player-overlay-renderer { top: 0 !important; }'
    ].join('\n');
    var addShortsCss = function () {
      if (document.getElementById('sp-css')) return;
      var s = document.createElement('style');
      s.id = 'sp-css'; s.textContent = shortsCss;
      (document.head || document.documentElement).appendChild(s);
    };
    addShortsCss();
    document.addEventListener('DOMContentLoaded', addShortsCss);
    var shortsSeen = false;
    setInterval(function () {
      if (/^\/shorts(\/|$)/.test(location.pathname)) { shortsSeen = true; return; }
      // A link led away from Shorts (a channel, a sound page): return to the feed.
      if (shortsSeen && history.length > 1) history.back();
    }, 700);
    // Shorts start muted on the mobile site; the app allows sound.
    var unmutedShort = null;
    setInterval(function () {
      var v = document.querySelector('video');
      if (!v || v.paused || unmutedShort === location.pathname) return;
      unmutedShort = location.pathname;
      if (v.muted) {
        v.muted = false;
        var b = document.querySelector('.ytp-unmute, .ytmUnmuteButtonHost, ytm-unmute-button button');
        if (b) b.click();
      }
    }, 400);
    window.__sp = {
      cmd: function (what) {
        var vs = document.querySelectorAll('video');
        for (var i = 0; i < vs.length; i++) {
          if (what === 'pause') vs[i].pause();
          else if (what === 'play' && i === 0) { var p = vs[i].play(); if (p && p.catch) p.catch(function () {}); }
        }
      }
    };
    return;
  }

  // ----------------------------------------------------------- player-only page
  var css = [
    'html, body { background: #000 !important; overflow: hidden !important; }',
    // Mobile site: everything but the player.
    'ytm-mobile-topbar-renderer, ytm-pivot-bar-renderer, ytm-single-column-watch-next-results-renderer,',
    'ytm-engagement-panel, ytm-app-promo, ytm-mealbar-promo-renderer, .mealbar-promo-renderer, ytm-consent-bump-v2-lightbox,',
    'ytm-companion-slot, ytm-promoted-sparkles-web-renderer, .ytp-ad-overlay-container, ytm-watch-metadata-app-promo-renderer,',
    '.open-app-button, ytm-open-app-promo, .ytm-autonav-bar, ytm-playlist-panel-renderer,',
    // Desktop site: everything but the player.
    '#masthead-container, #secondary, #below, #chat, ytd-popup-container, tp-yt-iron-overlay-backdrop,',
    'ytd-enforcement-message-view-model, .ytp-ad-message-container, .ytp-suggested-action, .ytp-paid-content-overlay,',
    '#player-ads, #panels { display: none !important; }',
    // Mobile page: the player container fills the view. The video and the controls are
    // siblings inside it, so only the container is lifted, never the video itself.
    '#player-container-id { position: fixed !important; top: 0 !important; left: 0 !important; right: 0 !important;',
    '  bottom: 0 !important; width: 100vw !important; height: 100vh !important; max-height: none !important;',
    '  z-index: 2147483000 !important; margin: 0 !important; transform: none !important; }',
    '#player-container-id #player, #player-container-id #movie_player { position: absolute !important; top: 0 !important;',
    '  left: 0 !important; width: 100% !important; height: 100% !important; padding: 0 !important; }',
    // The video layer carries its own z-index; keep the whole player below the controls.
    '#player-container-id #player { z-index: 0 !important; }',
    '#player-container-id #player-control-container { position: absolute !important; top: 0 !important; left: 0 !important;',
    '  width: 100% !important; height: 100% !important; z-index: 20 !important; }',
    '#player-container-id #player-control-overlay { width: 100% !important; height: 100% !important; }',
    // Desktop page: there the controls live inside the player element.
    'ytd-app #movie_player { position: fixed !important; top: 0 !important; left: 0 !important; width: 100vw !important;',
    '  height: 100vh !important; z-index: 2147483000 !important; }',
    '#movie_player .html5-video-container, #movie_player video { width: 100% !important; height: 100% !important;',
    '  left: 0 !important; top: 0 !important; object-fit: contain !important; }',
    '.player-placeholder { display: none !important; }',
    // Mini player and picture-in-picture: the picture only, no controls or overlays.
    // They stay laid out (the page pauses a player it believes is hidden), only unseen.
    'html.sp-compact #player-control-container, html.sp-compact .ytp-paid-content-overlay,',
    'html.sp-compact .ytp-caption-window-container, html.sp-compact #player-endscreen-container {',
    '  opacity: 0 !important; pointer-events: none !important; }',
    // Zoom to fill: the picture covers the whole view and its edges are cropped.
    'html.sp-fill #movie_player video { object-fit: cover !important; }',
    // Audio only: the picture is not drawn; the video's thumbnail shows in its place.
    'html.sp-audio #movie_player video { opacity: 0 !important; }',
    'html.sp-audio #player-container-id { background: #000 center / contain no-repeat !important; }',
    // The settings sheet (quality, speed, captions) must come above the control layer.
    'bottom-sheet-container, #player-container-id bottom-sheet-container { z-index: 2147483600 !important; }'
  ].join('\n');
  var addCss = function () {
    if (document.getElementById('sp-css')) return;
    var s = document.createElement('style');
    s.id = 'sp-css'; s.textContent = css;
    (document.head || document.documentElement).appendChild(s);
  };
  addCss();
  document.addEventListener('DOMContentLoaded', addCss);

  var player = function () { return document.querySelector('#movie_player, .html5-video-player'); };
  var video = function () { return document.querySelector('#movie_player video, .html5-video-player video, video.html5-main-video'); };
  var adState = function () {
    var p = player();
    if (!p) return false;
    return p.classList.contains('ad-showing') || p.classList.contains('ad-interrupting') ||
      !!p.querySelector('.ytp-ad-player-overlay, .ytp-ad-player-overlay-layout, .ytp-ad-skip-button-container, .ytp-skip-ad-button, .ytp-ad-preview-container');
  };

  // If an ad starts anyway: jump to its end and press Skip.
  var inAd = false, mutedByUs = false, adTicks = 0;
  setInterval(function () {
    var v = video();
    if (!adState()) {
      if (inAd && v && mutedByUs) { v.muted = false; mutedByUs = false; }
      inAd = false;
      return;
    }
    adTicks++;
    if (!inAd) { inAd = true; stats.skipped++; }
    if (v) {
      if (!v.muted) { v.muted = true; mutedByUs = true; }
      if (isFinite(v.duration) && v.duration > 0 && v.currentTime < v.duration - 0.3) v.currentTime = v.duration;
    }
    var b = document.querySelector('.ytp-skip-ad-button, .ytp-ad-skip-button, .ytp-ad-skip-button-modern, .ytp-ad-skip-button-container button');
    if (b) b.click();
  }, 120);

  // The mobile site starts videos muted ("tap to unmute") and sometimes paused. The app
  // allows sound without a tap, so for the first seconds of each video undo both.
  // Diagnostics: who pauses the video, and what the page sees around it.
  var debug = function (text) { if (OPTS.debug) post({ type: 'log', text: text }); };
  if (OPTS.debug) {
    var realPause = HTMLMediaElement.prototype.pause;
    HTMLMediaElement.prototype.pause = function () {
      debug('pause() called by: ' + (new Error().stack || '').split('\n').slice(2, 7).map(function (s) {
        return s.trim().replace(/https:\/\/[^ )]*\//, '').slice(0, 80);
      }).join(' < '));
      return realPause.apply(this, arguments);
    };
    window.addEventListener('resize', function () {
      debug('resize ' + innerWidth + 'x' + innerHeight + ' visibility=' + document.visibilityState);
    });
  }

  // When the player changes shape (fullscreen, mini bar, picture-in-picture, rotation) the
  // page sometimes pauses by itself. A pause that lands right around such a change, and
  // that the app did not ask for, is undone.
  var guardUntil = 0, lastSelfPause = 0, appPaused = false, resumes = [];
  var resume = function () {
    var v = video(), now = Date.now();
    resumes = resumes.filter(function (t) { return now - t < 6000; });
    if (!v || v.ended || appPaused || resumes.length >= 4) return;
    resumes.push(now);
    var p = v.play();
    if (p && p.catch) p.catch(function () {});
  };
  var shapeChanged = function () {
    guardUntil = Date.now() + 1800;
    var v = video();
    if (v && v.paused && Date.now() - lastSelfPause < 900) resume();
  };
  window.addEventListener('resize', shapeChanged);
  document.addEventListener('fullscreenchange', shapeChanged, true);
  document.addEventListener('webkitfullscreenchange', shapeChanged, true);
  var onPause = function () {
    if (appPaused || adState()) return;
    lastSelfPause = Date.now();
    if (Date.now() < guardUntil) setTimeout(resume, 60);
  };

  var startId = null, startUntil = 0, watched = null, lastPlaying = null;
  var reportPlaying = function () {
    var v = video();
    var now = !!v && !v.paused && !v.ended;
    if (now !== lastPlaying) { lastPlaying = now; post({ type: 'playing', playing: now }); }
  };
  setInterval(function () {
    var id = videoId(), v = video();
    // Tell the app when playback starts and stops (for its notification and mini player).
    if (v && v !== watched) {
      watched = v;
      ['play', 'playing', 'pause', 'ended', 'emptied'].forEach(function (name) {
        v.addEventListener(name, reportPlaying);
      });
      v.addEventListener('pause', onPause);
      ['pause', 'play', 'playing', 'waiting', 'stalled', 'suspend', 'error', 'emptied', 'abort'].forEach(function (name) {
        v.addEventListener(name, function () {
          debug('video ' + name + ' t=' + v.currentTime.toFixed(1) + ' ready=' + v.readyState + ' net=' + v.networkState +
            (v.error ? ' error=' + v.error.code : '') + ' muted=' + v.muted + ' ad=' + adState());
        });
      });
      v.addEventListener('play', function () { appPaused = false; });
    }
    reportPlaying();
    if (id !== startId) { startId = id; startUntil = Date.now() + 10000; }
    if (!v || !id || Date.now() > startUntil || adState() || mutedByUs) return;
    if (v.muted) {
      var p = player();
      try { if (p && typeof p.unMute === 'function') p.unMute(); } catch (e) {}
      v.muted = false;
      var b = document.querySelector('.ytp-unmute, .ytmUnmuteButtonHost, ytm-unmute-button button, button.ytp-unmute');
      if (b) b.click();
    }
    if (v.paused && v.currentTime < 0.5) {
      var pr = v.play();
      if (pr && pr.catch) pr.catch(function () {});
    }
  }, 300);

  function videoId() {
    var m = /[?&]v=([\w-]{11})/.exec(location.search);
    return m ? m[1] : null;
  }
  var lastId = null, endedFor = null;
  setInterval(function () {
    var v = video(), id = videoId();
    if (location.pathname !== '/watch' && OPTS.watchOnly && history.length > 1 && lastId) {
      // The player page wandered off (a link inside the player): go back to the video.
      history.back();
      return;
    }
    if (id && id !== lastId) { lastId = id; endedFor = null; post({ type: 'video', id: id }); }
    var ad = adState();
    if (v && id && v.ended && !ad && endedFor !== id) { endedFor = id; post({ type: 'ended', id: id }); }
    post({
      type: 'state', id: id, ad: ad, adTicks: adTicks,
      pos: v ? Math.round(v.currentTime * 10) / 10 : null,
      dur: v && isFinite(v.duration) ? Math.round(v.duration) : null,
      paused: v ? v.paused : null, height: v ? v.videoHeight : null,
      signedIn: (function () { try { return !!(window.ytcfg && window.ytcfg.get('LOGGED_IN')); } catch (e) { return null; } })(),
      wall: !!document.querySelector('ytd-enforcement-message-view-model'),
      stats: stats
    });
  }, 2000);
  post({ type: 'loaded', host: host, rules: (RULES || []).length, stats: stats });

  // Diagnostics: the structure around the player, logged once, to keep the layout rules
  // above matched to YouTube's page.
  if (OPTS.debug) {
    setTimeout(function () {
      var describe = function (el, depth) {
        if (!el || depth > 5) return '';
        var cs = getComputedStyle(el), r = el.getBoundingClientRect();
        var line = new Array(depth + 1).join('  ') + el.tagName.toLowerCase() + (el.id ? '#' + el.id : '') +
          (el.className && typeof el.className === 'string' ? '.' + el.className.trim().split(/\s+/).slice(0, 4).join('.') : '') +
          ' [' + cs.position + ' z' + cs.zIndex + ' ' + cs.display + ' ' + Math.round(r.left) + ',' + Math.round(r.top) + ' ' +
          Math.round(r.width) + 'x' + Math.round(r.height) + ']\n';
        for (var c = el.firstElementChild, n = 0; c && n < 14; c = c.nextElementSibling, n++) line += describe(c, depth + 1);
        return line;
      };
      var rootEl = document.querySelector('#player-container-id') || (player() && player().parentElement);
      post({ type: 'dom', tree: describe(rootEl, 0).slice(0, 6000), body: document.body ? document.body.className : '' });
    }, 6000);
  }

  // ------------------------------------------------------------- settings menu
  // The gear button opens the app's own menu instead of YouTube's sheet: the click is taken
  // here, and the app is sent what the player offers (qualities, speeds, caption tracks).
  var prefQuality = 'auto';
  var api = function (name) {
    var mp = player();
    try {
      if (mp && typeof mp[name] === 'function') return mp[name].apply(mp, Array.prototype.slice.call(arguments, 1));
    } catch (e) {}
    return undefined;
  };
  var captionTracks = function () {
    var out = [];
    try {
      var pr = api('getPlayerResponse');
      var list = pr.captions.playerCaptionsTracklistRenderer.captionTracks || [];
      list.forEach(function (t) {
        var name = t.name && (t.name.simpleText || (t.name.runs || []).map(function (r) { return r.text; }).join(''));
        out.push({ id: t.languageCode + (t.kind ? '.' + t.kind : ''), label: name || t.languageCode, code: t.languageCode, kind: t.kind || '' });
      });
    } catch (e) {}
    return out;
  };
  var settingsInfo = function () {
    if (!player()) return null;
    var qualities = [];
    (api('getAvailableQualityData') || []).forEach(function (q) {
      if (q && q.quality && q.isPlayable !== false) qualities.push({ id: q.quality, label: q.qualityLabel || q.quality });
    });
    if (!qualities.length) {
      (api('getAvailableQualityLevels') || []).forEach(function (id) {
        if (id !== 'auto') qualities.push({ id: id, label: id });
      });
    }
    var caption = '';
    try {
      var tr = api('getOption', 'captions', 'track');
      if (tr && tr.languageCode) caption = tr.languageCode + (tr.kind ? '.' + tr.kind : '');
    } catch (e) {}
    return {
      type: 'settings', qualities: qualities, quality: api('getPlaybackQuality') || '', preferred: prefQuality,
      rates: api('getAvailablePlaybackRates') || [0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2],
      rate: api('getPlaybackRate') || 1, captions: captionTracks(), caption: caption
    };
  };
  var onGear = function (t) {
    return !!(t && t.closest && t.closest('.player-settings-icon, button[aria-label="Playback Settings"]'));
  };
  document.addEventListener('click', function (e) {
    if (!onGear(e.target)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    var info = settingsInfo();
    if (info) post(info);
  }, true);
  // YouTube's own handlers for that button must not run on the touch either.
  ['touchstart', 'touchend', 'pointerdown', 'pointerup', 'mousedown', 'mouseup'].forEach(function (name) {
    document.addEventListener(name, function (e) {
      if (onGear(e.target)) e.stopImmediatePropagation();
    }, true);
  });
  var setQuality = function (id) {
    prefQuality = id || 'auto';
    try { api('setPlaybackQualityRange', prefQuality, prefQuality); } catch (e) {}
    try { api('setPlaybackQuality', prefQuality); } catch (e) {}
  };
  // Audio only: sound keeps playing, the picture is fetched at the lowest quality (the
  // player cannot fetch sound alone) and hidden behind the thumbnail.
  var audioOnly = false;
  var setAudioOnly = function (on) {
    audioOnly = !!on;
    document.documentElement.classList.toggle('sp-audio', audioOnly);
    var box = document.querySelector('#player-container-id');
    var id = videoId();
    if (box) box.style.backgroundImage = audioOnly && id ? 'url(https://i.ytimg.com/vi/' + id + '/hqdefault.jpg?sp=1)' : '';
    var q = audioOnly ? 'tiny' : prefQuality;
    try { api('setPlaybackQualityRange', q, q); } catch (e) {}
    try { api('setPlaybackQuality', q); } catch (e) {}
  };
  var setCaptions = function (id) {
    if (!id) {
      try { api('setOption', 'captions', 'track', {}); } catch (e) {}
      try { api('unloadModule', 'captions'); } catch (e) {}
      return;
    }
    var track = captionTracks().filter(function (t) { return t.id === id; })[0];
    if (!track) return;
    api('loadModule', 'captions');
    var apply = function () { api('setOption', 'captions', 'track', { languageCode: track.code, kind: track.kind }); };
    apply();
    // The captions module loads asynchronously the first time.
    setTimeout(apply, 600);
  };

  // The app can drive the player: window.__sp.cmd('pause' | 'play' | 'seek', seconds).
  window.__sp = {
    cmd: function (what, arg) {
      var v = video();
      if (what === 'compact') { shapeChanged(); document.documentElement.classList.toggle('sp-compact', !!arg); return; }
      if (what === 'shape') { shapeChanged(); return; }
      if (what === 'fill') { document.documentElement.classList.toggle('sp-fill', !!arg); return; }
      // Switch to another video inside this page: the site's own router handles a click
      // on a watch link without reloading, which is much faster than a new page.
      if (what === 'open') {
        var link = document.createElement('a');
        link.href = '/watch?v=' + encodeURIComponent(String(arg));
        link.style.display = 'none';
        (document.body || document.documentElement).appendChild(link);
        link.click();
        link.remove();
        startUntil = Date.now() + 10000;
        appPaused = false;
        return;
      }
      // The video was closed: stop playing and loading, but keep the page for the next one.
      if (what === 'stop') {
        appPaused = true;
        startUntil = 0;
        try { api('pauseVideo'); } catch (e) {}
        try { api('stopVideo'); } catch (e) {}
        if (v) v.pause();
        return;
      }
      if (what === 'quality') { setQuality(String(arg || 'auto')); if (audioOnly) setAudioOnly(true); return; }
      if (what === 'audio') { setAudioOnly(!!arg); return; }
      if (what === 'rate') { api('setPlaybackRate', Number(arg) || 1); return; }
      if (what === 'captions') { setCaptions(String(arg || '')); return; }
      if (!v) return;
      if (what === 'compact') { document.documentElement.classList.toggle('sp-compact', !!arg); return; }
      if (what === 'toggle') what = v.paused ? 'play' : 'pause';
      if (what === 'pause') { startUntil = 0; appPaused = true; v.pause(); }
      else if (what === 'play') { var p = v.play(); if (p && p.catch) p.catch(function () {}); }
      else if (what === 'seek') v.currentTime = Number(arg) || 0;
    }
  };
})(__RULES__, __OPTS__);
