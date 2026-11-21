// Minimal raw-mode line editor like Claude Code's input (Node, stdin raw): handles chars, ⌫ and Enter → file
const fs = require('fs'); const out = process.argv[2]; let line = '';
process.stdin.setRawMode(true); process.stdin.setEncoding('utf8');
process.stdin.on('data', chunk => {
  for (const ch of chunk) {
    if (ch === '\r') { fs.appendFileSync(out, line + '\n'); line = ''; process.stdout.write('\r\n'); }
    else if (ch === '\x7f') { line = [...line].slice(0, -1).join(''); process.stdout.write('\b \b'); }
    else if (ch === '\x03') process.exit();
    else { line += ch; process.stdout.write(ch); }
  }
});
