import React from 'react';
import ReactDOM from 'react-dom/client';
import { isTauri } from '@tauri-apps/api/core';
import App from './App';
import StatisticsWindow from './components/StatisticsWindow';
import './App.css';

// Only macOS desktop windows have a native blur layer behind the webview.
if (isTauri() && /Macintosh|Mac OS X/.test(navigator.userAgent)) {
  document.documentElement.dataset.nativeGlass = 'true';
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {window.location.hash === '#statistics' ? <StatisticsWindow /> : <App />}
  </React.StrictMode>,
);
