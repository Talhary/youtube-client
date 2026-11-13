// Evaluate an expression in one of the app's web views over the DevTools protocol.
//   node cdp.js <url substring | "player"> <expression | @file>
const fs = require('fs');
const [, , match, exprArg] = process.argv;
const expr = exprArg.startsWith('@') ? fs.readFileSync(exprArg.slice(1), 'utf8') : exprArg;
(async () => {
  const targets = await (await fetch('http://127.0.0.1:9222/json')).json();
  const t = targets.find((x) =>
    match === 'player' ? !x.url.startsWith('https://app.streampipe') : x.url.includes(match));
  if (!t) { console.error('no target; have:', targets.map((x) => x.url)); process.exit(2); }
  const ws = new WebSocket(t.webSocketDebuggerUrl);
  ws.onopen = () => ws.send(JSON.stringify({
    id: 1, method: 'Runtime.evaluate',
    params: { expression: expr, returnByValue: true, awaitPromise: true, userGesture: true },
  }));
  ws.onmessage = (m) => {
    const r = JSON.parse(m.data);
    if (r.id !== 1) return;
    if (r.error || !r.result || !r.result.result) {
      console.log('PROTOCOL ERROR', JSON.stringify(r.error || r).slice(0, 400));
      ws.close();
      return;
    }
    const res = r.result.result;
    if (r.result && r.result.exceptionDetails) console.log('EXCEPTION', JSON.stringify(r.result.exceptionDetails).slice(0, 800));
    else console.log(typeof res.value === 'string' ? res.value : JSON.stringify(res.value, null, 1));
    ws.close();
  };
  ws.onerror = (e) => { console.error('ws error', e.message); process.exit(1); };
})();
