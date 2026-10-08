import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
// Bare specifier so Vite resolves it through node_modules and emits it into
// the bundle. A relative '../node_modules/...' path also happens to work on
// disk, but it bypasses resolution and breaks as soon as the dep is hoisted
// or the tree is rebuilt.
import '@douyinfe/semi-ui/dist/css/semi.min.css';
import './styles/global.css';

try {
  if (localStorage.getItem('theme-mode') === 'dark') {
    document.body.setAttribute('theme-mode', 'dark');
  }
} catch {
  // Storage may be unavailable in hardened/private browser contexts.
}

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
