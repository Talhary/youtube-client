// StreamPipe challenge-solver server (runs on Node.js).
//
// Loads yt-dlp's EJS solver once and answers requests over a local socket, so the analysed
// player script stays in memory between videos.
//
//   node solver.js <port file>
//
// One request per connection, one JSON line each way:
//   -> {"player": "<path of the player script>", "pre": "<path of its analysed form>",
//       "n": ["…"], "sig": ["…"]}
//   <- {"ok": {"<challenge>": "<answer>"}}  or  {"err": "why"}
'use strict';
const net = require('net');
const fs = require('fs');
const path = require('path');

const here = __dirname;
const jsc = new Function(
  fs.readFileSync(path.join(here, 'lib.min.js'), 'utf8') +
  '\nvar meriyah=lib.meriyah,astring=lib.astring;\n' +
  fs.readFileSync(path.join(here, 'core.min.js'), 'utf8') +
  '\nreturn jsc;'
)();

// The analysed player now in memory, and which file it belongs to.
let loaded = { pre: null, text: null };

function solve(job) {
  const requests = [];
  if (job.n && job.n.length) requests.push({ type: 'n', challenges: job.n });
  if (job.sig && job.sig.length) requests.push({ type: 'sig', challenges: job.sig });
  if (loaded.pre !== job.pre) {
    loaded = { pre: null, text: null };
    try { loaded = { pre: job.pre, text: fs.readFileSync(job.pre, 'utf8') }; } catch (e) {}
  }
  let result;
  if (loaded.text) {
    result = jsc({ type: 'preprocessed', preprocessed_player: loaded.text, requests: requests });
  } else {
    result = jsc({ type: 'player', player: fs.readFileSync(job.player, 'utf8'), requests: requests, output_preprocessed: true });
    if (result.preprocessed_player) {
      loaded = { pre: job.pre, text: result.preprocessed_player };
      try {
        fs.writeFileSync(job.pre + '.tmp', result.preprocessed_player);
        fs.renameSync(job.pre + '.tmp', job.pre);
      } catch (e) {}
    }
  }
  const answers = {};
  for (const r of result.responses || []) {
    if (r.type !== 'result') throw new Error(String(r.error || 'unknown error').slice(0, 300));
    Object.assign(answers, r.data);
  }
  return answers;
}

const server = net.createServer(function (socket) {
  let text = '';
  socket.setEncoding('utf8');
  socket.on('error', function () {});
  socket.on('data', function (chunk) {
    text += chunk;
    const end = text.indexOf('\n');
    if (end < 0) return;
    let reply;
    try { reply = { ok: solve(JSON.parse(text.slice(0, end))) }; }
    catch (e) { reply = { err: String(e && e.message || e) }; }
    socket.end(JSON.stringify(reply) + '\n');
  });
});
server.listen(0, '127.0.0.1', function () {
  const file = process.argv[2];
  fs.writeFileSync(file + '.tmp', String(server.address().port));
  fs.renameSync(file + '.tmp', file);
});
