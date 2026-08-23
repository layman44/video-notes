// Zero-dependency Douyin cookie extractor using Chrome/Edge DevTools Protocol (CDP)
import { spawn, execSync } from 'child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync, accessSync } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';

function findBrowserPath() {
  const customBrowser = process.env.DOUYIN_BROWSER_PATH;
  if (customBrowser) {
    try { accessSync(customBrowser); return customBrowser; } catch {}
  }

  const standardPaths = [
    'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
    'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
    'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
    'C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe',
    join(process.env.LOCALAPPDATA || '', 'Microsoft\\Edge\\Application\\msedge.exe'),
    join(process.env.LOCALAPPDATA || '', 'Google\\Chrome\\Application\\chrome.exe'),
  ];

  for (const p of standardPaths) {
    try {
      if (p) { accessSync(p); return p; }
    } catch {}
  }

  // Try finding via `where` on Windows
  for (const exe of ['msedge.exe', 'chrome.exe']) {
    try {
      const stdout = execSync(`where.exe ${exe}`, { stdio: ['ignore', 'pipe', 'ignore'], encoding: 'utf8' }).trim();
      const firstLine = stdout.split('\n')[0]?.trim();
      if (firstLine) {
        accessSync(firstLine);
        return firstLine;
      }
    } catch {}
  }

  return null;
}

function parseArgs() {
  const args = process.argv.slice(2);
  let url = 'https://www.douyin.com/';
  let outputPath = join(tmpdir(), 'douyin_cookies.txt');

  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--url' && args[i + 1]) {
      url = args[++i];
    } else if (args[i] === '--output' && args[i + 1]) {
      outputPath = args[++i];
    } else if (i === 0 && !args[i].startsWith('--')) {
      url = args[i];
    } else if (i === 1 && !args[i].startsWith('--')) {
      outputPath = args[i];
    }
  }

  return { url, outputPath };
}

async function getDevToolsPort(userDir, maxWaitMs = 12000) {
  const start = Date.now();
  const portFile = join(userDir, 'DevToolsActivePort');
  while (Date.now() - start < maxWaitMs) {
    try {
      const content = readFileSync(portFile, 'utf8').trim().split('\n');
      if (content.length >= 2) {
        const port = content[0].trim();
        return port;
      }
    } catch {}
    await new Promise(r => setTimeout(r, 100));
  }
  throw new Error('启动无头浏览器超时：未找到 DevToolsActivePort');
}

async function main() {
  const { url, outputPath } = parseArgs();
  const browserPath = findBrowserPath();

  if (!browserPath) {
    console.error(JSON.stringify({
      ok: false,
      error: '未找到 Microsoft Edge 或 Google Chrome 浏览器，无法获取抖音反爬令牌'
    }));
    process.exit(1);
  }

  const userDir = mkdtempSync(join(tmpdir(), 'douyin-cdp-'));
  const browserProcess = spawn(browserPath, [
    '--headless=new',
    '--disable-gpu',
    '--no-sandbox',
    '--remote-debugging-port=0',
    `--user-data-dir=${userDir}`,
    '--no-first-run',
    '--no-default-browser-check',
    'about:blank',
  ], { stdio: ['ignore', 'ignore', 'ignore'] });

  let pageWs = null;

  try {
    const port = await getDevToolsPort(userDir);
    const listResp = await fetch(`http://127.0.0.1:${port}/json/list`);
    const targets = await listResp.json();
    const pageTarget = targets.find(t => t.type === 'page') || targets[0];
    if (!pageTarget || !pageTarget.webSocketDebuggerUrl) {
      throw new Error('未找到浏览器页面调试通道');
    }

    pageWs = new WebSocket(pageTarget.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      pageWs.onopen = resolve;
      pageWs.onerror = reject;
    });

    let idCounter = 1;
    const pending = new Map();
    pageWs.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data);
        if (msg.id && pending.has(msg.id)) {
          const { resolve, reject } = pending.get(msg.id);
          pending.delete(msg.id);
          if (msg.error) reject(new Error(msg.error.message));
          else resolve(msg.result);
        }
      } catch {}
    };

    function send(method, params = {}) {
      return new Promise((resolve, reject) => {
        const id = idCounter++;
        pending.set(id, { resolve, reject });
        pageWs.send(JSON.stringify({ id, method, params }));
      });
    }

    await send('Page.enable');
    await send('Network.enable');
    await send('Network.setUserAgentOverride', {
      userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36'
    });

    await send('Page.navigate', { url });

    // 等待 5 秒供抖音 JS 反爬与令牌初始化脚本执行完成
    await new Promise(r => setTimeout(r, 5000));

    const { cookies } = await send('Network.getAllCookies');
    if (!cookies || cookies.length === 0) {
      throw new Error('未能从浏览器提取到有效 Cookie');
    }

    const lines = ['# Netscape HTTP Cookie File'];
    for (const c of cookies) {
      if (!c.name) continue;
      const domain = c.domain.startsWith('.') ? c.domain : `.${c.domain}`;
      const secure = c.secure ? 'TRUE' : 'FALSE';
      const expires = (c.expires && c.expires > 0) ? Math.floor(c.expires) : 0;
      lines.push(`${domain}\tTRUE\t${c.path || '/'}\t${secure}\t${expires}\t${c.name}\t${c.value}`);
    }

    writeFileSync(outputPath, lines.join('\n') + '\n', 'ascii');

    console.log(JSON.stringify({
      ok: true,
      cookieCount: cookies.length,
      outputPath
    }));
  } catch (err) {
    console.error(JSON.stringify({
      ok: false,
      error: err.message || String(err)
    }));
    process.exit(1);
  } finally {
    if (pageWs) {
      try { pageWs.close(); } catch {}
    }
    if (browserProcess && browserProcess.pid) {
      try {
        if (process.platform === 'win32') {
          execSync(`taskkill /F /T /PID ${browserProcess.pid}`, { stdio: 'ignore' });
        } else {
          browserProcess.kill('SIGKILL');
        }
      } catch {}
    }
    try { rmSync(userDir, { recursive: true, force: true }); } catch {}
  }
}

main();
