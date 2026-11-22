// Raw-mode sink like a TUI prompt (Claude Code): logs every stdin chunk as JSON, one per line.
// `node rawline.js out.txt [kittyFlags]` — with flags, enables the kitty keyboard protocol (CSI > flags u).
const fs = require('fs'); const out = process.argv[2]; const flags = process.argv[3];
process.stdin.setRawMode(true); process.stdin.setEncoding('utf8');
if (flags) process.stdout.write(`\x1b[>${flags}u`);
if (process.argv[4]) process.stdout.write('\x1b[?2004h\x1b[?1004h\x1b[>4;2m'); // + bracketed paste, focus events, modifyOtherKeys 2 (as Claude Code)
process.stdin.on('data', chunk => {
  fs.appendFileSync(out, JSON.stringify(chunk).replace(/\x7f/g, '<BS>') + '\n');
  if (chunk.includes('\x03') || chunk.includes('[99;5u')) { if (flags) process.stdout.write('\x1b[<u'); process.exit(); }
});
