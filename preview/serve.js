#!/usr/bin/env node
/*
 * 本地预览服务器：把仓库里已构建的 WebUI 挂到 /5700/，并给 /cgi-bin/at-ws-info
 * 一个最小响应。仅用于桌面预览与截图，不参与设备运行。
 *
 *   node preview/serve.js          # http://127.0.0.1:8788/5700/
 *   node preview/serve.js 9000     # 自定义端口
 */
const http = require('http');
const fs = require('fs');
const path = require('path');

const ROOT = path.resolve(__dirname, '..');
const WEB = path.join(ROOT, 'mt5700webui-openwrt-server/at-webserver/files/www/5700');
const PREVIEW = path.join(ROOT, 'preview');
const PORT = Number(process.argv[2] || 8788);

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.ico': 'image/x-icon',
};

function send(res, code, type, body) {
  res.writeHead(code, { 'content-type': type, 'cache-control': 'no-store' });
  res.end(body);
}

http
  .createServer((req, res) => {
    const url = new URL(req.url, 'http://localhost');
    let pathname = decodeURIComponent(url.pathname);

    if (pathname === '/cgi-bin/at-ws-info') {
      return send(
        res,
        200,
        'application/json; charset=utf-8',
        JSON.stringify({
          success: true,
          data: {
            host: '127.0.0.1',
            port: 8765,
            allow_wan: 0,
            require_auth: false,
            ws_url: 'ws://127.0.0.1:8765',
            timestamp: Math.floor(Date.now() / 1000),
          },
        }),
      );
    }

    let base = WEB;
    if (pathname === '/' || pathname === '/5700' || pathname === '/5700/') {
      pathname = '/5700/index.html';
    }
    if (pathname.startsWith('/5700/')) {
      pathname = pathname.slice('/5700'.length);
    } else if (pathname.startsWith('/luci-static/')) {
      base = path.join(ROOT, 'luci-app-mt5700m/htdocs/luci-static');
      pathname = pathname.slice('/luci-static'.length);
    } else if (pathname.startsWith('/preview/')) {
      base = PREVIEW;
      pathname = pathname.slice('/preview'.length);
    } else {
      return send(res, 404, 'text/plain; charset=utf-8', 'not found');
    }

    const file = path.normalize(path.join(base, pathname));
    if (!file.startsWith(base)) {
      return send(res, 403, 'text/plain; charset=utf-8', 'forbidden');
    }
    fs.readFile(file, (err, buf) => {
      if (err) return send(res, 404, 'text/plain; charset=utf-8', 'not found');
      send(res, 200, MIME[path.extname(file).toLowerCase()] || 'application/octet-stream', buf);
    });
  })
  .listen(PORT, '127.0.0.1', () => {
    console.log(`WebUI preview: http://127.0.0.1:${PORT}/5700/`);
    console.log(`LuCI preview : http://127.0.0.1:${PORT}/preview/luci-status.html`);
  });
